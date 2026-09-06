use forge_provider_common::{
    SecretBytes,
    adapter::{
        AdapterError, RuntimeActivityPhase, RuntimeFailureKind, RuntimeObservation,
        RuntimeToolKind, RuntimeUsage,
    },
};
use serde::Deserialize;
use serde_json::value::RawValue;

const MAX_EVENT_BYTES: usize = 1024 * 1024;

/// Parses the pinned JSONL protocol without promoting text/exit into an outcome.
/// Raw reasoning bodies, arguments, shell output, and error text are never copied
/// into normalized events. Caller must redact any separately retained raw logs.
pub fn parse_jsonl_event(line: &str) -> Result<RuntimeObservation, AdapterError> {
    if line.len() > MAX_EVENT_BYTES {
        return Err(AdapterError::InvalidEvent);
    }
    let event: Event<'_> = serde_json::from_str(line).map_err(|_| AdapterError::InvalidEvent)?;
    match event.kind {
        "thread.started" => Ok(RuntimeObservation::ThreadStarted),
        "turn.started" => Ok(RuntimeObservation::TurnStarted),
        "turn.completed" => Ok(RuntimeObservation::TurnCompleted {
            usage: event.usage.map(Usage::validate).transpose()?,
        }),
        "turn.failed" => failure(event.error.and_then(|error| error.message)),
        "error" => failure(event.message),
        "item.started" | "item.updated" | "item.completed" => {
            let item = event.item.ok_or(AdapterError::InvalidEvent)?;
            let header: ItemHeader<'_> =
                serde_json::from_str(item.get()).map_err(|_| AdapterError::InvalidEvent)?;
            if header.kind == "reasoning" {
                return Ok(RuntimeObservation::Ignored);
            }
            parse_item(event.kind, header.kind, item)
        }
        _ => Ok(RuntimeObservation::Ignored),
    }
}

fn parse_item(event: &str, kind: &str, raw: &RawValue) -> Result<RuntimeObservation, AdapterError> {
    let phase = match event {
        "item.started" => RuntimeActivityPhase::Started,
        "item.updated" => RuntimeActivityPhase::Updated,
        _ => RuntimeActivityPhase::Completed,
    };
    if kind == "agent_message" {
        let item: TextItem =
            serde_json::from_str(raw.get()).map_err(|_| AdapterError::InvalidEvent)?;
        return Ok(RuntimeObservation::AssistantOutput { text: item.text.0 });
    }
    if kind == "error" {
        let error: ErrorBody =
            serde_json::from_str(raw.get()).map_err(|_| AdapterError::InvalidEvent)?;
        return failure(error.message);
    }
    let kind = match kind {
        "command_execution" => RuntimeToolKind::Shell,
        "file_change" => RuntimeToolKind::FileChange,
        "mcp_tool_call" => RuntimeToolKind::Mcp,
        "web_search" => RuntimeToolKind::WebSearch,
        "todo_list" => RuntimeToolKind::Plan,
        "collab_tool_call" => RuntimeToolKind::Collaboration,
        _ => return Ok(RuntimeObservation::Ignored),
    };
    let status: Status<'_> =
        serde_json::from_str(raw.get()).map_err(|_| AdapterError::InvalidEvent)?;
    let succeeded = match status.status {
        Some("completed") => Some(true),
        Some("failed" | "declined") => Some(false),
        Some("in_progress") | None => None,
        Some(_) => return Err(AdapterError::InvalidEvent),
    };
    Ok(RuntimeObservation::ToolActivity {
        kind,
        phase,
        succeeded,
    })
}

fn failure(message: Option<SensitiveText>) -> Result<RuntimeObservation, AdapterError> {
    let message = message.ok_or(AdapterError::InvalidEvent)?;
    let message =
        std::str::from_utf8(message.0.expose()).map_err(|_| AdapterError::InvalidEvent)?;
    let contains = |needle: &str| {
        message
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
    };
    // Classification observes fixed indicators only. Never return the provider's
    // message: it may contain a token, a prompt, or an upstream URL query string.
    let kind =
        if contains("refresh_token_reused") || contains("refresh token has already been used") {
            RuntimeFailureKind::AuthRefreshReused
        } else if contains("refresh_token_expired") {
            RuntimeFailureKind::AuthExpired
        } else if contains("refresh_token_invalidated") {
            RuntimeFailureKind::AuthInvalidated
        } else if contains("401") || contains("not logged in") || contains("authentication") {
            RuntimeFailureKind::AuthRequired
        } else if contains("429")
            || contains("rate limit")
            || contains("usage limit")
            || contains("quota")
        {
            RuntimeFailureKind::RateLimited
        } else if contains("503")
            || contains("502")
            || contains("connection")
            || contains("temporarily unavailable")
        {
            RuntimeFailureKind::ProviderUnavailable
        } else {
            RuntimeFailureKind::RuntimeError
        };
    Ok(RuntimeObservation::Failure { kind })
}

#[derive(Deserialize)]
struct Event<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(borrow)]
    item: Option<&'a RawValue>,
    usage: Option<Usage>,
    error: Option<ErrorBody>,
    message: Option<SensitiveText>,
}

#[derive(Deserialize)]
struct ItemHeader<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}
#[derive(Deserialize)]
struct TextItem {
    text: SensitiveText,
}
#[derive(Deserialize)]
struct ErrorBody {
    message: Option<SensitiveText>,
}
#[derive(Deserialize)]
struct Status<'a> {
    status: Option<&'a str>,
}

struct SensitiveText(SecretBytes);

impl<'de> Deserialize<'de> for SensitiveText {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self(SecretBytes::new(text.into_bytes())))
    }
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: Option<u64>,
    cache_write_input_tokens: Option<u64>,
    reasoning_output_tokens: Option<u64>,
}

impl Usage {
    fn validate(self) -> Result<RuntimeUsage, AdapterError> {
        if self
            .cached_input_tokens
            .is_some_and(|cached| cached > self.input_tokens)
            || self
                .reasoning_output_tokens
                .is_some_and(|reasoning| reasoning > self.output_tokens)
        {
            return Err(AdapterError::InvalidUsage);
        }
        Ok(RuntimeUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cached_input_tokens: self.cached_input_tokens,
            cache_write_input_tokens: self.cache_write_input_tokens,
            reasoning_output_tokens: self.reasoning_output_tokens,
        })
    }
}
