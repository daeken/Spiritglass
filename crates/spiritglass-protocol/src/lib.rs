//! Version 1 wire types. JSON schemas are generated from these definitions.
use anyhow::{Result, bail, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RpcId {
    Number(u64),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum RpcVersion {
    #[serde(rename = "2.0")]
    V2,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub jsonrpc: RpcVersion,
    pub id: RpcId,
    pub method: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Notification {
    pub jsonrpc: RpcVersion,
    pub method: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Success {
    pub jsonrpc: RpcVersion,
    pub id: RpcId,
    pub result: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub jsonrpc: RpcVersion,
    pub id: Option<RpcId>,
    pub error: RpcError,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }
}
impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}
impl std::error::Error for RpcError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Notification(Notification),
    Success(Success),
    Failure(Failure),
}

fn empty_object() -> Value {
    json!({})
}

impl Message {
    pub fn request(id: impl Into<RpcId>, method: &str, params: impl Serialize) -> Self {
        Self::Request(Request {
            jsonrpc: RpcVersion::V2,
            id: id.into(),
            method: method.into(),
            params: serde_json::to_value(params).expect("wire parameters must serialize"),
        })
    }
    pub fn notification(method: &str, params: impl Serialize) -> Self {
        Self::Notification(Notification {
            jsonrpc: RpcVersion::V2,
            method: method.into(),
            params: serde_json::to_value(params).expect("wire parameters must serialize"),
        })
    }
    pub fn success(id: RpcId, result: impl Serialize) -> Self {
        Self::Success(Success {
            jsonrpc: RpcVersion::V2,
            id,
            result: serde_json::to_value(result).expect("wire result must serialize"),
        })
    }
    pub fn failure(id: Option<RpcId>, error: RpcError) -> Self {
        Self::Failure(Failure {
            jsonrpc: RpcVersion::V2,
            id,
            error,
        })
    }
    pub fn parse(line: &str) -> Result<Self> {
        ensure!(line.len() <= MAX_MESSAGE_BYTES, "message exceeds 1 MiB");
        let message: Self = serde_json::from_str(line)?;
        match &message {
            Self::Request(r) => ensure!(
                !r.method.is_empty() && (r.params.is_object() || r.params.is_array()),
                "invalid request"
            ),
            Self::Notification(n) => ensure!(
                !n.method.is_empty() && (n.params.is_object() || n.params.is_array()),
                "invalid notification"
            ),
            _ => {}
        }
        Ok(message)
    }
    pub fn encode(&self) -> Result<String> {
        let line = serde_json::to_string(self)?;
        ensure!(line.len() <= MAX_MESSAGE_BYTES, "message exceeds 1 MiB");
        Ok(line)
    }
}
impl From<u64> for RpcId {
    fn from(value: u64) -> Self {
        Self::Number(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitializeParams {
    pub protocol_versions: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitializeResult {
    pub protocol_version: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewGet {
    pub view_id: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionInvoke {
    pub action: String,
    pub arguments: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatePublish {
    pub source: String,
    pub value: Value,
}

/// A deliberately small, closed schema vocabulary. Every object property is required.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataType {
    String,
    Number,
    Integer,
    Boolean,
    Array {
        items: Box<DataType>,
    },
    Object {
        properties: BTreeMap<String, DataType>,
    },
}
impl DataType {
    pub fn validate(&self, value: &Value) -> Result<()> {
        self.validate_at(value, "$", 0)
    }
    fn validate_at(&self, value: &Value, path: &str, depth: usize) -> Result<()> {
        ensure!(depth <= 32, "schema exceeds maximum depth");
        match self {
            Self::String => ensure!(value.is_string(), "{path}: expected string"),
            Self::Number => ensure!(value.is_number(), "{path}: expected number"),
            Self::Integer => ensure!(value.is_i64() || value.is_u64(), "{path}: expected integer"),
            Self::Boolean => ensure!(value.is_boolean(), "{path}: expected boolean"),
            Self::Array { items } => {
                let array = value
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("{path}: expected array"))?;
                for (i, item) in array.iter().enumerate() {
                    items.validate_at(item, &format!("{path}[{i}]"), depth + 1)?;
                }
            }
            Self::Object { properties } => {
                let object = value
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("{path}: expected object"))?;
                ensure!(
                    object.len() == properties.len(),
                    "{path}: unexpected or missing properties"
                );
                for (key, schema) in properties {
                    schema.validate_at(
                        object
                            .get(key)
                            .ok_or_else(|| anyhow::anyhow!("{path}: missing {key}"))?,
                        &format!("{path}.{key}"),
                        depth + 1,
                    )?;
                }
            }
        }
        Ok(())
    }
    fn at_path(&self, path: &[String]) -> Result<&Self> {
        let mut current = self;
        for part in path {
            current = match current {
                Self::Object { properties } => properties
                    .get(part)
                    .ok_or_else(|| anyhow::anyhow!("unknown binding field {part}"))?,
                Self::Array { items } if part.parse::<usize>().is_ok() => items,
                _ => bail!("invalid binding path segment {part}"),
            };
        }
        Ok(current)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceDefinition {
    pub id: String,
    pub schema: DataType,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionDefinition {
    pub id: String,
    pub arguments: DataType,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleDescription {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sources: Vec<SourceDefinition>,
    pub actions: Vec<ActionDefinition>,
    pub view_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Property<T> {
    Literal { value: T },
    Binding { source: String, path: Vec<String> },
}
impl<T> Property<T> {
    pub fn literal(value: T) -> Self {
        Self::Literal { value }
    }
    pub fn binding(source: &str, path: &[&str]) -> Self {
        Self::Binding {
            source: source.into(),
            path: path.iter().map(|v| (*v).into()).collect(),
        }
    }
    fn validate(&self, description: &ModuleDescription, expected: DataType) -> Result<()> {
        if let Self::Binding { source, path } = self {
            let definition = description.source(source)?;
            let actual = definition.schema.at_path(path)?;
            ensure!(
                *actual == expected
                    || (expected == DataType::Number && *actual == DataType::Integer),
                "binding type mismatch for {source}/{}",
                path.join("/")
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Clear,
    Notice,
    Urgent,
    Emergency,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Widget {
    VStack {
        id: String,
        children: Vec<Widget>,
    },
    HStack {
        id: String,
        children: Vec<Widget>,
    },
    Card {
        id: String,
        title: String,
        children: Vec<Widget>,
    },
    Text {
        id: String,
        value: Property<String>,
    },
    Metric {
        id: String,
        label: String,
        value: Property<f64>,
        unit: Option<String>,
    },
    Status {
        id: String,
        label: String,
        value: Property<Status>,
    },
    Button {
        id: String,
        label: String,
        action: String,
        arguments: Value,
    },
    Divider {
        id: String,
    },
}
impl Widget {
    pub fn id(&self) -> &str {
        match self {
            Self::VStack { id, .. }
            | Self::HStack { id, .. }
            | Self::Card { id, .. }
            | Self::Text { id, .. }
            | Self::Metric { id, .. }
            | Self::Status { id, .. }
            | Self::Button { id, .. }
            | Self::Divider { id } => id,
        }
    }
    pub fn validate(&self, description: &ModuleDescription) -> Result<()> {
        let mut ids = BTreeSet::new();
        let mut pending = vec![(self, 0)];
        while let Some((widget, depth)) = pending.pop() {
            ensure!(
                depth <= 32 && ids.len() < 1000,
                "view exceeds complexity limits"
            );
            validate_id(widget.id())?;
            ensure!(
                ids.insert(widget.id()),
                "duplicate widget ID {}",
                widget.id()
            );
            match widget {
                Self::VStack { children, .. }
                | Self::HStack { children, .. }
                | Self::Card { children, .. } => {
                    pending.extend(children.iter().map(|child| (child, depth + 1)));
                }
                Self::Text { value, .. } => value.validate(description, DataType::String)?,
                Self::Metric { value, .. } => value.validate(description, DataType::Number)?,
                Self::Status { value, .. } => value.validate(description, DataType::String)?,
                Self::Button {
                    action, arguments, ..
                } => description.action(action)?.arguments.validate(arguments)?,
                Self::Divider { .. } => {}
            }
        }
        Ok(())
    }
}
fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        "invalid stable ID {id:?}"
    );
    Ok(())
}
impl ModuleDescription {
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id)?;
        validate_id(&self.view_id)?;
        ensure!(!self.name.is_empty(), "module name must not be empty");
        for ids in [
            self.sources
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            self.actions.iter().map(|a| a.id.as_str()).collect(),
        ] {
            let mut seen = BTreeSet::new();
            for id in ids {
                validate_id(id)?;
                ensure!(seen.insert(id), "duplicate declaration ID {id}");
            }
        }
        Ok(())
    }
    pub fn source(&self, id: &str) -> Result<&SourceDefinition> {
        self.sources
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow::anyhow!("unknown source {id}"))
    }
    pub fn action(&self, id: &str) -> Result<&ActionDefinition> {
        self.actions
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| anyhow::anyhow!("unknown action {id}"))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModuleHealth {
    Starting,
    Ready,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Available,
    Stale,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceState {
    pub value: Value,
    pub status: SourceStatus,
    pub revision: u64,
    pub updated_at_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub protocol_version: u32,
    pub module: Option<ModuleDescription>,
    pub view: Option<Widget>,
    pub sources: BTreeMap<String, SourceState>,
    pub health: ModuleHealth,
    pub diagnostic: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            module: None,
            view: None,
            sources: BTreeMap::new(),
            health: ModuleHealth::Starting,
            diagnostic: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StateChanged {
    pub sources: BTreeMap<String, SourceState>,
    pub health: ModuleHealth,
    pub diagnostic: Option<String>,
}
impl From<&Snapshot> for StateChanged {
    fn from(snapshot: &Snapshot) -> Self {
        Self {
            sources: snapshot.sources.clone(),
            health: snapshot.health.clone(),
            diagnostic: snapshot.diagnostic.clone(),
        }
    }
}

/// Schema catalog for all version 1 envelopes and method payloads.
#[derive(JsonSchema)]
pub struct ProtocolCatalog {
    pub message: Message,
    pub initialize_params: InitializeParams,
    pub initialize_result: InitializeResult,
    pub empty_params: EmptyParams,
    pub module_description: ModuleDescription,
    pub view_get: ViewGet,
    pub view: Widget,
    pub action_invoke: ActionInvoke,
    pub state_publish: StatePublish,
    pub snapshot: Snapshot,
    pub state_changed: StateChanged,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_envelopes() {
        for line in [
            r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"no"}}"#,
            r#"{"jsonrpc":"2.0","method":"x","params":5}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            "[]",
        ] {
            assert!(Message::parse(line).is_err(), "{line}");
        }
        assert!(Message::parse(&"x".repeat(MAX_MESSAGE_BYTES + 1)).is_err());
    }
    #[test]
    fn accepts_string_ids_and_null_results() {
        let message = Message::success(RpcId::String("action-1".into()), Value::Null);
        assert_eq!(Message::parse(&message.encode().unwrap()).unwrap(), message);
        assert!(Message::parse(r#"{"jsonrpc":"2.0","method":"x"}"#).is_ok());
    }
    #[test]
    fn validates_structured_data_and_binding_types() {
        let schema = DataType::Object {
            properties: BTreeMap::from([("count".into(), DataType::Integer)]),
        };
        assert!(schema.validate(&json!({"count":3})).is_ok());
        for value in [
            json!({"count":"3"}),
            json!({}),
            json!({"count":3,"extra":true}),
        ] {
            assert!(schema.validate(&value).is_err());
        }
        let description = ModuleDescription {
            id: "demo".into(),
            name: "Demo".into(),
            description: String::new(),
            sources: vec![SourceDefinition {
                id: "overview".into(),
                schema,
            }],
            actions: vec![],
            view_id: "main".into(),
        };
        assert!(
            Property::<f64>::binding("overview", &["count"])
                .validate(&description, DataType::Number)
                .is_ok()
        );
        assert!(
            Property::<String>::binding("overview", &["count"])
                .validate(&description, DataType::String)
                .is_err()
        );
        assert!(
            Property::<f64>::binding("overview", &["missing"])
                .validate(&description, DataType::Number)
                .is_err()
        );
        let duplicate = Widget::VStack {
            id: "same".into(),
            children: vec![Widget::Divider { id: "same".into() }],
        };
        assert!(duplicate.validate(&description).is_err());
    }
    #[test]
    fn committed_schema_matches_wire_types() {
        let generated = serde_json::to_value(schemars::schema_for!(ProtocolCatalog)).unwrap();
        let committed: Value =
            serde_json::from_str(include_str!("../../../protocol/v1.schema.json")).unwrap();
        assert_eq!(
            generated, committed,
            "run cargo run -p spiritglass-protocol --bin export-schema"
        );
    }

    #[test]
    fn shared_swift_fixture_is_a_valid_snapshot() {
        let snapshot: Snapshot = serde_json::from_str(include_str!(
            "../../../protocol/fixtures/demo.snapshot.json"
        ))
        .unwrap();
        let description = snapshot.module.unwrap();
        description.validate().unwrap();
        snapshot.view.unwrap().validate(&description).unwrap();
        for (id, source) in snapshot.sources {
            description
                .source(&id)
                .unwrap()
                .schema
                .validate(&source.value)
                .unwrap();
        }
    }
}
