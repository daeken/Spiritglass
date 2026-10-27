mod discovery;
mod module;
mod process;
mod registry;
mod socket;

use anyhow::{Context, Result};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use registry::{Command, Control, Installation};
use serde_json::{Value, json};
use spiritglass_protocol::*;
use std::{os::unix::fs::DirBuilderExt, path::PathBuf, time::Duration};
use tokio::{
    net::UnixStream,
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_util::codec::{Framed, LinesCodec};

#[derive(Parser)]
#[command(about = "Spiritglass local module supervisor", version)]
struct Options {
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long, conflicts_with = "modules_dir")]
    module: Option<PathBuf>,
    #[arg(long)]
    modules_dir: Option<PathBuf>,
    #[arg(last = true, requires = "module")]
    module_args: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let options = Options::parse();
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is unset; supply explicit paths")
    };
    let socket = match options
        .socket
        .or_else(|| std::env::var_os("SPIRITGLASS_SOCKET").map(PathBuf::from))
    {
        Some(path) => path,
        None => home()?.join(".spiritglass/run/control.sock"),
    };
    let installation = if let Some(executable) = options.module {
        Installation::Single(discovery::Config::legacy(executable, options.module_args))
    } else {
        let directory = match options.modules_dir {
            Some(path) => path,
            None => home()?.join(".spiritglass/modules"),
        };
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        Installation::Directory(std::path::absolute(directory)?)
    };
    let (listener, _guard) = socket::bind(&socket)?;
    let (updates, _) = watch::channel(SessionSnapshot::default());
    let (commands, receiver) = mpsc::channel(64);
    let (shutdown, stopped) = watch::channel(false);
    let module_updates = updates.clone();
    let mut registry_task = tokio::spawn(registry::run(
        installation,
        module_updates,
        receiver,
        stopped,
    ));
    let mut clients = JoinSet::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    eprintln!("Spiritglass listening at {}", socket.display());
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = terminate.recv() => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                if clients.len() >= 64 { eprintln!("client limit reached"); continue; }
                let receiver = updates.subscribe();
                let commands = commands.clone();
                clients.spawn(async move {
                    if let Err(error) = client(stream, receiver, commands).await { eprintln!("client disconnected: {error:#}"); }
                });
            }
            _ = clients.join_next(), if !clients.is_empty() => {}
        }
    }
    let _ = shutdown.send(true);
    clients.abort_all();
    if tokio::time::timeout(Duration::from_secs(9), &mut registry_task)
        .await
        .is_err()
    {
        registry_task.abort();
        let _ = registry_task.await;
    }
    Ok(())
}

async fn client(
    stream: UnixStream,
    mut updates: watch::Receiver<SessionSnapshot>,
    commands: mpsc::Sender<Command>,
) -> Result<()> {
    let mut framed = Framed::new(stream, LinesCodec::new_with_max_length(MAX_MESSAGE_BYTES));
    let mut initialized = false;
    let mut subscribed = false;
    let mut last = SessionSnapshot::default();
    loop {
        tokio::select! {
            line = framed.next() => {
                let Some(line) = line else { return Ok(()); };
                let line = line?;
                let message = match Message::parse(&line) {
                    Ok(message) => message,
                    Err(error) => {
                        let code = if serde_json::from_str::<Value>(&line).is_err() { -32700 } else { -32600 };
                        write(&mut framed, Message::failure(None, RpcError::new(code, format!("invalid message: {error}")))).await?;
                        continue;
                    }
                };
                let Message::Request(request) = message else { continue; };
                let result: Result<Value, RpcError> = async {
                    if request.method == "initialize" {
                        if initialized { return Err(RpcError::new(-32002, "already initialized")); }
                        let params: InitializeParams = parse(request.params)?;
                        if !params.protocol_versions.contains(&SESSION_PROTOCOL_VERSION) { return Err(RpcError::new(-32001, "client protocol version 2 is required; module stdio still uses version 1")); }
                        initialized = true;
                        return Ok(json!(InitializeResult { protocol_version: SESSION_PROTOCOL_VERSION }));
                    }
                    if !initialized { return Err(RpcError::new(-32002, "initialize first")); }
                    match request.method.as_str() {
                        "session.subscribe" => {
                            let _: EmptyParams = parse(request.params)?;
                            subscribed = true;
                            last = updates.borrow_and_update().clone();
                            Ok(json!(last))
                        }
                        "action.invoke" => {
                            let params: SessionActionInvoke = parse(request.params)?;
                            let module_id = params.module_id.clone();
                            let (reply, receiver) = oneshot::channel();
                            commands.try_send(Command::Invoke { params, reply }).map_err(|_| RpcError::new(-32003, "supervisor is busy"))?;
                            let mut result = tokio::time::timeout(Duration::from_secs(6), receiver).await
                                .map_err(|_| RpcError::new(-32003, "action timed out; completion is unknown"))?
                                .map_err(|_| RpcError::new(-32003, "module disconnected; completion is unknown"))??;
                            result.active_generation = updates.borrow().modules.get(&module_id).filter(|module| module.phase == ModulePhase::Ready).map(|module| module.generation);
                            Ok(json!(result))
                        }
                        "module.reload" | "module.activate" | "module.deactivate" => {
                            let target: ModuleTarget = parse(request.params)?;
                            let control = match request.method.as_str() { "module.reload" => Control::Reload, "module.activate" => Control::Activate, _ => Control::Deactivate };
                            let (reply, receiver) = oneshot::channel();
                            commands.try_send(Command::Control { target, control, reply }).map_err(|_| RpcError::new(-32003, "supervisor is busy"))?;
                            tokio::time::timeout(Duration::from_secs(2), receiver).await
                                .map_err(|_| RpcError::new(-32003, "supervisor did not accept the command"))?
                                .map_err(|_| RpcError::new(-32003, "supervisor stopped"))?
                        }
                        _ => Err(RpcError::new(-32601, "method not found")),
                    }
                }.await;
                let response = match result { Ok(result) => Message::success(request.id, result), Err(error) => Message::failure(Some(request.id), error) };
                write(&mut framed, response).await?;
            }
            changed = updates.changed(), if subscribed => {
                if changed.is_err() { return Ok(()); }
                let current = updates.borrow_and_update().clone();
                let message = if !current.same_structure(&last) {
                    Message::notification("session.snapshot", &current)
                } else { Message::notification("state.changed", SessionStateChanged::from(&current)) };
                write(&mut framed, message).await?;
                last = current;
            }
        }
    }
}
fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|error| RpcError::invalid_params(error.to_string()))
}
async fn write(framed: &mut Framed<UnixStream, LinesCodec>, message: Message) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), framed.send(message.encode()?))
        .await
        .context("client write timed out")??;
    Ok(())
}
