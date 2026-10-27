mod module;
mod socket;

use anyhow::{Context, Result};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use module::Invocation;
use serde_json::{Value, json};
use spiritglass_protocol::*;
use std::{path::PathBuf, time::Duration};
use tokio::{
    net::UnixStream,
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_util::codec::{Framed, LinesCodec};

#[derive(Parser)]
#[command(about = "Spiritglass local module supervisor (phase 1)", version)]
struct Options {
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long)]
    module: PathBuf,
    #[arg(last = true)]
    module_args: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let options = Options::parse();
    let socket = match options
        .socket
        .or_else(|| std::env::var_os("SPIRITGLASS_SOCKET").map(PathBuf::from))
    {
        Some(path) => path,
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is unset; pass --socket")?
            .join(".spiritglass/run/control.sock"),
    };
    let (listener, _guard) = socket::bind(&socket)?;
    let (updates, _) = watch::channel(Snapshot::default());
    let (invocations, receiver) = mpsc::channel(32);
    let (shutdown, stopped) = watch::channel(false);
    let module_updates = updates.clone();
    let mut module_task = tokio::spawn(async move {
        module::supervise(
            &options.module,
            &options.module_args,
            receiver,
            module_updates,
            stopped,
        )
        .await
    });
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
                let invocations = invocations.clone();
                clients.spawn(async move {
                    if let Err(error) = client(stream, receiver, invocations).await { eprintln!("client disconnected: {error:#}"); }
                });
            }
            _ = clients.join_next(), if !clients.is_empty() => {}
        }
    }
    let _ = shutdown.send(true);
    clients.abort_all();
    if tokio::time::timeout(Duration::from_secs(8), &mut module_task)
        .await
        .is_err()
    {
        module_task.abort();
        let _ = module_task.await;
    }
    Ok(())
}

async fn client(
    stream: UnixStream,
    mut updates: watch::Receiver<Snapshot>,
    invocations: mpsc::Sender<Invocation>,
) -> Result<()> {
    let mut framed = Framed::new(stream, LinesCodec::new_with_max_length(MAX_MESSAGE_BYTES));
    let mut initialized = false;
    let mut subscribed = false;
    let mut last = Snapshot::default();
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
                        let params: InitializeParams = serde_json::from_value(request.params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                        if !params.protocol_versions.contains(&PROTOCOL_VERSION) { return Err(RpcError::new(-32001, "no compatible protocol version")); }
                        initialized = true;
                        return Ok(json!(InitializeResult { protocol_version: PROTOCOL_VERSION }));
                    }
                    if !initialized { return Err(RpcError::new(-32002, "initialize first")); }
                    match request.method.as_str() {
                        "session.subscribe" => {
                            serde_json::from_value::<EmptyParams>(request.params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                            subscribed = true;
                            last = updates.borrow_and_update().clone();
                            Ok(json!(last))
                        }
                        "action.invoke" => {
                            let params: ActionInvoke = serde_json::from_value(request.params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                            let snapshot = updates.borrow().clone();
                            if snapshot.health != ModuleHealth::Ready { return Err(RpcError::new(-32003, "module is unavailable")); }
                            let description = snapshot.module.as_ref().ok_or_else(|| RpcError::new(-32003, "module is unavailable"))?;
                            description.action(&params.action).and_then(|action| action.arguments.validate(&params.arguments)).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                            let (reply, receiver) = oneshot::channel();
                            invocations.try_send(Invocation { params, reply }).map_err(|_| RpcError::new(-32003, "module is unavailable or busy"))?;
                            tokio::time::timeout(Duration::from_secs(6), receiver).await.map_err(|_| RpcError::new(-32003, "action timed out; completion is unknown"))?.map_err(|_| RpcError::new(-32003, "module disconnected; completion is unknown"))?
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
                let message = if current.module != last.module || current.view != last.view {
                    Message::notification("session.snapshot", &current)
                } else { Message::notification("state.changed", StateChanged::from(&current)) };
                write(&mut framed, message).await?;
                last = current;
            }
        }
    }
}
async fn write(framed: &mut Framed<UnixStream, LinesCodec>, message: Message) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), framed.send(message.encode()?))
        .await
        .context("client write timed out")??;
    Ok(())
}
