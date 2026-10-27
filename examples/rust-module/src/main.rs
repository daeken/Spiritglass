use serde_json::{Value, json};
use spiritglass_protocol::*;
use spiritglass_sdk::Module;
use std::{collections::BTreeMap, time::Instant};

struct Demo {
    started: Instant,
    count: u64,
}
impl Module for Demo {
    fn describe(&self) -> ModuleDescription {
        ModuleDescription {
            id: "dev.spiritglass.example".into(),
            name: "Hello, Spiritglass".into(),
            description: "A small window into a live, independent Rust process.".into(),
            view_id: "main".into(),
            sources: vec![SourceDefinition {
                id: "overview".into(),
                schema: DataType::Object {
                    properties: BTreeMap::from([
                        ("uptime".into(), DataType::Integer),
                        ("count".into(), DataType::Integer),
                        ("status".into(), DataType::String),
                        ("message".into(), DataType::String),
                    ]),
                },
            }],
            actions: ["increment", "reset"]
                .iter()
                .map(|id| ActionDefinition {
                    id: (*id).into(),
                    arguments: DataType::Object {
                        properties: BTreeMap::new(),
                    },
                })
                .collect(),
        }
    }
    fn view(&self) -> Widget {
        Widget::VStack { id: "dashboard".into(), children: vec![
            Widget::Text { id: "intro".into(), value: Property::literal("This dashboard comes from the module. Every control is rendered natively.".into()) },
            Widget::HStack { id: "metrics".into(), children: vec![
                Widget::Metric { id: "uptime".into(), label: "Time together".into(), value: Property::binding("overview", &["uptime"]), unit: Some("seconds".into()) },
                Widget::Metric { id: "count".into(), label: "Actions received".into(), value: Property::binding("overview", &["count"]), unit: None },
            ] },
            Widget::Card { id: "playground".into(), title: "A conversation with your module".into(), children: vec![
                Widget::Status { id: "status".into(), label: "Example status".into(), value: Property::binding("overview", &["status"]) },
                Widget::Text { id: "message".into(), value: Property::binding("overview", &["message"]) },
                Widget::HStack { id: "controls".into(), children: vec![
                    Widget::Button { id: "increment".into(), label: "Send a little signal".into(), action: "increment".into(), arguments: json!({}) },
                    Widget::Button { id: "reset".into(), label: "Reset".into(), action: "reset".into(), arguments: json!({}) },
                ] },
            ] },
            Widget::Text { id: "hint".into(), value: Property::literal("Send five signals to see the example status change. Close and reopen the window: the supervisor keeps the module alive.".into()) },
        ] }
    }
    fn state(&self) -> Vec<StatePublish> {
        vec![StatePublish {
            source: "overview".into(),
            value: json!({
                "uptime": self.started.elapsed().as_secs(), "count": self.count,
                "status": if self.count >= 5 { "notice" } else { "clear" },
                "message": if self.count == 0 { "Ready when you are.".to_owned() } else { format!("Signal received. The module has handled {} action{}.", self.count, if self.count == 1 { "" } else { "s" }) },
            }),
        }]
    }
    fn invoke(&mut self, action: &str, _arguments: Value) -> Result<Value, RpcError> {
        match action {
            "increment" => self.count = self.count.saturating_add(1),
            "reset" => self.count = 0,
            _ => return Err(RpcError::new(-32601, "unknown action")),
        }
        Ok(json!({"count": self.count}))
    }
}
#[tokio::main]
async fn main() {
    if let Err(error) = spiritglass_sdk::run(Demo {
        started: Instant::now(),
        count: 0,
    })
    .await
    {
        eprintln!("example module: {error:#}");
        std::process::exit(1);
    }
}
