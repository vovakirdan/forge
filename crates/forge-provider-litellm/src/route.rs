use std::fmt::Write;

use forge_provider_common::SecretBytes;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{LiteLlmClient, LiteLlmError};

/// Immutable route intent, persisted by Core before contacting LiteLLM. The
/// upstream credential travels only Core→LiteLLM, never in a Run grant or key info.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSpec {
    pub project_id: Uuid,
    pub credential_binding_id: Uuid,
    pub secret_id: Uuid,
    pub secret_version: u64,
    pub provider_id: String,
    pub model: String,
}
impl RouteSpec {
    pub fn alias(&self) -> Result<String, LiteLlmError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| LiteLlmError::InvalidPolicy)?;
        let mut alias = String::from("forge-route-");
        for byte in Sha256::digest(bytes) {
            write!(&mut alias, "{byte:02x}").map_err(|_| LiteLlmError::InvalidPolicy)?;
        }
        Ok(alias)
    }
    fn validate(&self) -> Result<(), LiteLlmError> {
        if [self.project_id, self.credential_binding_id, self.secret_id]
            .iter()
            .any(Uuid::is_nil)
            || self.secret_version == 0
            || self.model.trim().is_empty()
            || self.model != self.model.trim()
            || self.model.len() > 256
            || self.model.contains('*')
            || self.model.chars().any(char::is_control)
        {
            return Err(LiteLlmError::InvalidPolicy);
        }
        self.upstream_model()?;
        Ok(())
    }
    fn upstream_model(&self) -> Result<String, LiteLlmError> {
        let prefix = match self.provider_id.as_str() {
            "openai" => "openai",
            "anthropic" => "anthropic",
            "gemini" | "google" => "gemini",
            "openrouter" => "openrouter",
            "xai" => "xai",
            _ => return Err(LiteLlmError::InvalidPolicy),
        };
        Ok(format!("{prefix}/{}", self.model))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteProvisioned {
    pub alias: String,
    pub reused: bool,
}

impl LiteLlmClient {
    /// Uses OSS `/model/new` with STORE_MODEL_IN_DB=True. LiteLLM encrypts
    /// stored credentials using its separately persisted LITELLM_SALT_KEY.
    /// Existing routes are checked, never edited or silently rebound.
    pub async fn ensure_route(
        &self,
        spec: &RouteSpec,
        upstream_key: &SecretBytes,
    ) -> Result<RouteProvisioned, LiteLlmError> {
        let alias = spec.alias()?;
        if self.route_exists(spec, &alias).await? {
            return Ok(RouteProvisioned {
                alias,
                reused: true,
            });
        }
        let key =
            std::str::from_utf8(upstream_key.expose()).map_err(|_| LiteLlmError::InvalidPolicy)?;
        if key.is_empty() || key.len() > 16 * 1024 || key.chars().any(char::is_control) {
            return Err(LiteLlmError::InvalidPolicy);
        }
        #[derive(Serialize)]
        struct Parameters<'a> {
            model: String,
            api_key: &'a str,
        }
        #[derive(Serialize)]
        struct Deployment<'a> {
            model_name: &'a str,
            litellm_params: Parameters<'a>,
            model_info: Value,
        }
        let body = Deployment {
            model_name: &alias,
            litellm_params: Parameters {
                model: spec.upstream_model()?,
                api_key: key,
            },
            model_info: json!({"id":alias,"forge":spec}),
        };
        let response = self
            .request(Method::POST, "model/new")?
            .json(&body)
            .send()
            .await
            .map_err(|_| LiteLlmError::Transport)?;
        let status = response.status();
        // A concurrent create can return a non-2xx duplicate-ID response. The
        // exact immutable route must still be visible and conformant afterward.
        if !status.is_success() && !matches!(status.as_u16(), 400 | 409 | 500) {
            return Err(LiteLlmError::Http(status.as_u16()));
        }
        drop(response); // Creation responses may contain encrypted parameters.
        if !self.route_exists(spec, &alias).await? {
            return Err(LiteLlmError::Http(status.as_u16()));
        }
        Ok(RouteProvisioned {
            alias,
            reused: false,
        })
    }

    async fn route_exists(&self, spec: &RouteSpec, alias: &str) -> Result<bool, LiteLlmError> {
        // Inspect the sanitized model catalog, not raw /model/*/credentials.
        // Also detect duplicate aliases which would enable load-balancing across
        // a different credential binding despite a matching deployment ID.
        let mut response = self
            .request(Method::GET, "model/info")?
            .send()
            .await
            .map_err(|_| LiteLlmError::Transport)?;
        if !response.status().is_success() {
            return Err(LiteLlmError::Http(response.status().as_u16()));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| LiteLlmError::Transport)?
        {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err(LiteLlmError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        let response: Value =
            serde_json::from_slice(&bytes).map_err(|_| LiteLlmError::InvalidResponse)?;
        let models = response
            .get("data")
            .and_then(Value::as_array)
            .ok_or(LiteLlmError::InvalidResponse)?;
        let matching: Vec<_> = models
            .iter()
            .filter(|model| {
                model.get("model_name").and_then(Value::as_str) == Some(alias)
                    || model.pointer("/model_info/id").and_then(Value::as_str) == Some(alias)
            })
            .collect();
        if matching.is_empty() {
            return Ok(false);
        }
        if matching.len() != 1 {
            return Err(LiteLlmError::PolicyConflict);
        }
        let model = matching[0];
        if model.get("model_name").and_then(Value::as_str) != Some(alias)
            || model.pointer("/model_info/id").and_then(Value::as_str) != Some(alias)
            || model.pointer("/model_info/forge")
                != Some(&serde_json::to_value(spec).map_err(|_| LiteLlmError::InvalidPolicy)?)
            || model
                .pointer("/litellm_params/model")
                .and_then(Value::as_str)
                != Some(&spec.upstream_model()?)
        {
            return Err(LiteLlmError::PolicyConflict);
        }
        Ok(true)
    }
}
