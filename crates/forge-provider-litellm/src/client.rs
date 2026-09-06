use std::{collections::BTreeSet, fmt::Write, time::Duration};

use forge_provider_common::SecretBytes;
use reqwest::{
    Client, Method, StatusCode, Url,
    header::{AUTHORIZATION, HeaderValue},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use zeroize::Zeroizing;

use crate::{KeyProvisioned, LiteLlmError, RunKeySpec, SpendObservation, model::ALLOWED_ROUTES};

/// Admin access remains Core-side. Requests never log credentials or raw errors.
pub struct LiteLlmClient {
    http: Client,
    base: Url,
    authorization: HeaderValue,
}

impl LiteLlmClient {
    pub fn new(endpoint: &str, master_key: &SecretBytes) -> Result<Self, LiteLlmError> {
        let base = Url::parse(endpoint).map_err(|_| LiteLlmError::InvalidPolicy)?;
        let local = base
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "[::1]"));
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
            || !(base.scheme() == "https" || (base.scheme() == "http" && local))
        {
            return Err(LiteLlmError::InvalidPolicy);
        }
        let token =
            std::str::from_utf8(master_key.expose()).map_err(|_| LiteLlmError::InvalidPolicy)?;
        if token.is_empty() {
            return Err(LiteLlmError::InvalidPolicy);
        }
        let header = Zeroizing::new(format!("Bearer {token}"));
        let mut authorization =
            HeaderValue::from_str(&header).map_err(|_| LiteLlmError::InvalidPolicy)?;
        authorization.set_sensitive(true);
        let http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| LiteLlmError::Transport)?;
        Ok(Self {
            http,
            base,
            authorization,
        })
    }

    /// Reuses the already-sealed Run key, including after a lost create response.
    pub async fn create_or_reconcile(
        &self,
        spec: &RunKeySpec,
        virtual_key: &SecretBytes,
        now: OffsetDateTime,
    ) -> Result<KeyProvisioned, LiteLlmError> {
        spec.validate()?;
        if spec.expires_at <= now {
            return Err(LiteLlmError::InvalidPolicy);
        }
        let hash = key_hash(virtual_key)?;
        if let Some(info) = self.info(&hash).await? {
            verify_info(spec, &info, now)?;
            return Ok(KeyProvisioned {
                alias: spec.alias(),
                key_hash: hash,
                reused: true,
            });
        }
        // The local server uses duration rather than absolute expiry. Reserve two
        // request timeouts so issuance cannot silently extend the intent deadline.
        let seconds = (spec.expires_at - now).whole_seconds() - 30;
        if seconds <= 0 {
            return Err(LiteLlmError::InvalidPolicy);
        }
        let key =
            std::str::from_utf8(virtual_key.expose()).map_err(|_| LiteLlmError::InvalidPolicy)?;
        let policy = json!({
            "key_alias":spec.alias(),"duration":format!("{seconds}s"),"models":spec.models,
            "metadata":spec.metadata()?,"rpm_limit":spec.requests_per_minute,"tpm_limit":spec.tokens_per_minute,
            "max_budget":spec.max_budget_usd,"budget_duration":null,"auto_rotate":false,
            // In 1.99.0 key_type=llm_api overwrites explicit routes with a broad
            // group. Null preserves this minimal allowlist without admin routes.
            "key_type":null,"allowed_routes":ALLOWED_ROUTES,"allowed_passthrough_routes":[],
            "permissions":{},"config":{},"aliases":{}
        });
        #[derive(serde::Serialize)]
        struct Issue<'a> {
            key: &'a str,
            #[serde(flatten)]
            policy: Value,
        }
        let body = Issue { key, policy };
        let response = self
            .request(Method::POST, "key/generate")?
            .json(&body)
            .send()
            .await
            .map_err(|_| LiteLlmError::Transport)?;
        if !response.status().is_success() && response.status() != StatusCode::CONFLICT {
            return Err(LiteLlmError::Http(response.status().as_u16()));
        }
        let info = self
            .info(&hash)
            .await?
            .ok_or(LiteLlmError::InvalidResponse)?;
        verify_info(spec, &info, now)?;
        Ok(KeyProvisioned {
            alias: spec.alias(),
            key_hash: hash,
            reused: false,
        })
    }

    pub async fn revoke(&self, spec: &RunKeySpec, key_hash: &str) -> Result<(), LiteLlmError> {
        spec.validate()?;
        validate_hash(key_hash)?;
        let Some(info) = self.info(key_hash).await? else {
            return Ok(());
        };
        verify_identity(spec, &info)?;
        let response = self
            .request(Method::POST, "key/delete")?
            // Aliases are not unique in LiteLLM; delete only the checked hash.
            .json(&json!({"keys":[key_hash]}))
            .send()
            .await
            .map_err(|_| LiteLlmError::Transport)?;
        if !response.status().is_success() {
            return Err(LiteLlmError::Http(response.status().as_u16()));
        }
        if self.info(key_hash).await?.is_some() {
            return Err(LiteLlmError::PolicyConflict);
        }
        Ok(())
    }

    pub async fn usage(
        &self,
        spec: &RunKeySpec,
        key_hash: &str,
        now: OffsetDateTime,
    ) -> Result<SpendObservation, LiteLlmError> {
        validate_hash(key_hash)?;
        let info = self
            .info(key_hash)
            .await?
            .ok_or(LiteLlmError::InvalidResponse)?;
        verify_identity(spec, &info)?;
        let spend_usd = match info.get("spend") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_f64()
                    .filter(|spend| spend.is_finite() && *spend >= 0.0)
                    .ok_or(LiteLlmError::InvalidResponse)?,
            ),
        };
        Ok(SpendObservation {
            spend_usd,
            observed_at: now,
            final_accounting_confirmed: false,
        })
    }

    pub(crate) fn request(
        &self,
        method: Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, LiteLlmError> {
        let url = self
            .base
            .join(path)
            .map_err(|_| LiteLlmError::InvalidPolicy)?;
        Ok(self
            .http
            .request(method, url)
            .header(AUTHORIZATION, self.authorization.clone()))
    }

    async fn info(&self, hash: &str) -> Result<Option<Value>, LiteLlmError> {
        let mut response = self
            .request(Method::GET, "key/info")?
            .query(&[("key", hash)])
            .send()
            .await
            .map_err(|_| LiteLlmError::Transport)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
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
        let mut response: Value =
            serde_json::from_slice(&bytes).map_err(|_| LiteLlmError::InvalidResponse)?;
        let info = response
            .get_mut("info")
            .filter(|info| info.is_object())
            .ok_or(LiteLlmError::InvalidResponse)?
            .take();
        Ok(Some(info))
    }
}

fn key_hash(key: &SecretBytes) -> Result<String, LiteLlmError> {
    if !key.expose().starts_with(b"sk-") || key.expose().len() < 16 {
        return Err(LiteLlmError::InvalidPolicy);
    }
    let mut hash = String::with_capacity(64);
    for byte in Sha256::digest(key.expose()) {
        write!(&mut hash, "{byte:02x}").map_err(|_| LiteLlmError::InvalidPolicy)?;
    }
    Ok(hash)
}
fn validate_hash(hash: &str) -> Result<(), LiteLlmError> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(LiteLlmError::InvalidPolicy);
    }
    Ok(())
}
fn verify_identity(spec: &RunKeySpec, info: &Value) -> Result<(), LiteLlmError> {
    let metadata = info.get("metadata").ok_or(LiteLlmError::InvalidResponse)?;
    let expected = spec.metadata()?;
    if info.get("key_alias").and_then(Value::as_str) != Some(&spec.alias())
        || metadata.get("forge") != expected.get("forge")
        // LiteLLM 1.99.0 stores allowed_passthrough_routes inside metadata.
        || !empty_or_null(metadata, "allowed_passthrough_routes")
        || !metadata.as_object().is_some_and(|metadata| {
            metadata.keys().all(|key| matches!(key.as_str(), "forge" | "allowed_passthrough_routes"))
        })
    {
        return Err(LiteLlmError::PolicyConflict);
    }
    Ok(())
}
fn verify_info(spec: &RunKeySpec, info: &Value, now: OffsetDateTime) -> Result<(), LiteLlmError> {
    verify_identity(spec, info)?;
    let strings = |field| -> Result<BTreeSet<String>, LiteLlmError> {
        info.get(field)
            .and_then(Value::as_array)
            .ok_or(LiteLlmError::InvalidResponse)?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(LiteLlmError::InvalidResponse)
            })
            .collect()
    };
    let expires = info
        .get("expires")
        .and_then(Value::as_str)
        .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
        .ok_or(LiteLlmError::InvalidResponse)?;
    if strings("models")? != spec.models
        || strings("allowed_routes")? != ALLOWED_ROUTES.into_iter().map(str::to_owned).collect()
        || info.get("rpm_limit").and_then(Value::as_u64)
            != Some(u64::from(spec.requests_per_minute))
        || info.get("tpm_limit").and_then(Value::as_u64) != Some(u64::from(spec.tokens_per_minute))
        || info.get("max_budget").and_then(Value::as_f64) != spec.max_budget_usd
        || info.get("blocked").and_then(Value::as_bool) == Some(true)
        || info.get("auto_rotate").and_then(Value::as_bool) == Some(true)
        || !empty_or_null(info, "budget_duration")
        || !empty_or_null(info, "aliases")
        || !empty_or_null(info, "config")
        || !empty_or_null(info, "permissions")
        || !empty_or_null(info, "allowed_passthrough_routes")
        || expires > spec.expires_at
        || expires <= now
    {
        return Err(LiteLlmError::PolicyConflict);
    }
    Ok(())
}

fn empty_or_null(info: &Value, field: &str) -> bool {
    match info.get(field) {
        None | Some(Value::Null) => true,
        Some(Value::Object(value)) => value.is_empty(),
        Some(Value::Array(value)) => value.is_empty(),
        _ => false,
    }
}
