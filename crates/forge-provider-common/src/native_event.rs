//! Allowlisted live-driver metadata. Provider transcripts and error bodies are excluded.
use crate::{
    SecretBytes,
    adapter::{AdapterError, RuntimeFailureKind, RuntimeUsage},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UsageBasis {
    PerTurn,
    Cumulative,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "forge_event", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeDriverEvent {
    InputAccepted {
        run_id: Uuid,
        input_id: Uuid,
        session_id: String,
        turn_id: String,
    },
    TurnFinished {
        run_id: Uuid,
        input_id: Uuid,
        session_id: String,
        turn_id: String,
        completed: bool,
        usage: Option<RuntimeUsage>,
        usage_basis: UsageBasis,
    },
    Failure {
        run_id: Uuid,
        input_id: Uuid,
        kind: RuntimeFailureKind,
    },
}
impl NativeDriverEvent {
    pub fn encode(&self) -> Result<SecretBytes, AdapterError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map(SecretBytes::new)
            .map_err(|_| AdapterError::InvalidEvent)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, AdapterError> {
        if bytes.len() > 4096 {
            return Err(AdapterError::InvalidEvent);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| AdapterError::InvalidEvent)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), AdapterError> {
        let (run, input) = match self {
            Self::InputAccepted {
                run_id,
                input_id,
                session_id,
                turn_id,
            }
            | Self::TurnFinished {
                run_id,
                input_id,
                session_id,
                turn_id,
                ..
            } => {
                for id in [session_id, turn_id] {
                    if id.is_empty()
                        || id.len() > 256
                        || !id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    {
                        return Err(AdapterError::InvalidEvent);
                    }
                }
                (*run_id, *input_id)
            }
            Self::Failure {
                run_id, input_id, ..
            } => (*run_id, *input_id),
        };
        if run.get_version_num() != 7 || input.get_version_num() != 7 {
            return Err(AdapterError::InvalidEvent);
        }
        if let Self::TurnFinished {
            usage: Some(usage), ..
        } = self
            && (usage
                .cached_input_tokens
                .is_some_and(|n| n > usage.input_tokens)
                || usage
                    .cache_write_input_tokens
                    .is_some_and(|n| n > usage.input_tokens)
                || usage
                    .reasoning_output_tokens
                    .is_some_and(|n| n > usage.output_tokens))
        {
            return Err(AdapterError::InvalidUsage);
        }
        Ok(())
    }
}
