use forge_provider_common::{
    SecretBytes,
    adapter::{
        AdapterError, RuntimeActivityPhase, RuntimeFailureKind, RuntimeObservation,
        RuntimeToolKind, RuntimeUsage,
    },
};
use serde::Deserialize;
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::MAX_EVENT_BYTES;

/// Normalized observations and bounded protocol correlation. No raw tool body,
/// reasoning, provider error text or echoed prompt survives this boundary.
#[derive(Debug)]
pub struct ClaudeEvent {
    /// Claimed session identity; the driver must match it against the active Run.
    pub session_id: Option<Uuid>,
    /// Runtime event identity, useful for duplicate detection where present.
    pub event_id: Option<Uuid>,
    /// Replayed user input identity, not confirmation of Employee comprehension.
    /// The driver must match an outstanding message in this exact session.
    pub replayed_message_id: Option<Uuid>,
    /// End of one query loop, not physical process exit or Task completion.
    pub turn_end: Option<ClaudeTurnEnd>,
    /// Usage reported at any terminal outcome, including failure/interruption.
    /// Successful turns also expose this through `TurnCompleted`; count it once.
    pub usage: Option<RuntimeUsage>,
    /// A stream message can contain both visible text and tool activity.
    pub observations: Vec<RuntimeObservation>,
}

/// Provider-reported end of a turn; an interrupted stream must not look complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeTurnEnd {
    /// The query loop finished without a provider error or interruption.
    Completed,
    /// Claude reports a cancelled model/tool loop; process stop is still separate.
    Interrupted,
    /// The query loop ended because of a limit or failure.
    Failed,
}

/// Parses one bounded Claude stream-json message. Provider completion remains
/// turn evidence; it never implies accepted work. Unknown events are ignored.
pub fn parse_jsonl_event(line: &str) -> Result<ClaudeEvent, AdapterError> {
    if line.len() > MAX_EVENT_BYTES {
        return Err(AdapterError::InvalidEvent);
    }
    let header: Header<'_> = decode(line)?;
    let mut parsed = ClaudeEvent {
        session_id: header.session_id,
        event_id: header.uuid,
        replayed_message_id: None,
        turn_end: None,
        usage: None,
        observations: Vec::new(),
    };
    if parsed.session_id.is_some_and(|id| id.is_nil())
        || parsed.event_id.is_some_and(|id| id.is_nil())
    {
        return Err(AdapterError::InvalidEvent);
    }
    match header.kind {
        "system" if header.subtype == Some("init") => {
            required_session(&parsed)?;
            parsed.observations.push(RuntimeObservation::ThreadStarted);
        }
        "assistant" => {
            required_session(&parsed)?;
            if header.error.is_some()
                || header.is_api_error_message
                || header.api_error_status.is_some_and(|status| status >= 400)
            {
                parsed.observations.push(failure(
                    header.error.unwrap_or("api_error"),
                    header.api_error_status,
                ));
            } else {
                parse_assistant(
                    header.message.ok_or(AdapterError::InvalidEvent)?,
                    &mut parsed.observations,
                )?;
            }
        }
        "user" => {
            required_session(&parsed)?;
            let message: UserMessage<'_> =
                decode(header.message.ok_or(AdapterError::InvalidEvent)?.get())?;
            // Tool-result user messages and subagent traffic are not stdin ACKs.
            if header.parent_tool_use_id.is_none()
                && message.role == "user"
                && is_text_content(message.content)?
                && header.origin.is_none_or(|origin| origin.kind == "human")
            {
                parsed.replayed_message_id = Some(header.uuid.ok_or(AdapterError::InvalidEvent)?);
            }
        }
        "result" => {
            required_session(&parsed)?;
            let result: ResultEvent<'_> = decode(line)?;
            parsed.usage = result.usage.map(parse_usage).transpose()?;
            if header.parent_tool_use_id.is_some()
                || header.origin.is_some_and(|origin| origin.kind != "human")
            {
                return Err(AdapterError::InvalidEvent);
            }
            if matches!(
                result.terminal_reason,
                Some("aborted_streaming" | "aborted_tools")
            ) {
                parsed.turn_end = Some(ClaudeTurnEnd::Interrupted);
            } else if result.is_error
                || result.subtype != "success"
                || result.api_error_status.is_some_and(|status| status >= 400)
                || result
                    .terminal_reason
                    .is_some_and(|reason| reason != "completed")
            {
                parsed.turn_end = Some(ClaudeTurnEnd::Failed);
                parsed
                    .observations
                    .push(failure(result.subtype, result.api_error_status));
            } else {
                parsed.turn_end = Some(ClaudeTurnEnd::Completed);
                parsed.observations.push(RuntimeObservation::TurnCompleted {
                    usage: parsed.usage.clone(),
                });
            }
        }
        "rate_limit_event" => {
            let rate: RateEvent<'_> = decode(line)?;
            if rate.rate_limit_info.status == "rejected" {
                required_session(&parsed)?;
                parsed.observations.push(RuntimeObservation::Failure {
                    kind: RuntimeFailureKind::RateLimited,
                });
            }
        }
        "stream_event" => {
            // Partial messages are deliberately not requested. Returning them as
            // text would duplicate the later complete assistant message.
        }
        // Conversation reset invalidates the session/budget accounting contract;
        // do not silently continue in an unpinned conversation.
        "conversation_reset" => return Err(AdapterError::InvalidEvent),
        _ => {}
    }
    if parsed.observations.is_empty() && parsed.replayed_message_id.is_none() {
        parsed.observations.push(RuntimeObservation::Ignored);
    }
    Ok(parsed)
}

fn parse_assistant(
    raw: &RawValue,
    observations: &mut Vec<RuntimeObservation>,
) -> Result<(), AdapterError> {
    let message: AssistantMessage<'_> = decode(raw.get())?;
    if message.role != "assistant" {
        return Err(AdapterError::InvalidEvent);
    }
    for raw in message.content {
        let block: Block<'_> = decode(raw.get())?;
        match block.kind {
            "text" => {
                let text: TextBlock = decode(raw.get())?;
                observations.push(RuntimeObservation::AssistantOutput { text: text.text.0 });
            }
            "tool_use" => {
                let tool: ToolBlock<'_> = decode(raw.get())?;
                let kind = match tool.name {
                    "Bash" => Some(RuntimeToolKind::Shell),
                    "Write" | "Edit" | "NotebookEdit" => Some(RuntimeToolKind::FileChange),
                    "TodoWrite" | "EnterPlanMode" | "ExitPlanMode" => Some(RuntimeToolKind::Plan),
                    "Task" | "Agent" => Some(RuntimeToolKind::Collaboration),
                    "WebSearch" | "WebFetch" => Some(RuntimeToolKind::WebSearch),
                    name if name.starts_with("mcp__forge__") => Some(RuntimeToolKind::Mcp),
                    _ => None,
                };
                if let Some(kind) = kind {
                    observations.push(RuntimeObservation::ToolActivity {
                        kind,
                        phase: RuntimeActivityPhase::Started,
                        succeeded: None,
                    });
                }
            }
            // Hidden thinking/redacted_thinking and future blocks are not output.
            _ => {}
        }
    }
    Ok(())
}

fn is_text_content(raw: &RawValue) -> Result<bool, AdapterError> {
    if raw.get().starts_with('"') {
        // The JSON parser already validated the string; do not copy the prompt.
        return Ok(true);
    }
    let blocks: Vec<&RawValue> = decode(raw.get())?;
    if blocks.is_empty() {
        return Ok(false);
    }
    for block in blocks {
        if decode::<Block<'_>>(block.get())?.kind != "text" {
            return Ok(false);
        }
    }
    Ok(true)
}

fn parse_usage(raw: &RawValue) -> Result<RuntimeUsage, AdapterError> {
    let usage: Usage = serde_json::from_str(raw.get()).map_err(|_| AdapterError::InvalidUsage)?;
    // Anthropic's input_tokens excludes cache reads and writes. Forge's input
    // counter includes them, unlike a direct relabeling of the vendor field.
    let input_tokens = usage
        .input_tokens
        .checked_add(usage.cache_read_input_tokens.unwrap_or(0))
        .and_then(|total| total.checked_add(usage.cache_creation_input_tokens.unwrap_or(0)))
        .ok_or(AdapterError::InvalidUsage)?;
    Ok(RuntimeUsage {
        input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cache_read_input_tokens,
        cache_write_input_tokens: usage.cache_creation_input_tokens,
        reasoning_output_tokens: None,
    })
}

fn failure(code: &str, status: Option<u16>) -> RuntimeObservation {
    let kind = match (code, status) {
        ("authentication_failed", _) | (_, Some(401)) => RuntimeFailureKind::AuthRequired,
        ("billing_error", _) | ("rate_limit", _) | (_, Some(429)) => {
            RuntimeFailureKind::RateLimited
        }
        ("server_error", _) | (_, Some(500..=599)) => RuntimeFailureKind::ProviderUnavailable,
        _ => RuntimeFailureKind::RuntimeError,
    };
    RuntimeObservation::Failure { kind }
}

fn required_session(event: &ClaudeEvent) -> Result<(), AdapterError> {
    event
        .session_id
        .ok_or(AdapterError::InvalidEvent)
        .map(|_| ())
}

fn decode<'a, T: Deserialize<'a>>(value: &'a str) -> Result<T, AdapterError> {
    serde_json::from_str(value).map_err(|_| AdapterError::InvalidEvent)
}

#[derive(Deserialize)]
struct Header<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    subtype: Option<&'a str>,
    session_id: Option<Uuid>,
    uuid: Option<Uuid>,
    #[serde(borrow)]
    message: Option<&'a RawValue>,
    parent_tool_use_id: Option<&'a RawValue>,
    error: Option<&'a str>,
    #[serde(default)]
    is_api_error_message: bool,
    api_error_status: Option<u16>,
    origin: Option<Origin<'a>>,
}
#[derive(Deserialize)]
struct Origin<'a> {
    kind: &'a str,
}
#[derive(Deserialize)]
struct AssistantMessage<'a> {
    role: &'a str,
    #[serde(borrow)]
    content: Vec<&'a RawValue>,
}
#[derive(Deserialize)]
struct UserMessage<'a> {
    role: &'a str,
    #[serde(borrow)]
    content: &'a RawValue,
}
#[derive(Deserialize)]
struct Block<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}
#[derive(Deserialize)]
struct ToolBlock<'a> {
    name: &'a str,
}
#[derive(Deserialize)]
struct TextBlock {
    text: SensitiveText,
}
struct SensitiveText(SecretBytes);
impl<'de> Deserialize<'de> for SensitiveText {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(SecretBytes::new(
            String::deserialize(deserializer)?.into_bytes(),
        )))
    }
}
#[derive(Deserialize)]
struct ResultEvent<'a> {
    subtype: &'a str,
    is_error: bool,
    api_error_status: Option<u16>,
    terminal_reason: Option<&'a str>,
    #[serde(borrow)]
    usage: Option<&'a RawValue>,
}
#[derive(Deserialize)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}
#[derive(Deserialize)]
struct RateEvent<'a> {
    #[serde(borrow)]
    rate_limit_info: RateInfo<'a>,
}
#[derive(Deserialize)]
struct RateInfo<'a> {
    status: &'a str,
}
