//! Pinned 0.153.2 app-server protocol subset; never a model/tool execution loop.
use forge_provider_common::{
    SecretBytes,
    adapter::{
        AdapterError, RuntimeActivityPhase, RuntimeFailureKind, RuntimeObservation,
        RuntimeToolKind, RuntimeUsage,
    },
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

pub fn initialize() -> Value {
    json!({"id":"forge-initialize","method":"initialize","params":{"clientInfo":{"name":"forge","title":"Forge managed runtime","version":"0.1.0"},"capabilities":{"experimentalApi":false}}})
}
pub fn start_thread(model: &str, cwd: &str) -> Value {
    json!({"id":"forge-thread","method":"thread/start","params":{"model":model,"modelProvider":"openai","cwd":cwd,"ephemeral":true,"approvalPolicy":"never","sandbox":"danger-full-access"}})
}
pub fn start_turn(
    input: Uuid,
    thread: &str,
    text: &SecretBytes,
) -> Result<SecretBytes, AdapterError> {
    let text = std::str::from_utf8(text.expose()).map_err(|_| AdapterError::InvalidEvent)?;
    if input.get_version_num() != 7 || text.is_empty() || text.len() > 1024 * 1024 {
        return Err(AdapterError::InvalidEvent);
    }
    #[derive(serde::Serialize)]
    struct Request<'a> {
        id: Uuid,
        method: &'static str,
        params: Params<'a>,
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Params<'a> {
        thread_id: &'a str,
        client_user_message_id: Uuid,
        input: [Part<'a>; 1],
    }
    #[derive(serde::Serialize)]
    struct Part<'a> {
        r#type: &'static str,
        text: &'a str,
    }
    serde_json::to_vec(&Request {
        id: input,
        method: "turn/start",
        params: Params {
            thread_id: thread,
            client_user_message_id: input,
            input: [Part {
                r#type: "text",
                text,
            }],
        },
    })
    .map(SecretBytes::new)
    .map_err(|_| AdapterError::InvalidEvent)
}
pub fn validate_thread(value: &Value, model: &str, cwd: &str) -> Result<String, AdapterError> {
    let result = value.get("result").ok_or(AdapterError::InvalidEvent)?;
    let thread = result.get("thread").ok_or(AdapterError::InvalidEvent)?;
    let id = id(thread.get("id"))?;
    if thread.get("sessionId").and_then(Value::as_str) != Some(id.as_str())
        || thread.get("ephemeral").and_then(Value::as_bool) != Some(true)
        || thread.get("cliVersion").and_then(Value::as_str) != Some(crate::PINNED_CODEX_VERSION)
        || result.get("model").and_then(Value::as_str) != Some(model)
        || result.get("modelProvider").and_then(Value::as_str) != Some("openai")
        || result.get("cwd").and_then(Value::as_str) != Some(cwd)
        || result.get("approvalPolicy").and_then(Value::as_str) != Some("never")
        || result.pointer("/sandbox/type").and_then(Value::as_str) != Some("dangerFullAccess")
        || result
            .get("instructionSources")
            .is_some_and(|value| value.as_array().is_none_or(|items| !items.is_empty()))
    {
        return Err(AdapterError::ProfileMismatch);
    }
    Ok(id)
}
pub fn accepted_turn(value: &Value, input: Uuid) -> Result<String, AdapterError> {
    if value.get("id").and_then(Value::as_str) != Some(input.to_string().as_str())
        || value.get("error").is_some()
    {
        return Err(AdapterError::InvalidEvent);
    }
    let turn = value
        .pointer("/result/turn")
        .ok_or(AdapterError::InvalidEvent)?;
    if turn.get("status").and_then(Value::as_str) != Some("inProgress")
        || turn.get("error").is_some_and(|error| !error.is_null())
    {
        return Err(AdapterError::InvalidEvent);
    }
    id(turn.get("id"))
}
fn id(value: Option<&Value>) -> Result<String, AdapterError> {
    value
        .and_then(Value::as_str)
        .filter(|id| {
            Uuid::parse_str(id)
                .ok()
                .is_some_and(|id| id.get_version_num() == 7)
        })
        .map(str::to_owned)
        .ok_or(AdapterError::InvalidEvent)
}

pub struct AppServerTurn {
    thread: String,
    turn: String,
    seen_items: BTreeSet<String>,
    usage: Option<RuntimeUsage>,
    finished: bool,
}
pub struct AppServerEvent {
    pub observations: Vec<RuntimeObservation>,
    pub finished: Option<bool>,
}
impl AppServerTurn {
    pub fn new(thread: String, turn: String) -> Self {
        Self {
            thread,
            turn,
            seen_items: BTreeSet::new(),
            usage: None,
            finished: false,
        }
    }
    pub fn usage(&self) -> Option<RuntimeUsage> {
        self.usage.clone()
    }
    pub fn ingest(&mut self, value: &Value) -> Result<AppServerEvent, AdapterError> {
        let mut event = AppServerEvent {
            observations: Vec::new(),
            finished: None,
        };
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return Err(AdapterError::InvalidEvent);
        };
        // Unexpected server requests must not hang unattended waiting for approval.
        if value.get("id").is_some() {
            return Err(AdapterError::InvalidEvent);
        }
        let params = value.get("params").unwrap_or(&Value::Null);
        if params
            .get("threadId")
            .is_some_and(|id| id.as_str() != Some(&self.thread))
            || params
                .get("turnId")
                .is_some_and(|id| id.as_str() != Some(&self.turn))
        {
            return Err(AdapterError::InvalidEvent);
        }
        match method {
            "thread/tokenUsage/updated" => {
                exact(params, &self.thread, &self.turn)?;
                self.usage = Some(parse_usage(
                    params
                        .pointer("/tokenUsage/total")
                        .ok_or(AdapterError::InvalidUsage)?,
                )?);
            }
            "turn/completed" => {
                if self.finished
                    || params.get("threadId").and_then(Value::as_str) != Some(&self.thread)
                    || params.pointer("/turn/id").and_then(Value::as_str) != Some(&self.turn)
                {
                    return Err(AdapterError::InvalidEvent);
                }
                self.finished = true;
                let completed = match params.pointer("/turn/status").and_then(Value::as_str) {
                    Some("completed") => true,
                    Some("failed" | "interrupted") => false,
                    _ => return Err(AdapterError::InvalidEvent),
                };
                if !completed {
                    event.observations.push(RuntimeObservation::Failure {
                        kind: failure(params.pointer("/turn/error")),
                    });
                }
                event.finished = Some(completed);
            }
            "item/completed" => {
                exact(params, &self.thread, &self.turn)?;
                let item = params.get("item").ok_or(AdapterError::InvalidEvent)?;
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .ok_or(AdapterError::InvalidEvent)?;
                if self.seen_items.len() >= 16384 {
                    return Err(AdapterError::InvalidEvent);
                }
                if !self.seen_items.insert(id.into()) {
                    return Ok(event);
                }
                match item.get("type").and_then(Value::as_str) {
                    Some("agentMessage") => {
                        let text = item
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or(AdapterError::InvalidEvent)?;
                        event
                            .observations
                            .push(RuntimeObservation::AssistantOutput {
                                text: SecretBytes::new(text.as_bytes().to_vec()),
                            });
                    }
                    Some("commandExecution" | "fileChange" | "mcpToolCall") => {
                        let kind = match item.get("type").and_then(Value::as_str) {
                            Some("commandExecution") => RuntimeToolKind::Shell,
                            Some("fileChange") => RuntimeToolKind::FileChange,
                            _ => RuntimeToolKind::Mcp,
                        };
                        let succeeded = item
                            .get("status")
                            .and_then(Value::as_str)
                            .map(|status| status == "completed");
                        event.observations.push(RuntimeObservation::ToolActivity {
                            kind,
                            phase: RuntimeActivityPhase::Completed,
                            succeeded,
                        });
                    }
                    _ => {}
                }
            }
            "error" => event.observations.push(RuntimeObservation::Failure {
                kind: failure(params.get("error")),
            }),
            // Reasoning/deltas/raw tool payloads never enter normalized evidence.
            _ => {}
        }
        Ok(event)
    }
}
fn exact(params: &Value, thread: &str, turn: &str) -> Result<(), AdapterError> {
    if params.get("threadId").and_then(Value::as_str) != Some(thread)
        || params.get("turnId").and_then(Value::as_str) != Some(turn)
    {
        return Err(AdapterError::InvalidEvent);
    }
    Ok(())
}
fn parse_usage(value: &Value) -> Result<RuntimeUsage, AdapterError> {
    let get = |key| {
        value
            .get(key)
            .and_then(Value::as_u64)
            .ok_or(AdapterError::InvalidUsage)
    };
    let usage = RuntimeUsage {
        input_tokens: get("inputTokens")?,
        output_tokens: get("outputTokens")?,
        cached_input_tokens: Some(get("cachedInputTokens")?),
        cache_write_input_tokens: Some(
            value.get("cacheWriteInputTokens").map_or(Ok(0), |value| {
                value.as_u64().ok_or(AdapterError::InvalidUsage)
            })?,
        ),
        reasoning_output_tokens: Some(get("reasoningOutputTokens")?),
    };
    if usage
        .cached_input_tokens
        .is_some_and(|n| n > usage.input_tokens)
        || usage
            .cache_write_input_tokens
            .is_some_and(|n| n > usage.input_tokens)
        || usage
            .reasoning_output_tokens
            .is_some_and(|n| n > usage.output_tokens)
        || usage.input_tokens.checked_add(usage.output_tokens) != Some(get("totalTokens")?)
    {
        return Err(AdapterError::InvalidUsage);
    }
    Ok(usage)
}
pub fn request_failure(value: &Value) -> Option<RuntimeFailureKind> {
    value.get("error").map(|error| failure(Some(error)))
}
fn failure(value: Option<&Value>) -> RuntimeFailureKind {
    match value
        .and_then(|value| value.get("codexErrorInfo"))
        .and_then(Value::as_str)
    {
        Some("usageLimitExceeded" | "rateLimitExceeded") => RuntimeFailureKind::RateLimited,
        Some("unauthorized") => RuntimeFailureKind::AuthRequired,
        Some("serverOverloaded") => RuntimeFailureKind::ProviderUnavailable,
        _ => value
            .and_then(|value| value.get("message"))
            .and_then(Value::as_str)
            .map(crate::events::classify_message)
            .unwrap_or(RuntimeFailureKind::RuntimeError),
    }
}
