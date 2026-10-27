//! A read-only local Git monitor. Repository work stays off the module's RPC loop.
use serde_json::{Value, json};
use spiritglass_protocol::*;
use spiritglass_sdk::Module;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: u64 = 128 * 1024;

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut child = Command::new("git")
        .arg("--no-pager")
        .arg("--no-optional-locks")
        .args(["-c", "core.fsmonitor=false", "-C"])
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let output = child.stdout.take().unwrap();
    let errors = child.stderr.take().unwrap();
    let read = |input: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            input
                .take(OUTPUT_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        })
    };
    let stdout = read(Box::new(output));
    let stderr = read(Box::new(errors));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < Duration::from_secs(1) => {
                thread::sleep(Duration::from_millis(10))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("Git inspection timed out".to_owned());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error.to_string());
            }
        }
    };
    let stdout = stdout
        .join()
        .map_err(|_| "Git output reader failed")?
        .map_err(|error| error.to_string())?;
    let stderr = stderr
        .join()
        .map_err(|_| "Git error reader failed")?
        .map_err(|error| error.to_string())?;
    if stdout.len() as u64 > OUTPUT_LIMIT || stderr.len() as u64 > OUTPUT_LIMIT {
        return Err("Repository output exceeds the monitoring limit".into());
    }
    if !status?.success() {
        return Err(String::from_utf8_lossy(&stderr)
            .trim()
            .chars()
            .take(256)
            .collect());
    }
    String::from_utf8(stdout).map_err(|_| "Repository output contains non-UTF-8 paths".into())
}

#[derive(Default)]
struct Counts {
    changed: u64,
    staged: u64,
    untracked: u64,
}
fn counts(status: &str) -> Counts {
    let mut counts = Counts::default();
    let mut records = status.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        if record.len() < 3 {
            continue;
        }
        let flags = record.as_bytes();
        counts.changed += 1;
        if &flags[..2] == b"??" {
            counts.untracked += 1;
        } else if flags[0] != b' ' && flags[0] != b'?' {
            counts.staged += 1;
        }
        if flags[..2].iter().any(|flag| *flag == b'R' || *flag == b'C') {
            records.next();
        }
    }
    counts
}

fn inspect(root: &Path) -> Result<Value, String> {
    let branch = git(root, &["symbolic-ref", "--short", "HEAD"])
        .or_else(|_| git(root, &["rev-parse", "--short", "HEAD"]))?;
    let status = git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
    )?;
    let counts = counts(&status);
    let upstream = git(
        root,
        &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
    );
    let (behind, ahead, tracked) = match upstream {
        Ok(value) => {
            let values = value
                .split_whitespace()
                .filter_map(|value| value.parse::<u64>().ok())
                .collect::<Vec<_>>();
            if values.len() != 2 {
                return Err("Unexpected Git tracking information".into());
            }
            (values[0], values[1], true)
        }
        Err(error) if error.contains("no upstream configured") => (0, 0, false),
        Err(error) => return Err(error),
    };
    let last_commit =
        git(root, &["log", "-1", "--format=%h · %s"]).unwrap_or_else(|_| "No commits yet".into());
    let message = if !tracked {
        "No upstream configured. Changes are monitored locally."
    } else if behind > 0 {
        "This branch is behind its last fetched upstream. Refresh your remote when convenient."
    } else if counts.changed > 0 {
        "You have local work in progress."
    } else if ahead > 0 {
        "Local commits are waiting to be pushed."
    } else {
        "Working tree clean and aligned with the last fetched upstream."
    };
    Ok(json!({
        "project": root.display().to_string(), "branch": format!("Branch · {}", branch.trim()), "changed": counts.changed,
        "staged": counts.staged, "untracked": counts.untracked, "ahead": ahead, "behind": behind,
        "status": if counts.changed > 0 || behind > 0 || ahead > 0 { "notice" } else { "clear" },
        "message": message, "last_commit": last_commit.trim().chars().take(256).collect::<String>(),
        "checked": "Checking…",
    }))
}

struct Observation {
    value: Value,
    checked: Option<Instant>,
}
struct ProjectMonitor {
    state: Arc<Mutex<Observation>>,
    refresh: mpsc::SyncSender<()>,
}
impl ProjectMonitor {
    fn new(root: PathBuf) -> Self {
        let state = Arc::new(Mutex::new(Observation {
            value: json!({"project": root.display().to_string(), "branch":"Checking…", "changed":0, "staged":0, "untracked":0, "ahead":0, "behind":0, "status":"unknown", "message":"Inspecting your local project…", "last_commit":"Checking…", "checked":"Waiting for the first inspection"}),
            checked: None,
        }));
        let (refresh, requests) = mpsc::sync_channel(1);
        let latest = state.clone();
        thread::spawn(move || {
            loop {
                match inspect(&root) {
                    Ok(value) => {
                        *latest.lock().unwrap() = Observation {
                            value,
                            checked: Some(Instant::now()),
                        }
                    }
                    Err(error) => {
                        let mut value = latest.lock().unwrap();
                        value.checked = Some(Instant::now());
                        value.value["status"] = json!("unknown");
                        value.value["message"] =
                            json!(format!("Couldn’t inspect this project: {error}"));
                    }
                }
                match requests.recv_timeout(Duration::from_secs(5)) {
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    _ => while requests.try_recv().is_ok() {},
                }
            }
        });
        Self { state, refresh }
    }
}
impl Module for ProjectMonitor {
    fn describe(&self) -> ModuleDescription {
        let mut properties = BTreeMap::new();
        for field in [
            "project",
            "branch",
            "status",
            "message",
            "last_commit",
            "checked",
        ] {
            properties.insert(field.into(), DataType::String);
        }
        for field in ["changed", "staged", "untracked", "ahead", "behind"] {
            properties.insert(field.into(), DataType::Integer);
        }
        ModuleDescription {
            id: "dev.spiritglass.project".into(),
            name: "Project watch".into(),
            description: "A quiet, read-only watch on your local Git project.".into(),
            view_id: "main".into(),
            sources: vec![SourceDefinition {
                id: "project".into(),
                schema: DataType::Object { properties },
            }],
            actions: vec![ActionDefinition {
                id: "refresh".into(),
                arguments: DataType::Object {
                    properties: BTreeMap::new(),
                },
            }],
        }
    }
    fn state(&self) -> Vec<StatePublish> {
        let observation = self.state.lock().unwrap();
        let mut value = observation.value.clone();
        if let Some(checked) = observation.checked {
            value["checked"] = json!(format!(
                "Checked {} seconds ago · local Git data only",
                checked.elapsed().as_secs()
            ));
            if checked.elapsed() > Duration::from_secs(15) {
                value["status"] = json!("unknown");
                value["message"] = json!("The project inspector hasn’t refreshed recently.");
            }
        }
        vec![StatePublish {
            source: "project".into(),
            value,
        }]
    }
    fn invoke(&mut self, action: &str, _arguments: Value) -> Result<Value, RpcError> {
        if action != "refresh" {
            return Err(RpcError::new(-32601, "unknown action"));
        }
        if let Err(mpsc::TrySendError::Disconnected(_)) = self.refresh.try_send(()) {
            return Err(RpcError::new(-32003, "project inspection worker stopped"));
        }
        Ok(json!({"refresh_requested":true}))
    }
    fn view(&self) -> Widget {
        let text = |id: &str, field: &str| Widget::Text {
            id: id.into(),
            value: Property::binding("project", &[field]),
        };
        let metric = |id: &str, label: &str, field: &str| Widget::Metric {
            id: id.into(),
            label: label.into(),
            value: Property::binding("project", &[field]),
            unit: None,
        };
        Widget::VStack {
            id: "project-dashboard".into(),
            children: vec![
                Widget::Card {
                    id: "project-summary".into(),
                    title: "Your project, at a glance".into(),
                    children: vec![
                        text("project-path", "project"),
                        text("project-branch", "branch"),
                        Widget::Status {
                            id: "project-status".into(),
                            label: "Working tree".into(),
                            value: Property::binding("project", &["status"]),
                        },
                        text("project-message", "message"),
                    ],
                },
                Widget::HStack {
                    id: "working-metrics".into(),
                    children: vec![
                        metric("changed", "Changed files", "changed"),
                        metric("staged", "Staged", "staged"),
                        metric("untracked", "Untracked", "untracked"),
                    ],
                },
                Widget::HStack {
                    id: "tracking-metrics".into(),
                    children: vec![
                        metric("ahead", "Commits ahead", "ahead"),
                        metric("behind", "Commits behind", "behind"),
                    ],
                },
                Widget::Card {
                    id: "project-details".into(),
                    title: "Latest activity".into(),
                    children: vec![
                        text("last-commit", "last_commit"),
                        text("last-checked", "checked"),
                        Widget::Button {
                            id: "refresh".into(),
                            label: "Check project now".into(),
                            action: "refresh".into(),
                            arguments: json!({}),
                        },
                    ],
                },
            ],
        }
    }
}
#[tokio::main]
async fn main() {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    if let Err(error) = spiritglass_sdk::run(ProjectMonitor::new(root)).await {
        eprintln!("project monitor: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_renames_once_and_distinguishes_staged_and_untracked() {
        let result = counts(" M changed\0M  staged\0?? new\0R  destination\0source\0");
        assert_eq!(result.changed, 4);
        assert_eq!(result.staged, 2);
        assert_eq!(result.untracked, 1);
    }
}
