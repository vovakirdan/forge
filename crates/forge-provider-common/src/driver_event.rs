//! Provider-neutral allowlisted driver JSONL, not canonical Task outcomes.
use crate::{
    SecretBytes,
    adapter::{
        AdapterError, RuntimeActivityPhase, RuntimeFailureKind, RuntimeObservation,
        RuntimeToolKind, RuntimeUsage,
    },
};
use serde::Deserialize;
const MAX_EVENT_BYTES: usize = 1024 * 1024;
#[derive(Deserialize)]
struct Kind<'a> {
    #[serde(rename = "type", borrow)]
    kind: &'a str,
}
fn decode<'a, T: Deserialize<'a>>(data: &'a str) -> Result<T, AdapterError> {
    serde_json::from_str(data).map_err(|_| AdapterError::InvalidEvent)
}
struct SensitiveText(SecretBytes);
impl<'de> Deserialize<'de> for SensitiveText {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = SensitiveText;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("text")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(SensitiveText(SecretBytes::new(value.as_bytes().to_vec())))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(SensitiveText(SecretBytes::new(value.into_bytes())))
            }
        }
        deserializer.deserialize_string(Visitor)
    }
}

/// Small provider-neutral JSONL envelope, never a Task outcome or tool payload.
pub fn write_driver_event(event: &RuntimeObservation) -> Result<SecretBytes, AdapterError> {
    use serde_json::json;
    let value = match event {
        RuntimeObservation::ThreadStarted => json!({"type":"thread_started"}),
        RuntimeObservation::TurnStarted => json!({"type":"turn_started"}),
        RuntimeObservation::TurnCompleted { usage } => {
            json!({"type":"turn_completed","usage":usage})
        }
        RuntimeObservation::AssistantOutput { text } => {
            // Serialize a borrowed value directly so no unredacted owned String
            // survives in a JSON Value/Debug representation.
            #[derive(serde::Serialize)]
            struct Text<'a> {
                r#type: &'static str,
                text: &'a str,
            }
            let text =
                std::str::from_utf8(text.expose()).map_err(|_| AdapterError::InvalidEvent)?;
            return serde_json::to_vec(&Text {
                r#type: "assistant_output",
                text,
            })
            .map(SecretBytes::new)
            .map_err(|_| AdapterError::InvalidEvent);
        }
        RuntimeObservation::ToolActivity {
            kind,
            phase,
            succeeded,
        } => json!({"type":"tool_activity","kind":kind,"phase":phase,"succeeded":succeeded}),
        RuntimeObservation::Failure { kind } => json!({"type":"failure","kind":kind}),
        RuntimeObservation::Ignored => json!({"type":"ignored"}),
    };
    serde_json::to_vec(&value)
        .map(SecretBytes::new)
        .map_err(|_| AdapterError::InvalidEvent)
}

pub fn parse_driver_event(data: &str) -> Result<RuntimeObservation, AdapterError> {
    if data.len() > MAX_EVENT_BYTES {
        return Err(AdapterError::InvalidEvent);
    }
    let header: Kind<'_> = decode(data)?;
    #[derive(Deserialize)]
    struct Output {
        text: SensitiveText,
    }
    #[derive(Deserialize)]
    struct Completed {
        usage: Option<RuntimeUsage>,
    }
    #[derive(Deserialize)]
    struct Failure {
        kind: RuntimeFailureKind,
    }
    #[derive(Deserialize)]
    struct Tool {
        kind: RuntimeToolKind,
        phase: RuntimeActivityPhase,
        succeeded: Option<bool>,
    }
    Ok(match header.kind {
        "thread_started" => RuntimeObservation::ThreadStarted,
        "turn_started" => RuntimeObservation::TurnStarted,
        "turn_completed" => RuntimeObservation::TurnCompleted {
            usage: decode::<Completed>(data)?.usage,
        },
        "assistant_output" => RuntimeObservation::AssistantOutput {
            text: decode::<Output>(data)?.text.0,
        },
        "failure" => RuntimeObservation::Failure {
            kind: decode::<Failure>(data)?.kind,
        },
        "tool_activity" => {
            let tool: Tool = decode(data)?;
            RuntimeObservation::ToolActivity {
                kind: tool.kind,
                phase: tool.phase,
                succeeded: tool.succeeded,
            }
        }
        _ => RuntimeObservation::Ignored,
    })
}
