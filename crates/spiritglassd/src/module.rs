use crate::{discovery::Config, process::Process};
use anyhow::{Context, Result};
use serde_json::Value;
use spiritglass_protocol::*;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

pub struct Invocation {
    pub params: ActionInvoke,
    pub reply: oneshot::Sender<Result<SessionActionResult, RpcError>>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Stop {
    Running,
    Retire,
    Now,
}

pub struct Handle {
    pub actions: mpsc::Sender<Invocation>,
    pub stop: watch::Sender<Stop>,
    pub task: tokio::task::JoinHandle<()>,
}
impl Handle {
    pub fn stop(&self, mode: Stop) {
        self.stop.send_replace(mode);
    }
}

#[derive(Debug)]
pub enum Event {
    Ready {
        id: String,
        token: u64,
        pid: Option<u32>,
        state: Snapshot,
    },
    State {
        id: String,
        token: u64,
        state: Snapshot,
    },
    Exited {
        id: String,
        token: u64,
        error: Option<String>,
    },
    Log {
        id: String,
        token: u64,
        message: String,
    },
}

pub fn launch(config: Config, token: u64, events: mpsc::Sender<Event>) -> Handle {
    let (actions, receiver) = mpsc::channel(32);
    let (stop, stopping) = watch::channel(Stop::Running);
    let task = tokio::spawn(async move {
        let id = config.manifest.id.clone();
        let result = run(config, token, receiver, stopping, events.clone()).await;
        let error = result
            .err()
            .map(|error| format!("{error:#}").chars().take(512).collect::<String>());
        if let Some(error) = &error {
            eprintln!("[{id}] unavailable: {error}");
        }
        let _ = events.send(Event::Exited { id, token, error }).await;
    });
    Handle {
        actions,
        stop,
        task,
    }
}

struct Observer(tokio::task::JoinHandle<()>);
impl Drop for Observer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn run(
    config: Config,
    token: u64,
    mut actions: mpsc::Receiver<Invocation>,
    mut stopping: watch::Receiver<Stop>,
    events: mpsc::Sender<Event>,
) -> Result<()> {
    let id = config.manifest.id.clone();
    let (updates, _) = watch::channel(Snapshot::default());
    let mut process = Process::spawn(&config, updates.clone(), events.clone(), token)?;
    tokio::select! {
        biased;
        _ = stopping.changed() => { process.stop().await; return Ok(()); },
        result = process.initialize(config.enforce_id.then_some(id.as_str())) => result?,
    }
    let mut observer = updates.subscribe();
    let state = observer.borrow_and_update().clone();
    events
        .send(Event::Ready {
            id: id.clone(),
            token,
            pid: process.child.id(),
            state,
        })
        .await
        .context("supervisor stopped")?;
    let observer_events = events.clone();
    let _observer = Observer(tokio::spawn(async move {
        while observer.changed().await.is_ok() {
            let state = observer.borrow_and_update().clone();
            if observer_events
                .send(Event::State {
                    id: id.clone(),
                    token,
                    state,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    }));
    loop {
        let mode = *stopping.borrow_and_update();
        if mode != Stop::Running {
            actions.close();
            if mode == Stop::Retire {
                // Calls accepted before promotion finish on their original process.
                let _ = tokio::time::timeout(Duration::from_secs(6), async {
                    while let Some(action) = actions.recv().await {
                        invoke(&mut process, token, action).await?;
                    }
                    Ok::<(), anyhow::Error>(())
                })
                .await;
            }
            process.stop().await;
            return Ok(());
        }
        tokio::select! {
            _ = stopping.changed() => {},
            message = process.next() => { let message = message?; process.incoming(message).await?; },
            action = actions.recv() => {
                let Some(action) = action else { process.stop().await; return Ok(()); };
                invoke(&mut process, token, action).await?;
            }
        }
    }
}

async fn invoke(process: &mut Process, token: u64, action: Invocation) -> Result<()> {
    if action.reply.is_closed() {
        return Ok(());
    }
    match process
        .request("action.invoke", action.params, Duration::from_secs(5))
        .await
    {
        Ok(response) => {
            let result = response.map(|value: Value| SessionActionResult {
                generation: token,
                active_generation: None,
                value,
            });
            let _ = action.reply.send(result);
            Ok(())
        }
        Err(error) => {
            let _ = action.reply.send(Err(RpcError::new(
                -32003,
                "module action failed; completion is unknown",
            )));
            Err(error)
        }
    }
}
