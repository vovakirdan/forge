//! Reject known credentials before Run-provided content reaches canonical records.

use crate::{CoreError, CoreService, credentials::credential_error};
use forge_domain::runtime::RunScope;
use forge_provider_common::SecretBytes;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct AuthDocument<'a> {
    #[serde(borrow)]
    tokens: AuthTokens<'a>,
}
#[derive(Deserialize)]
struct AuthTokens<'a> {
    access_token: &'a str,
    refresh_token: &'a str,
    id_token: &'a str,
}

impl CoreService {
    pub(crate) async fn run_redaction_secrets(
        &self,
        transaction: &mut forge_storage::StorageTransaction<'_>,
        run: &forge_storage::RunProjection,
    ) -> Result<zeroize::Zeroizing<Vec<Vec<u8>>>, CoreError> {
        let mut secrets = zeroize::Zeroizing::new(Vec::new());
        let spec: forge_domain::runtime::SandboxRunSpec =
            serde_json::from_value(run.run_spec.clone()).map_err(|_| credential_error())?;
        let record = transaction
            .run_credential(run.id)
            .await?
            .ok_or_else(credential_error)?;
        let secret = self.open_runtime_credential(
            &record,
            spec.binding.execution_profile.credential_binding(),
        )?;
        if spec.binding.execution_profile.adapter_id() == "codex_cli" {
            add_auth_tokens(&secret, &mut secrets)?;
            if let Some(execution) = &self.execution {
                let path = execution
                    .root
                    .join("runtime")
                    .join(run.id.to_string())
                    .join(run.environment_epoch.to_string())
                    .join("codex-home/auth.json");
                let relative = path
                    .strip_prefix(&execution.root)
                    .map_err(|_| credential_error())?;
                match std::fs::symlink_metadata(&path) {
                    Ok(_) => {
                        let secret = forge_provider_common::PrivateMaterialization::read_beneath(
                            &execution.root,
                            relative,
                            1024 * 1024,
                        )
                        .map_err(|_| credential_error())?;
                        add_auth_tokens(&secret, &mut secrets)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if transaction
                            .run_recovery_state(run.id)
                            .await?
                            .is_some_and(|state| state.started_at.is_some())
                        {
                            return Err(credential_error());
                        }
                    }
                    Err(_) => return Err(credential_error()),
                }
            }
        } else {
            secrets.push(secret.expose().to_vec());
        }
        if let Some(saved) = transaction.run_proxy_key(run.id).await? {
            secrets.push(
                self.open_proxy_key(run, &saved.sealed_key)?
                    .expose()
                    .to_vec(),
            );
        }
        Ok(secrets)
    }
    pub(crate) async fn reject_run_secrets(
        &self,
        scope: RunScope,
        value: &Value,
    ) -> Result<(), CoreError> {
        let mut transaction = self.store.begin().await?;
        let run = transaction
            .validate_gateway_scope(&scope)
            .await?
            .ok_or_else(credential_error)?;
        // M0 has no credential material. Real Runs must have a pinned snapshot.
        if run.run_spec_version == 1 {
            return Ok(());
        }
        for secret in self
            .run_redaction_secrets(&mut transaction, &run)
            .await?
            .iter()
        {
            if contains_value(value, secret) {
                return Err(CoreError::InvalidTransport {
                    field: "submission",
                    reason: "known credential material is forbidden in submissions".into(),
                });
            }
        }
        transaction.commit().await?;
        Ok(())
    }
}

fn add_auth_tokens(secret: &SecretBytes, secrets: &mut Vec<Vec<u8>>) -> Result<(), CoreError> {
    let auth: AuthDocument<'_> =
        serde_json::from_slice(secret.expose()).map_err(|_| credential_error())?;
    secrets.extend(
        [
            auth.tokens.access_token,
            auth.tokens.refresh_token,
            auth.tokens.id_token,
        ]
        .iter()
        .map(|token| token.as_bytes().to_vec()),
    );
    Ok(())
}

#[cfg(test)]
fn reject_known(value: &Value, secret: &SecretBytes, codex: bool) -> Result<(), CoreError> {
    let contains = if codex {
        let auth: AuthDocument<'_> =
            serde_json::from_slice(secret.expose()).map_err(|_| credential_error())?;
        [
            auth.tokens.access_token,
            auth.tokens.refresh_token,
            auth.tokens.id_token,
        ]
        .iter()
        .any(|token| contains_value(value, token.as_bytes()))
    } else {
        contains_value(value, secret.expose())
    };
    if contains {
        Err(CoreError::InvalidTransport {
            field: "submission",
            reason: "known credential material is forbidden in submissions".into(),
        })
    } else {
        Ok(())
    }
}

fn contains_value(value: &Value, secret: &[u8]) -> bool {
    if secret.is_empty() {
        return false;
    }
    match value {
        Value::String(text) => text
            .as_bytes()
            .windows(secret.len())
            .any(|window| window == secret),
        Value::Array(values) => values.iter().any(|value| contains_value(value, secret)),
        Value::Object(values) => values.iter().any(|(key, value)| {
            key.as_bytes()
                .windows(secret.len())
                .any(|window| window == secret)
                || contains_value(value, secret)
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_nested_and_json_escaped_credentials_without_echo() {
        let key = SecretBytes::new(b"synthetic-secret\"value".to_vec());
        let payload =
            serde_json::json!({"metadata":[{"value":"prefix synthetic-secret\"value suffix"}]});
        let error = reject_known(&payload, &key, false).expect_err("must reject");
        assert!(!error.to_string().contains("synthetic-secret"));
    }
    #[test]
    fn account_identity_is_not_treated_as_a_token() {
        let auth=SecretBytes::new(br#"{"tokens":{"access_token":"synthetic-access","refresh_token":"synthetic-refresh","id_token":"synthetic-id","account_id":"account"}}"#.to_vec());
        assert!(reject_known(&serde_json::json!({"note":"account"}), &auth, true).is_ok());
        assert!(
            reject_known(
                &serde_json::json!({"note":"synthetic-refresh"}),
                &auth,
                true
            )
            .is_err()
        );
    }
}
