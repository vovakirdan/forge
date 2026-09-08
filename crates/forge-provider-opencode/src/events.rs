use std::collections::BTreeSet;

use forge_provider_common::{
    SecretBytes,
    adapter::{
        AdapterError, RuntimeActivityPhase, RuntimeFailureKind, RuntimeObservation,
        RuntimeToolKind, RuntimeUsage,
    },
};
use serde::Deserialize;
use serde_json::value::RawValue;

pub const MAX_EVENT_BYTES: usize = 1024 * 1024;
const MAX_TRACKED_ITEMS: usize = 16_384;

#[derive(Debug)]
pub enum SessionEvent {
    Observation(RuntimeObservation),
    Idle,
}

/// One interpreter belongs to one fresh session. It neither logs raw events nor
/// copies hidden reasoning, tool arguments, tool output, or provider error bodies.
pub struct EventInterpreter {
    session_id: String,
    busy: bool,
    assistants: BTreeSet<String>,
    completed: BTreeSet<String>,
    text_parts: BTreeSet<String>,
    usage: Option<RuntimeUsage>,
    missing_usage: bool,
    active_parent: Option<String>,
    completed_before_turn: usize,
}

impl EventInterpreter {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.into(),
            busy: false,
            assistants: BTreeSet::new(),
            completed: BTreeSet::new(),
            text_parts: BTreeSet::new(),
            usage: None,
            missing_usage: false,
            active_parent: None,
            completed_before_turn: 0,
        }
    }

    pub fn usage(&self) -> Option<RuntimeUsage> {
        if self.missing_usage {
            None
        } else {
            self.usage.clone()
        }
    }

    /// Native turns share usage/deduplication state, but not completion evidence.
    pub(crate) fn begin_native_turn(&mut self, user_message: &str) {
        self.active_parent = Some(user_message.into());
        self.completed_before_turn = self.completed.len();
        self.assistants.clear();
        self.busy = false;
    }

    pub fn ingest(&mut self, data: &str) -> Result<SessionEvent, AdapterError> {
        if data.len() > MAX_EVENT_BYTES {
            return Err(AdapterError::InvalidEvent);
        }
        let event: EventEnvelope<'_> = decode(data)?;
        let properties: Properties<'_> = decode(event.properties.get())?;
        if properties
            .session_id
            .is_some_and(|id| id != self.session_id)
        {
            return ignored();
        }
        match event.kind {
            "session.status" => {
                if properties.session_id != Some(&self.session_id) {
                    return ignored();
                }
                let status: Kind<'_> =
                    decode(properties.status.ok_or(AdapterError::InvalidEvent)?.get())?;
                match status.kind {
                    "busy" if !self.busy => {
                        self.busy = true;
                        observed(RuntimeObservation::TurnStarted)
                    }
                    "idle" => self.idle(),
                    _ => ignored(),
                }
            }
            "session.idle" if properties.session_id == Some(&self.session_id) => self.idle(),
            "session.error" if properties.session_id == Some(&self.session_id) => {
                observed(RuntimeObservation::Failure {
                    kind: failure(properties.error)?,
                })
            }
            "message.updated" => {
                self.message(properties.info.ok_or(AdapterError::InvalidEvent)?.get())
            }
            "message.part.updated" => {
                self.part(properties.part.ok_or(AdapterError::InvalidEvent)?.get())
            }
            // Deltas can contain reasoning with the same `field=text`. Only
            // allowlisted complete text parts are emitted, once per part ID.
            _ => ignored(),
        }
    }

    fn idle(&self) -> Result<SessionEvent, AdapterError> {
        if !self.busy {
            return ignored();
        }
        if self.completed.len() == self.completed_before_turn {
            return Err(AdapterError::InvalidEvent);
        }
        Ok(SessionEvent::Idle)
    }

    fn message(&mut self, data: &str) -> Result<SessionEvent, AdapterError> {
        let message: Message<'_> = decode(data)?;
        if message.session_id != self.session_id
            || message.role != "assistant"
            || self
                .active_parent
                .as_deref()
                .is_some_and(|parent| message.parent_id != Some(parent))
        {
            return ignored();
        }
        bounded_insert(&mut self.assistants, message.id)?;
        if (message.time.completed.is_some() || message.error.is_some())
            && bounded_insert(&mut self.completed, message.id)?
        {
            match message.tokens {
                None => self.missing_usage = true,
                Some(tokens) => add_usage(
                    &mut self.usage,
                    decode::<Tokens>(tokens.get())?.normalize()?,
                )?,
            }
        }
        if message.error.is_some() {
            return observed(RuntimeObservation::Failure {
                kind: failure(message.error)?,
            });
        }
        ignored()
    }

    fn part(&mut self, data: &str) -> Result<SessionEvent, AdapterError> {
        let part: Part<'_> = decode(data)?;
        if part.session_id != self.session_id || !self.assistants.contains(part.message_id) {
            return ignored();
        }
        match part.kind {
            "text" if part.time.is_some_and(|time| time.end.is_some()) => {
                if !bounded_insert(&mut self.text_parts, part.id)? {
                    return ignored();
                }
                let text: SensitiveText =
                    decode(part.text.ok_or(AdapterError::InvalidEvent)?.get())?;
                observed(RuntimeObservation::AssistantOutput { text: text.0 })
            }
            "tool" => {
                let state: ToolState<'_> =
                    decode(part.state.ok_or(AdapterError::InvalidEvent)?.get())?;
                let (phase, succeeded) = match state.status {
                    "pending" => (RuntimeActivityPhase::Started, None),
                    "running" => (RuntimeActivityPhase::Updated, None),
                    "completed" => (RuntimeActivityPhase::Completed, Some(true)),
                    "error" => (RuntimeActivityPhase::Completed, Some(false)),
                    _ => return ignored(),
                };
                let kind = match part.tool.unwrap_or("") {
                    "bash" => RuntimeToolKind::Shell,
                    "edit" | "write" | "apply_patch" => RuntimeToolKind::FileChange,
                    "webfetch" | "websearch" => RuntimeToolKind::WebSearch,
                    "todowrite" => RuntimeToolKind::Plan,
                    tool if tool.starts_with("forge_") => RuntimeToolKind::Mcp,
                    _ => return ignored(),
                };
                observed(RuntimeObservation::ToolActivity {
                    kind,
                    phase,
                    succeeded,
                })
            }
            _ => ignored(),
        }
    }
}

fn bounded_insert(set: &mut BTreeSet<String>, id: &str) -> Result<bool, AdapterError> {
    if set.contains(id) {
        return Ok(false);
    }
    if set.len() >= MAX_TRACKED_ITEMS || id.len() > 256 {
        return Err(AdapterError::InvalidEvent);
    }
    Ok(set.insert(id.into()))
}
fn observed(event: RuntimeObservation) -> Result<SessionEvent, AdapterError> {
    Ok(SessionEvent::Observation(event))
}
fn ignored() -> Result<SessionEvent, AdapterError> {
    observed(RuntimeObservation::Ignored)
}
fn decode<'a, T: Deserialize<'a>>(data: &'a str) -> Result<T, AdapterError> {
    serde_json::from_str(data).map_err(|_| AdapterError::InvalidEvent)
}

#[derive(Deserialize)]
struct EventEnvelope<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(borrow)]
    properties: &'a RawValue,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Properties<'a> {
    #[serde(rename = "sessionID")]
    session_id: Option<&'a str>,
    #[serde(borrow)]
    status: Option<&'a RawValue>,
    #[serde(borrow)]
    error: Option<&'a RawValue>,
    #[serde(borrow)]
    info: Option<&'a RawValue>,
    #[serde(borrow)]
    part: Option<&'a RawValue>,
}
#[derive(Deserialize)]
struct Kind<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Message<'a> {
    id: &'a str,
    #[serde(rename = "sessionID")]
    session_id: &'a str,
    role: &'a str,
    #[serde(rename = "parentID")]
    parent_id: Option<&'a str>,
    time: MessageTime,
    #[serde(borrow)]
    error: Option<&'a RawValue>,
    #[serde(borrow)]
    tokens: Option<&'a RawValue>,
}
#[derive(Deserialize)]
struct MessageTime {
    completed: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Part<'a> {
    id: &'a str,
    #[serde(rename = "sessionID")]
    session_id: &'a str,
    #[serde(rename = "messageID")]
    message_id: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    time: Option<PartTime>,
    #[serde(borrow)]
    text: Option<&'a RawValue>,
    tool: Option<&'a str>,
    #[serde(borrow)]
    state: Option<&'a RawValue>,
}
#[derive(Deserialize, Clone, Copy)]
struct PartTime {
    end: Option<u64>,
}
#[derive(Deserialize)]
struct ToolState<'a> {
    status: &'a str,
}
#[derive(Deserialize)]
struct Tokens {
    input: u64,
    output: u64,
    reasoning: Option<u64>,
    cache: Option<CacheTokens>,
}
#[derive(Deserialize)]
struct CacheTokens {
    read: u64,
    write: u64,
}
impl Tokens {
    fn normalize(self) -> Result<RuntimeUsage, AdapterError> {
        let (read, write) = self
            .cache
            .map_or((None, None), |cache| (Some(cache.read), Some(cache.write)));
        // OpenCode reports non-cached input separately; common input is total.
        let input_tokens = self
            .input
            .checked_add(read.unwrap_or(0))
            .and_then(|value| value.checked_add(write.unwrap_or(0)))
            .ok_or(AdapterError::InvalidUsage)?;
        Ok(RuntimeUsage {
            input_tokens,
            output_tokens: self.output,
            cached_input_tokens: read,
            cache_write_input_tokens: write,
            reasoning_output_tokens: self.reasoning,
        })
    }
}
fn add_usage(total: &mut Option<RuntimeUsage>, next: RuntimeUsage) -> Result<(), AdapterError> {
    let Some(total) = total else {
        *total = Some(next);
        return Ok(());
    };
    let sum = |a: u64, b: u64| a.checked_add(b).ok_or(AdapterError::InvalidUsage);
    let optional = |a: Option<u64>, b: Option<u64>| a.zip(b).map(|(a, b)| sum(a, b)).transpose();
    total.input_tokens = sum(total.input_tokens, next.input_tokens)?;
    total.output_tokens = sum(total.output_tokens, next.output_tokens)?;
    total.cached_input_tokens = optional(total.cached_input_tokens, next.cached_input_tokens)?;
    total.cache_write_input_tokens = optional(
        total.cache_write_input_tokens,
        next.cache_write_input_tokens,
    )?;
    total.reasoning_output_tokens =
        optional(total.reasoning_output_tokens, next.reasoning_output_tokens)?;
    Ok(())
}

fn failure(data: Option<&RawValue>) -> Result<RuntimeFailureKind, AdapterError> {
    #[derive(Deserialize)]
    struct ErrorData {
        #[serde(rename = "statusCode")]
        status_code: Option<u16>,
    }
    #[derive(Deserialize)]
    struct Error<'a> {
        name: &'a str,
        data: Option<ErrorData>,
    }
    let error: Error<'_> = decode(data.ok_or(AdapterError::InvalidEvent)?.get())?;
    Ok(
        match (error.name, error.data.and_then(|data| data.status_code)) {
            ("ProviderAuthError", _) | (_, Some(401 | 403)) => RuntimeFailureKind::AuthRequired,
            (_, Some(429)) => RuntimeFailureKind::RateLimited,
            (_, Some(500..=599)) => RuntimeFailureKind::ProviderUnavailable,
            _ => RuntimeFailureKind::RuntimeError,
        },
    )
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

pub use forge_provider_common::driver_event::{parse_driver_event, write_driver_event};
