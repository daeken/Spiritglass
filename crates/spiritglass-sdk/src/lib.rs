//! Small stdio host for trusted, independently built modules.
use anyhow::{Result, ensure};
use futures_util::{SinkExt, StreamExt};
use protocol::*;
use serde_json::{Value, json};
pub use spiritglass_protocol as protocol;
use std::time::Duration;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

pub trait Module {
    fn describe(&self) -> ModuleDescription;
    fn view(&self) -> Widget;
    fn state(&self) -> Vec<StatePublish>;
    fn invoke(&mut self, action: &str, arguments: Value) -> Result<Value, RpcError>;
    fn tick(&mut self) {}
    fn tick_interval(&self) -> Duration {
        Duration::from_secs(1)
    }
}

pub async fn run(mut module: impl Module) -> Result<()> {
    let description = module.describe();
    description.validate()?;
    let view = module.view();
    view.validate(&description)?;
    ensure!(
        !module.tick_interval().is_zero(),
        "tick interval must be positive"
    );
    let mut input = FramedRead::new(
        tokio::io::stdin(),
        LinesCodec::new_with_max_length(MAX_MESSAGE_BYTES),
    );
    let mut output = FramedWrite::new(
        tokio::io::stdout(),
        LinesCodec::new_with_max_length(MAX_MESSAGE_BYTES),
    );
    let mut timer = tokio::time::interval(module.tick_interval());
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut initialized = false;
    let mut streaming = false;
    loop {
        tokio::select! {
            line = input.next() => {
                let Some(line) = line else { return Ok(()); };
                let line = line?;
                let message = match Message::parse(&line) {
                    Ok(message) => message,
                    Err(error) => {
                        let code = if serde_json::from_str::<Value>(&line).is_err() { -32700 } else { -32600 };
                        let failure = Message::failure(None, RpcError::new(code, format!("invalid message: {error}")));
                        output.send(failure.encode()?).await?;
                        continue;
                    }
                };
                let Message::Request(request) = message else { continue; };
                let mut start_stream = false;
                let mut shutdown = false;
                let result: Result<Value, RpcError> = (|| {
                    if request.method == "initialize" {
                        if initialized { return Err(RpcError::new(-32002, "already initialized")); }
                        let params: InitializeParams = serde_json::from_value(request.params.clone()).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                        if !params.protocol_versions.contains(&PROTOCOL_VERSION) { return Err(RpcError::new(-32001, "no compatible protocol version")); }
                        initialized = true;
                        return Ok(json!(InitializeResult { protocol_version: PROTOCOL_VERSION }));
                    }
                    if !initialized { return Err(RpcError::new(-32002, "initialize first")); }
                    match request.method.as_str() {
                        "module.describe" => { parse_empty(request.params.clone())?; Ok(json!(description)) },
                        "view.get" => {
                            let params: ViewGet = serde_json::from_value(request.params.clone()).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                            if params.view_id != description.view_id { return Err(RpcError::invalid_params("unknown view")); }
                            start_stream = true;
                            Ok(json!(view))
                        },
                        "action.invoke" => {
                            if !streaming { return Err(RpcError::new(-32002, "retrieve view first")); }
                            let params: ActionInvoke = serde_json::from_value(request.params.clone()).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                            let action = description.action(&params.action).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                            action.arguments.validate(&params.arguments).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                            module.invoke(&params.action, params.arguments)
                        },
                        "shutdown" => { parse_empty(request.params.clone())?; shutdown = true; Ok(Value::Null) },
                        _ => Err(RpcError::new(-32601, "method not found")),
                    }
                })();
                let succeeded = result.is_ok();
                // Actions publish before their response, so successful completion observes current state.
                if succeeded && request.method == "action.invoke" {
                    publish(&module, &description, &mut output).await?;
                }
                let response = match result { Ok(value) => Message::success(request.id, value), Err(error) => Message::failure(Some(request.id), error) };
                output.send(response.encode()?).await?;
                if shutdown { return Ok(()); }
                if start_stream {
                    streaming = true;
                    publish(&module, &description, &mut output).await?;
                }
            },
            _ = timer.tick(), if streaming => {
                module.tick();
                publish(&module, &description, &mut output).await?;
            }
        }
    }
}

fn parse_empty(value: Value) -> Result<(), RpcError> {
    serde_json::from_value::<EmptyParams>(value)
        .map(|_| ())
        .map_err(|e| RpcError::invalid_params(e.to_string()))
}
async fn publish(
    module: &impl Module,
    description: &ModuleDescription,
    output: &mut FramedWrite<tokio::io::Stdout, LinesCodec>,
) -> Result<()> {
    for update in module.state() {
        description
            .source(&update.source)?
            .schema
            .validate(&update.value)?;
        output
            .send(Message::notification("state.publish", update).encode()?)
            .await?;
    }
    Ok(())
}
