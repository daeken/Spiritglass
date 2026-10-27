use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use spiritglass_protocol::*;
use std::{
    path::Path,
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot, watch},
};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

pub struct Invocation {
    pub params: ActionInvoke,
    pub reply: oneshot::Sender<Result<Value, RpcError>>,
}

struct Process {
    child: Child,
    input: FramedWrite<ChildStdin, LinesCodec>,
    output: FramedRead<ChildStdout, LinesCodec>,
    updates: watch::Sender<Snapshot>,
    next_id: u64,
    description: Option<ModuleDescription>,
    stderr: tokio::task::JoinHandle<()>,
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stderr.abort();
    }
}

impl Process {
    fn spawn(executable: &Path, args: &[String], updates: watch::Sender<Snapshot>) -> Result<Self> {
        let mut child = Command::new(executable)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("launch {}", executable.display()))?;
        let input = FramedWrite::new(
            child.stdin.take().unwrap(),
            LinesCodec::new_with_max_length(MAX_MESSAGE_BYTES),
        );
        let output = FramedRead::new(
            child.stdout.take().unwrap(),
            LinesCodec::new_with_max_length(MAX_MESSAGE_BYTES),
        );
        let stderr = child.stderr.take().unwrap();
        let stderr = tokio::spawn(async move {
            let mut lines = FramedRead::new(stderr, LinesCodec::new_with_max_length(8192));
            while let Some(line) = lines.next().await {
                match line {
                    Ok(line) => eprintln!("[module] {line}"),
                    Err(error) => {
                        eprintln!("[module stderr] {error}");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
            updates,
            next_id: 1,
            description: None,
            stderr,
        })
    }
    async fn send(&mut self, message: Message) -> Result<()> {
        self.input
            .send(message.encode()?)
            .await
            .context("write module message")
    }
    async fn next(&mut self) -> Result<Message> {
        let line = self
            .output
            .next()
            .await
            .context("module stdout closed")?
            .context("read module message")?;
        Message::parse(&line).context("invalid module message")
    }
    async fn incoming(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Notification(notification) if notification.method == "state.publish" => {
                let update: StatePublish =
                    serde_json::from_value(notification.params).context("invalid state.publish")?;
                let description = self
                    .description
                    .as_ref()
                    .context("state published before module.describe completed")?;
                description
                    .source(&update.source)?
                    .schema
                    .validate(&update.value)
                    .context("invalid source state")?;
                let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
                let mut snapshot = self.updates.borrow().clone();
                let revision = snapshot
                    .sources
                    .get(&update.source)
                    .map_or(1, |source| source.revision + 1);
                snapshot.sources.insert(
                    update.source,
                    SourceState {
                        value: update.value,
                        status: SourceStatus::Available,
                        revision,
                        updated_at_ms: now,
                    },
                );
                Message::notification("session.snapshot", &snapshot)
                    .encode()
                    .context("aggregate snapshot exceeds message limit")?;
                self.updates.send_replace(snapshot);
                Ok(())
            }
            Message::Request(request) => {
                self.send(Message::failure(
                    Some(request.id),
                    RpcError::new(-32601, "host capabilities are not implemented in version 1"),
                ))
                .await
            }
            _ => bail!("unexpected module message"),
        }
    }
    async fn request(
        &mut self,
        method: &str,
        params: impl serde::Serialize,
        duration: Duration,
    ) -> Result<Result<Value, RpcError>> {
        let id = RpcId::Number(self.next_id);
        self.next_id += 1;
        let request = Message::request(id.clone(), method, params);
        tokio::time::timeout(duration, async {
            self.send(request).await?;
            loop {
                match self.next().await? {
                    Message::Success(response) => {
                        ensure!(response.id == id, "unexpected response ID");
                        return Ok(Ok(response.result));
                    }
                    Message::Failure(response) => {
                        ensure!(
                            response.id.as_ref() == Some(&id),
                            "unexpected error response ID"
                        );
                        return Ok(Err(response.error));
                    }
                    message => self.incoming(message).await?,
                }
            }
        })
        .await
        .with_context(|| format!("module {method} timed out; completion is unknown"))?
    }
    async fn initialize(&mut self) -> Result<()> {
        let result = self
            .request(
                "initialize",
                InitializeParams {
                    protocol_versions: vec![PROTOCOL_VERSION],
                },
                Duration::from_secs(5),
            )
            .await??;
        let result: InitializeResult = serde_json::from_value(result)?;
        ensure!(
            result.protocol_version == PROTOCOL_VERSION,
            "module selected unsupported protocol version"
        );
        let description: ModuleDescription = serde_json::from_value(
            self.request("module.describe", EmptyParams {}, Duration::from_secs(5))
                .await??,
        )?;
        description.validate()?;
        self.description = Some(description.clone());
        let view: Widget = serde_json::from_value(
            self.request(
                "view.get",
                ViewGet {
                    view_id: description.view_id.clone(),
                },
                Duration::from_secs(5),
            )
            .await??,
        )?;
        view.validate(&description)?;
        let mut snapshot = self.updates.borrow().clone();
        snapshot.module = Some(description);
        snapshot.view = Some(view);
        snapshot.health = ModuleHealth::Ready;
        Message::notification("session.snapshot", &snapshot).encode()?;
        self.updates.send_replace(snapshot);
        Ok(())
    }
    async fn stop(&mut self) {
        let _ = self
            .request("shutdown", EmptyParams {}, Duration::from_secs(1))
            .await;
        if tokio::time::timeout(Duration::from_secs(1), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
            let _ = self.child.wait().await;
        }
    }
}

pub async fn supervise(
    executable: &Path,
    args: &[String],
    mut invocations: mpsc::Receiver<Invocation>,
    updates: watch::Sender<Snapshot>,
    mut shutdown: watch::Receiver<bool>,
) {
    let result = async {
        let mut process = Process::spawn(executable, args, updates.clone())?;
        // Startup is cancelable, including modules that never answer initialize.
        tokio::select! {
            result = process.initialize() => result?,
            _ = shutdown.changed() => { process.stop().await; return Ok::<(), anyhow::Error>(()); }
        }
        loop {
            tokio::select! {
                _ = shutdown.changed() => { process.stop().await; return Ok(()); }
                message = process.next() => process.incoming(message?).await?,
                invocation = invocations.recv() => {
                    let Some(invocation) = invocation else { process.stop().await; return Ok(()); };
                    if invocation.reply.is_closed() { continue; }
                    match process.request("action.invoke", invocation.params, Duration::from_secs(5)).await {
                        Ok(response) => { let _ = invocation.reply.send(response); }
                        Err(error) => {
                            let _ = invocation.reply.send(Err(RpcError::new(-32003, "module action failed; completion is unknown")));
                            return Err(error);
                        }
                    }
                }
            }
        }
    }.await;
    if let Err(error) = result {
        let diagnostic = format!("{error:#}").chars().take(1024).collect::<String>();
        eprintln!("module unavailable: {diagnostic}");
        updates.send_modify(|snapshot| {
            snapshot.health = ModuleHealth::Unavailable;
            snapshot.diagnostic = Some(diagnostic);
            for source in snapshot.sources.values_mut() {
                source.status = SourceStatus::Stale;
            }
        });
    }
}
