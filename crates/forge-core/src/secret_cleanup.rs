//! Exact ephemeral-file cleanup after positive exit, writeback and redaction.

use crate::{CoreError, CoreService, credentials::credential_error};
use forge_provider_common::PrivateMaterialization;

impl CoreService {
    pub(crate) async fn cleanup_retired_run_secrets(
        &self,
        only_run: Option<uuid::Uuid>,
    ) -> Result<(), CoreError> {
        let Some(execution) = &self.execution else {
            return Ok(());
        };
        for run_id in self.store.secret_cleanup_candidates(only_run).await? {
            let mut transaction = self.store.begin().await?;
            let run = transaction
                .load_run(run_id)
                .await?
                .ok_or_else(credential_error)?;
            transaction
                .lock_project(run.project_id)
                .await?
                .ok_or_else(credential_error)?;
            let adapter = run
                .run_spec
                .pointer("/binding/execution_profile/adapter_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(credential_error)?;
            let codex = adapter == "codex_cli";
            let safe = !codex || transaction.auth_cleanup_is_safe(run_id).await?;
            let grants = std::path::PathBuf::from("grants")
                .join(run.id.to_string())
                .join(run.environment_epoch.to_string());
            let runtime = std::path::PathBuf::from("runtime")
                .join(run.id.to_string())
                .join(run.environment_epoch.to_string());
            // Pinned initial credential is already encrypted. Failed/unsafe
            // returned auth remains quarantined for explicit operator recovery.
            let mut files = vec![
                grants.join("auth.json"),
                grants.join("api-key"),
                grants.join("claude-setup-token"),
                grants.join("stdin"),
            ];
            if safe {
                let path = match adapter {
                    "codex_cli" => "codex-home/auth.json",
                    "opencode_runtime" => "opencode/virtual-key",
                    "claude_code_cli" => "claude-home/setup-token",
                    _ => return Err(credential_error()),
                };
                files.push(runtime.join(path));
            }
            let root = execution.root.clone();
            let removed = tokio::task::spawn_blocking(move || {
                let mut removed = 0;
                for file in files {
                    removed += usize::from(PrivateMaterialization::cleanup_beneath(&root, &file)?);
                }
                Ok::<_, forge_provider_common::SecretStoreError>(removed)
            })
            .await
            .map_err(|_| credential_error())?
            .map_err(|_| credential_error())?;
            transaction
                .record_secret_cleanup(
                    run_id,
                    if safe {
                        "removed"
                    } else {
                        "unsafe_auth_retained"
                    },
                )
                .await?;
            transaction.commit().await?;
            tracing::info!(%run_id,removed,unsafe_auth_retained=!safe,"retired Run private delivery copies cleaned; encrypted credentials and technical evidence retained");
        }
        Ok(())
    }
}
