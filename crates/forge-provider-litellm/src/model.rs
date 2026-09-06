use std::{collections::BTreeSet, fmt::Write};

use forge_provider_common::SecretBytes;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

pub(crate) const ALLOWED_ROUTES: [&str; 4] = [
    "/chat/completions",
    "/v1/chat/completions",
    "/responses",
    "/v1/responses",
];

/// Core must persist this intent and the generated virtual key before issuance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunKeySpec {
    pub run_id: Uuid,
    pub project_id: Uuid,
    pub environment_epoch: u64,
    pub fencing_token: u64,
    pub execution_profile_id: Uuid,
    pub execution_profile_revision: u64,
    pub models: BTreeSet<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub requests_per_minute: u32,
    pub tokens_per_minute: u32,
    /// Observed-spend gate in LiteLLM, not a guarantee against in-flight overshoot.
    pub max_budget_usd: Option<f64>,
}

impl RunKeySpec {
    pub fn alias(&self) -> String {
        format!("forge-run-{}-e{}", self.run_id, self.environment_epoch)
    }

    pub(crate) fn validate(&self) -> Result<(), LiteLlmError> {
        if [self.run_id, self.project_id, self.execution_profile_id]
            .iter()
            .any(Uuid::is_nil)
            || self.environment_epoch == 0
            || self.fencing_token == 0
            || self.execution_profile_revision == 0
            || self.models.is_empty()
            || self.models.len() > 32
            || self.models.iter().any(|model| {
                model.trim().is_empty()
                    || model.trim() != model
                    || model.len() > 256
                    || model.contains('*')
                    || model.chars().any(char::is_control)
            })
            || self.requests_per_minute == 0
            || self.tokens_per_minute == 0
            || self
                .max_budget_usd
                .is_some_and(|budget| !budget.is_finite() || budget <= 0.0)
        {
            return Err(LiteLlmError::InvalidPolicy);
        }
        Ok(())
    }

    pub(crate) fn metadata(&self) -> Result<serde_json::Value, LiteLlmError> {
        Ok(serde_json::json!({"forge": {
            "run_id":self.run_id,"project_id":self.project_id,"environment_epoch":self.environment_epoch,
            "fencing_token":self.fencing_token,"execution_profile_id":self.execution_profile_id,
            "execution_profile_revision":self.execution_profile_revision,
            "expires_at":self.expires_at.format(&Rfc3339).map_err(|_|LiteLlmError::InvalidPolicy)?
        }}))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyProvisioned {
    pub alias: String,
    /// SHA256 lookup identity, never the plaintext virtual key.
    pub key_hash: String,
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpendObservation {
    pub spend_usd: Option<f64>,
    #[serde(with = "time::serde::rfc3339")]
    pub observed_at: OffsetDateTime,
    /// LiteLLM accounting can lag the last inference response.
    pub final_accounting_confirmed: bool,
}

#[derive(Debug, Error)]
pub enum LiteLlmError {
    #[error("LiteLLM endpoint or key policy is invalid")]
    InvalidPolicy,
    #[error("LiteLLM request failed; reconcile the persisted intent before retrying")]
    Transport,
    #[error("LiteLLM rejected the request with HTTP {0}")]
    Http(u16),
    #[error("LiteLLM returned malformed or oversized metadata")]
    InvalidResponse,
    #[error("existing LiteLLM key does not match the persisted Run policy")]
    PolicyConflict,
    #[error("virtual-key random generation failed")]
    Random,
}

/// Generate once, seal with Forge Secret Store, then reuse for idempotent issue.
pub fn generate_virtual_key() -> Result<SecretBytes, LiteLlmError> {
    let random = SecretBytes::random(32).map_err(|_| LiteLlmError::Random)?;
    let mut text = zeroize::Zeroizing::new(String::from("sk-forge-"));
    for byte in random.expose() {
        write!(&mut *text, "{byte:02x}").map_err(|_| LiteLlmError::Random)?;
    }
    Ok(SecretBytes::new(std::mem::take(&mut *text).into_bytes()))
}
