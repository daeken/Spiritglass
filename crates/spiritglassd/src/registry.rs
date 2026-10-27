use crate::{
    discovery::{self, Config, FileStamp},
    module::{self, Event, Handle, Invocation, Stop},
};
use anyhow::Result;
use serde_json::{Value, json};
use spiritglass_protocol::*;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};

pub enum Control {
    Reload,
    Activate,
    Deactivate,
}
pub enum Command {
    Invoke {
        params: SessionActionInvoke,
        reply: oneshot::Sender<Result<SessionActionResult, RpcError>>,
    },
    Control {
        target: ModuleTarget,
        control: Control,
        reply: oneshot::Sender<Result<Value, RpcError>>,
    },
}
pub enum Installation {
    Directory(PathBuf),
    Single(Config),
}

struct Candidate {
    token: u64,
    stamp: Vec<FileStamp>,
    config: Config,
}
struct Entry {
    config: Config,
    desired: bool,
    active: Option<u64>,
    candidate: Option<Candidate>,
    observed: Option<Vec<FileStamp>>,
    settled: bool,
    attempted: Option<Vec<FileStamp>>,
    retry_at: Instant,
    failures: u32,
    launched: bool,
    healthy_since: Option<Instant>,
}

struct Registry {
    installation: Installation,
    entries: BTreeMap<String, Entry>,
    generations: BTreeMap<(String, u64), Handle>,
    session: SessionSnapshot,
    updates: watch::Sender<SessionSnapshot>,
    events: mpsc::Sender<Event>,
    next_token: u64,
    discovery_errors: Vec<String>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn short(message: impl AsRef<str>) -> String {
    let mut bytes = 0;
    message
        .as_ref()
        .chars()
        .take_while(|character| {
            bytes += character.len_utf8();
            bytes <= 256
        })
        .collect()
}
fn history(module: &mut ManagedModule, message: impl AsRef<str>) {
    module.history.push(Diagnostic {
        at_ms: now_ms(),
        message: short(message),
    });
    if module.history.len() > 8 {
        module.history.remove(0);
    }
}
fn stale(module: &mut ManagedModule, message: Option<String>) {
    module.state.health = ModuleHealth::Unavailable;
    module.state.diagnostic = message;
    module.pid = None;
    for source in module.state.sources.values_mut() {
        source.status = SourceStatus::Stale;
    }
}

pub async fn run(
    installation: Installation,
    updates: watch::Sender<SessionSnapshot>,
    mut commands: mpsc::Receiver<Command>,
    mut shutdown: watch::Receiver<bool>,
) {
    let (events, mut incoming) = mpsc::channel(128);
    let mut registry = Registry {
        installation,
        entries: BTreeMap::new(),
        generations: BTreeMap::new(),
        session: SessionSnapshot::default(),
        updates,
        events,
        next_token: 1,
        discovery_errors: vec![],
    };
    let mut timer = tokio::time::interval(Duration::from_millis(500));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            Some(command) = commands.recv() => registry.command(command),
            Some(event) = incoming.recv() => registry.event(event),
            _ = timer.tick() => registry.tick(),
        }
        registry.publish();
    }
    for generation in registry.generations.values() {
        generation.stop(Stop::Now);
    }
    let joined = async {
        for generation in registry.generations.values_mut() {
            let _ = (&mut generation.task).await;
        }
    };
    if tokio::time::timeout(Duration::from_secs(8), joined)
        .await
        .is_err()
    {
        for generation in registry.generations.values() {
            generation.task.abort();
        }
    }
}

impl Registry {
    fn publish(&self) {
        if *self.updates.borrow() != self.session {
            self.updates.send_replace(self.session.clone());
        }
    }
    fn fits(&self, id: &str, module: &ManagedModule) -> bool {
        let mut next = self.session.clone();
        next.modules.insert(id.into(), module.clone());
        // Reserve capacity for bounded health, diagnostic, and log metadata growth.
        Message::notification("session.snapshot", next)
            .encode()
            .is_ok_and(|line| line.len() < MAX_MESSAGE_BYTES - 128 * 1024)
    }
    fn diagnostics(&mut self, errors: Vec<String>) {
        if self.discovery_errors == errors {
            return;
        }
        self.discovery_errors = errors;
        self.session.diagnostics = self
            .discovery_errors
            .iter()
            .take(32)
            .map(|message| Diagnostic {
                at_ms: now_ms(),
                message: short(message),
            })
            .collect();
    }
    fn discover(&mut self) {
        let configs = match &self.installation {
            Installation::Single(config) => vec![config.clone()],
            Installation::Directory(directory) => match discovery::scan(directory) {
                Ok(scan) => {
                    let removed = self
                        .entries
                        .iter()
                        .filter(|(_, entry)| {
                            entry
                                .config
                                .path
                                .as_ref()
                                .is_some_and(|path| !scan.present.contains(path))
                        })
                        .map(|(id, _)| id.clone())
                        .collect::<Vec<_>>();
                    for id in removed {
                        self.remove(&id);
                    }
                    self.diagnostics(scan.errors);
                    scan.configs.into_values().collect()
                }
                Err(error) => {
                    self.diagnostics(vec![format!("{error:#}")]);
                    return;
                }
            },
        };
        for config in configs {
            let id = config.manifest.id.clone();
            // A valid ID change in the same manifest is an uninstall followed by install.
            let renamed = self
                .entries
                .iter()
                .filter(|(other_id, entry)| {
                    **other_id != id && config.path.is_some() && config.path == entry.config.path
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for old in renamed {
                self.remove(&old);
            }
            if let Some(entry) = self.entries.get_mut(&id) {
                if entry.config != config {
                    let was_always = entry.config.manifest.lifecycle == LifecyclePolicy::Always;
                    entry.config = config.clone();
                    entry.attempted = None;
                    entry.observed = None;
                    entry.settled = false;
                    if config.manifest.lifecycle == LifecyclePolicy::Always {
                        entry.desired = true;
                    } else if was_always && entry.active.is_none() {
                        entry.desired = false;
                    }
                    if let Some(candidate) = entry.candidate.take()
                        && let Some(handle) = self.generations.get(&(id.clone(), candidate.token))
                    {
                        handle.stop(Stop::Now);
                    }
                    let module = self.session.modules.get_mut(&id).unwrap();
                    module.name = config.manifest.name.clone();
                    module.lifecycle = config.manifest.lifecycle;
                    module.enabled = config.manifest.enabled;
                    module.reloading = false;
                    history(module, "Manifest updated");
                }
            } else {
                if self.entries.len() >= discovery::MAX_MODULES {
                    continue;
                }
                let desired = config.manifest.lifecycle == LifecyclePolicy::Always;
                let phase = if !config.manifest.enabled {
                    ModulePhase::Disabled
                } else if desired {
                    ModulePhase::Starting
                } else {
                    ModulePhase::Dormant
                };
                self.session.modules.insert(
                    id.clone(),
                    ManagedModule {
                        id: id.clone(),
                        name: config.manifest.name.clone(),
                        lifecycle: config.manifest.lifecycle,
                        enabled: config.manifest.enabled,
                        phase,
                        generation: 0,
                        pid: None,
                        restart_count: 0,
                        reloading: false,
                        reload_error: None,
                        history: vec![],
                        state: Snapshot::default(),
                    },
                );
                self.entries.insert(
                    id,
                    Entry {
                        config,
                        desired,
                        active: None,
                        candidate: None,
                        observed: None,
                        settled: false,
                        attempted: None,
                        retry_at: Instant::now(),
                        failures: 0,
                        launched: false,
                        healthy_since: None,
                    },
                );
            }
        }
    }
    fn remove(&mut self, id: &str) {
        if let Some(entry) = self.entries.remove(id) {
            for token in entry
                .active
                .into_iter()
                .chain(entry.candidate.map(|candidate| candidate.token))
            {
                if let Some(handle) = self.generations.get(&(id.into(), token)) {
                    handle.stop(Stop::Now);
                }
            }
        }
        self.session.modules.remove(id);
    }
    fn stop_entry(&mut self, id: &str, phase: ModulePhase) {
        let entry = self.entries.get_mut(id).unwrap();
        for token in entry
            .active
            .take()
            .into_iter()
            .chain(entry.candidate.take().map(|candidate| candidate.token))
        {
            if let Some(handle) = self.generations.get(&(id.into(), token)) {
                handle.stop(Stop::Now);
            }
        }
        let module = self.session.modules.get_mut(id).unwrap();
        if module.phase != phase {
            history(
                module,
                format!(
                    "Module {}",
                    if phase == ModulePhase::Disabled {
                        "disabled"
                    } else {
                        "stopped"
                    }
                ),
            );
        }
        module.phase = phase;
        module.reloading = false;
        module.reload_error = None;
        stale(module, None);
    }
    fn tick(&mut self) {
        self.discover();
        let ids = self.entries.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let entry = &self.entries[&id];
            if !entry.config.manifest.enabled {
                self.stop_entry(&id, ModulePhase::Disabled);
                continue;
            }
            if !entry.desired {
                self.stop_entry(&id, ModulePhase::Dormant);
                continue;
            }
            let fingerprint = entry.config.fingerprint();
            match fingerprint {
                Ok(stamp) => {
                    let entry = self.entries.get_mut(&id).unwrap();
                    if entry.observed.as_ref() != Some(&stamp) {
                        entry.observed = Some(stamp.clone());
                        entry.settled = false;
                    } else {
                        entry.settled = true;
                    }
                    let changed = entry.attempted.as_ref() != Some(&stamp);
                    if entry.candidate.is_none()
                        && entry.settled
                        && (changed || (entry.active.is_none() && Instant::now() >= entry.retry_at))
                    {
                        self.start(&id, stamp);
                    }
                }
                Err(error) => {
                    let message = short(format!("{error:#}"));
                    let module = self.session.modules.get_mut(&id).unwrap();
                    if self.entries[&id].active.is_some() {
                        if module.reload_error.as_ref() != Some(&message) {
                            module.reload_error = Some(message.clone());
                            history(module, &message);
                        }
                    } else {
                        module.phase = ModulePhase::Unavailable;
                        if module.state.diagnostic.as_ref() != Some(&message) {
                            stale(module, Some(message.clone()));
                            history(module, &message);
                        }
                    }
                }
            }
            let module = self.session.modules.get_mut(&id).unwrap();
            if let Some(ttl) = self.entries[&id].config.manifest.stale_after_ms {
                for source in module.state.sources.values_mut() {
                    if now_ms().saturating_sub(source.updated_at_ms) > ttl {
                        source.status = SourceStatus::Stale;
                    }
                }
            }
        }
    }
    fn start(&mut self, id: &str, stamp: Vec<FileStamp>) {
        if self.generations.len() >= discovery::MAX_MODULES * 3 {
            return;
        }
        let entry = self.entries.get_mut(id).unwrap();
        let token = self.next_token;
        self.next_token += 1;
        if let Some(candidate) = entry.candidate.take()
            && let Some(handle) = self.generations.get(&(id.into(), candidate.token))
        {
            handle.stop(Stop::Now);
        }
        entry.attempted = Some(stamp.clone());
        let config = entry.config.clone();
        entry.candidate = Some(Candidate {
            token,
            stamp,
            config: config.clone(),
        });
        let module = self.session.modules.get_mut(id).unwrap();
        module.reloading = entry.active.is_some();
        if entry.active.is_none() {
            if entry.launched {
                module.restart_count = module.restart_count.saturating_add(1);
            }
            module.phase = if entry.launched {
                ModulePhase::Restarting
            } else {
                ModulePhase::Starting
            };
        }
        entry.launched = true;
        history(module, format!("Validating generation {token}"));
        self.generations.insert(
            (id.into(), token),
            module::launch(config, token, self.events.clone()),
        );
    }
    fn fail(&mut self, id: &str, message: String) {
        let message = short(message);
        let entry = self.entries.get_mut(id).unwrap();
        let module = self.session.modules.get_mut(id).unwrap();
        history(module, &message);
        module.reloading = false;
        if entry.active.is_some() {
            module.reload_error = Some(message);
        } else {
            entry.failures = entry.failures.saturating_add(1);
            let delay = (250_u64 * 2_u64.pow(entry.failures.saturating_sub(1).min(7))).min(30_000);
            entry.retry_at = Instant::now() + Duration::from_millis(delay);
            module.phase = ModulePhase::Restarting;
            stale(
                module,
                Some(format!("{message} · retrying in {} ms", delay)),
            );
        }
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::Ready {
                id,
                token,
                pid,
                state,
            } => {
                let Some(entry) = self.entries.get(&id) else {
                    return;
                };
                let Some(candidate) = &entry.candidate else {
                    return;
                };
                if candidate.token != token {
                    return;
                }
                let unchanged = candidate.config == entry.config
                    && entry.config.fingerprint().ok().as_ref() == Some(&candidate.stamp);
                if !unchanged || !entry.desired || !entry.config.manifest.enabled {
                    self.generations[&(id.clone(), token)].stop(Stop::Now);
                    self.entries.get_mut(&id).unwrap().candidate = None;
                    self.session.modules.get_mut(&id).unwrap().reloading = false;
                    return;
                }
                let mut module = self.session.modules[&id].clone();
                module.state = state;
                module.phase = ModulePhase::Ready;
                module.generation = token;
                module.pid = pid;
                module.reloading = false;
                module.reload_error = None;
                if let Some(ttl) = entry.config.manifest.stale_after_ms
                    && module
                        .state
                        .sources
                        .values()
                        .any(|source| now_ms().saturating_sub(source.updated_at_ms) > ttl)
                {
                    self.generations[&(id.clone(), token)].stop(Stop::Now);
                    self.entries.get_mut(&id).unwrap().candidate = None;
                    self.fail(&id, "Candidate initial sources are already stale".into());
                    return;
                }
                history(&mut module, format!("Generation {token} is live"));
                if !self.fits(&id, &module) {
                    self.generations[&(id.clone(), token)].stop(Stop::Now);
                    self.entries.get_mut(&id).unwrap().candidate = None;
                    self.fail(
                        &id,
                        "Candidate exceeds the shared session snapshot budget".into(),
                    );
                    return;
                }
                let entry = self.entries.get_mut(&id).unwrap();
                if let Some(old) = entry.active.replace(token) {
                    self.generations[&(id.clone(), old)].stop(Stop::Retire);
                }
                entry.candidate = None;
                entry.healthy_since = Some(Instant::now());
                self.session.modules.insert(id, module);
            }
            Event::State { id, token, state } => {
                if self.entries.get(&id).and_then(|entry| entry.active) != Some(token) {
                    return;
                }
                let mut module = self.session.modules[&id].clone();
                module.state = state;
                if let Some(ttl) = self.entries[&id].config.manifest.stale_after_ms {
                    for source in module.state.sources.values_mut() {
                        if now_ms().saturating_sub(source.updated_at_ms) > ttl {
                            source.status = SourceStatus::Stale;
                        }
                    }
                }
                if self.fits(&id, &module) {
                    self.session.modules.insert(id, module);
                } else {
                    self.generations[&(id.clone(), token)].stop(Stop::Now);
                    self.entries.get_mut(&id).unwrap().active = None;
                    self.fail(
                        &id,
                        "Source update exceeds the shared session snapshot budget".into(),
                    );
                }
            }
            Event::Exited { id, token, error } => {
                self.generations.remove(&(id.clone(), token));
                let Some(entry) = self.entries.get_mut(&id) else {
                    return;
                };
                if entry
                    .candidate
                    .as_ref()
                    .is_some_and(|candidate| candidate.token == token)
                {
                    entry.candidate = None;
                    self.fail(
                        &id,
                        error.unwrap_or_else(|| "Candidate stopped before becoming healthy".into()),
                    );
                } else if entry.active == Some(token) {
                    entry.active = None;
                    if entry
                        .healthy_since
                        .is_some_and(|since| since.elapsed() > Duration::from_secs(30))
                    {
                        entry.failures = 0;
                    }
                    self.fail(&id, error.unwrap_or_else(|| "Module exited".into()));
                }
            }
            Event::Log { id, token, message } => {
                if let Some(entry) = self.entries.get(&id)
                    && (entry.active == Some(token)
                        || entry
                            .candidate
                            .as_ref()
                            .is_some_and(|candidate| candidate.token == token))
                {
                    history(
                        self.session.modules.get_mut(&id).unwrap(),
                        format!("[{token}] {message}"),
                    );
                }
            }
        }
    }
    fn command(&mut self, command: Command) {
        match command {
            Command::Invoke { params, reply } => match self.validate_action(&params) {
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
                Ok(token) => {
                    let invocation = Invocation {
                        params: ActionInvoke {
                            action: params.action,
                            arguments: params.arguments,
                        },
                        reply,
                    };
                    if let Some(handle) = self.generations.get(&(params.module_id, token)) {
                        if let Err(error) = handle.actions.try_send(invocation) {
                            let _ = error
                                .into_inner()
                                .reply
                                .send(Err(RpcError::new(-32003, "module is busy or unavailable")));
                        }
                    } else {
                        let _ = invocation
                            .reply
                            .send(Err(RpcError::new(-32003, "module is unavailable")));
                    }
                }
            },
            Command::Control {
                target,
                control,
                reply,
            } => {
                let result = self.control(&target.module_id, control);
                let _ = reply.send(result);
            }
        }
    }
    fn validate_action(&self, params: &SessionActionInvoke) -> Result<u64, RpcError> {
        let entry = self
            .entries
            .get(&params.module_id)
            .ok_or_else(|| RpcError::invalid_params("unknown module"))?;
        let token = entry
            .active
            .ok_or_else(|| RpcError::new(-32003, "module is not ready"))?;
        let module = &self.session.modules[&params.module_id];
        if module.phase != ModulePhase::Ready {
            return Err(RpcError::new(-32003, "module is not ready"));
        }
        let description = module
            .state
            .module
            .as_ref()
            .ok_or_else(|| RpcError::new(-32003, "module has no declaration"))?;
        description
            .action(&params.action)
            .and_then(|action| action.arguments.validate(&params.arguments))
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        Ok(token)
    }
    fn control(&mut self, id: &str, control: Control) -> Result<Value, RpcError> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| RpcError::invalid_params("unknown module"))?;
        if !entry.config.manifest.enabled {
            return Err(RpcError::new(-32003, "module is disabled in its manifest"));
        }
        match control {
            Control::Deactivate => {
                if entry.config.manifest.lifecycle != LifecyclePolicy::OnDemand {
                    return Err(RpcError::invalid_params(
                        "only on-demand modules can be stopped",
                    ));
                }
                entry.desired = false;
                self.stop_entry(id, ModulePhase::Dormant);
            }
            Control::Activate => {
                entry.desired = true;
                entry.retry_at = Instant::now();
            }
            Control::Reload => {
                entry.desired = true;
                entry.retry_at = Instant::now();
                let stamp = entry
                    .config
                    .fingerprint()
                    .map_err(|error| RpcError::new(-32003, error.to_string()))?;
                self.start(id, stamp);
            }
        }
        Ok(json!({"accepted":true}))
    }
}
