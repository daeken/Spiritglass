use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use spiritglass_protocol::*;
use std::{
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, watch},
};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

pub struct Process {
    pub child: Child,
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
    pub fn spawn(
        config: &crate::discovery::Config,
        updates: watch::Sender<Snapshot>,
        events: mpsc::Sender<crate::module::Event>,
        token: u64,
    ) -> Result<Self> {
        let executable = &config.manifest.executable;
        let mut command = Command::new(executable);
        command
            .args(&config.manifest.args)
            .envs(&config.manifest.environment);
        if let Some(directory) = &config.manifest.working_directory {
            command.current_dir(directory);
        }
        let mut child = command
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
        let mut stderr = child.stderr.take().unwrap();
        let id = config.manifest.id.clone();
        let stderr = tokio::spawn(async move {
            let mut buffer = [0; 4096];
            let mut line = Vec::new();
            let mut window = tokio::time::Instant::now();
            let mut count = 0;
            while let Ok(length) = stderr.read(&mut buffer).await {
                if length == 0 {
                    break;
                }
                for byte in &buffer[..length] {
                    if *byte == b'\n' {
                        if window.elapsed() >= Duration::from_secs(1) {
                            window = tokio::time::Instant::now();
                            count = 0;
                        }
                        if count < 20 && !line.is_empty() {
                            let message = String::from_utf8_lossy(&line)
                                .chars()
                                .take(512)
                                .collect::<String>();
                            eprintln!("[{id}] {message}");
                            let _ = events.try_send(crate::module::Event::Log {
                                id: id.clone(),
                                token,
                                message,
                            });
                            count += 1;
                        }
                        line.clear();
                    } else if line.len() < 8192 {
                        line.push(*byte);
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
    pub async fn next(&mut self) -> Result<Message> {
        let line = self
            .output
            .next()
            .await
            .context("module stdout closed")?
            .context("read module message")?;
        Message::parse(&line).context("invalid module message")
    }
    pub async fn incoming(&mut self, message: Message) -> Result<()> {
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
    pub async fn request(
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
    pub async fn initialize(&mut self, expected_id: Option<&str>) -> Result<()> {
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
        if let Some(expected) = expected_id {
            ensure!(
                description.id == expected,
                "module ID {} does not match manifest {expected}",
                description.id
            );
        }
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
        Message::notification("session.snapshot", &snapshot).encode()?;
        self.updates.send_replace(snapshot);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let complete = {
                    let state = self.updates.borrow();
                    self.description
                        .as_ref()
                        .unwrap()
                        .sources
                        .iter()
                        .all(|source| state.sources.contains_key(&source.id))
                };
                if complete {
                    break;
                }
                let message = self.next().await?;
                self.incoming(message).await?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("module did not publish all initial sources within five seconds")??;
        self.updates
            .send_modify(|snapshot| snapshot.health = ModuleHealth::Ready);
        Ok(())
    }
    pub async fn stop(&mut self) {
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
