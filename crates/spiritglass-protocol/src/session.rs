//! Multi-module client protocol. Module stdio remains on version 1.
use crate::*;
use std::path::PathBuf;

pub const SESSION_PROTOCOL_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePolicy {
    #[default]
    Always,
    OnDemand,
}

fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleManifest {
    pub id: String,
    pub name: String,
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<PathBuf>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub lifecycle: LifecyclePolicy,
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// Additional files to watch, such as an interpreted module's script or DLL.
    #[serde(default)]
    pub watch: Vec<PathBuf>,
    /// Optional source freshness budget. A quiet process is not assumed healthy.
    #[serde(default)]
    pub stale_after_ms: Option<u64>,
}
impl ModuleManifest {
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id)?;
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 256,
            "manifest name must contain 1–256 bytes"
        );
        ensure!(
            !self.executable.as_os_str().is_empty(),
            "manifest executable is empty"
        );
        ensure!(
            self.args.len() <= 64 && self.watch.len() <= 32 && self.environment.len() <= 64,
            "manifest has too many arguments, watched files, or environment variables"
        );
        ensure!(
            self.stale_after_ms
                .is_none_or(|value| (1000..=86_400_000).contains(&value)),
            "stale_after_ms must be between 1000 and 86400000"
        );
        ensure!(
            self.environment
                .keys()
                .all(|key| !key.is_empty() && !key.contains(['=', '\0'])),
            "invalid environment variable name"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModulePhase {
    Disabled,
    Dormant,
    Starting,
    Ready,
    Restarting,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub at_ms: u64,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedModule {
    pub id: String,
    pub name: String,
    pub lifecycle: LifecyclePolicy,
    pub enabled: bool,
    pub phase: ModulePhase,
    pub generation: u64,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub reloading: bool,
    pub reload_error: Option<String>,
    pub history: Vec<Diagnostic>,
    pub state: Snapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionSnapshot {
    pub protocol_version: u32,
    pub modules: BTreeMap<String, ManagedModule>,
    pub diagnostics: Vec<Diagnostic>,
}
impl Default for SessionSnapshot {
    fn default() -> Self {
        Self {
            protocol_version: SESSION_PROTOCOL_VERSION,
            modules: BTreeMap::new(),
            diagnostics: vec![],
        }
    }
}
impl SessionSnapshot {
    /// Source-only changes travel without decoding or replacing dashboard trees.
    pub fn same_structure(&self, other: &Self) -> bool {
        let strip = |snapshot: &Self| {
            let mut value = snapshot.clone();
            for module in value.modules.values_mut() {
                module.state.sources.clear();
            }
            value
        };
        strip(self) == strip(other)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionStateChanged {
    pub modules: BTreeMap<String, StateChanged>,
}
impl From<&SessionSnapshot> for SessionStateChanged {
    fn from(snapshot: &SessionSnapshot) -> Self {
        Self {
            modules: snapshot
                .modules
                .iter()
                .map(|(id, module)| (id.clone(), StateChanged::from(&module.state)))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleTarget {
    pub module_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionActionInvoke {
    pub module_id: String,
    pub action: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionActionResult {
    pub generation: u64,
    pub active_generation: Option<u64>,
    pub value: Value,
}

#[derive(JsonSchema)]
pub struct SessionProtocolCatalog {
    pub message: Message,
    pub initialize_params: InitializeParams,
    pub initialize_result: InitializeResult,
    pub snapshot: SessionSnapshot,
    pub state_changed: SessionStateChanged,
    pub module_target: ModuleTarget,
    pub action_invoke: SessionActionInvoke,
    pub action_result: SessionActionResult,
    pub manifest: ModuleManifest,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_matches_version_two() {
        let generated =
            serde_json::to_value(schemars::schema_for!(SessionProtocolCatalog)).unwrap();
        let committed: Value =
            serde_json::from_str(include_str!("../../../protocol/v2.schema.json")).unwrap();
        assert_eq!(generated, committed, "run the schema exporter");
    }
    #[test]
    fn manifest_defaults_and_validation() {
        let mut manifest: ModuleManifest =
            serde_json::from_value(json!({"id":"test", "name":"Test", "executable":"./module"}))
                .unwrap();
        assert_eq!(manifest.lifecycle, LifecyclePolicy::Always);
        assert!(manifest.enabled);
        manifest.validate().unwrap();
        manifest.stale_after_ms = Some(0);
        assert!(manifest.validate().is_err());
        assert!(
            serde_json::from_value::<ModuleManifest>(
                json!({"id":"test", "name":"Test", "executable":"./module", "typo":true})
            )
            .is_err()
        );
    }

    #[test]
    fn shared_client_fixture_contains_valid_independent_modules() {
        let session: SessionSnapshot = serde_json::from_str(include_str!(
            "../../../protocol/fixtures/session.snapshot.json"
        ))
        .unwrap();
        assert_eq!(session.protocol_version, SESSION_PROTOCOL_VERSION);
        assert_eq!(session.modules.len(), 2);
        for (id, module) in session.modules {
            assert_eq!(id, module.id);
            let description = module.state.module.unwrap();
            assert_eq!(id, description.id);
            description.validate().unwrap();
            module.state.view.unwrap().validate(&description).unwrap();
            for (id, source) in module.state.sources {
                description
                    .source(&id)
                    .unwrap()
                    .schema
                    .validate(&source.value)
                    .unwrap();
            }
        }
    }
}
