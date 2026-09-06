//! Core-owned enrollment and encrypted auth persistence. No credential HTTP bodies.

use crate::{CoreError, CoreService};
use forge_domain::ProjectId;
use forge_provider_common::{
    ManagedAuthSnapshot, PrivateMaterialization, SealedSecret, SecretBytes, SecretScope,
    enroll_codex_auth,
};
use forge_storage::{StorageTransaction, StoredCredential};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// Only encrypted variants can be serialized by the credential repository.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CredentialRecord {
    CodexChatgpt {
        snapshot: ManagedAuthSnapshot,
    },
    ApiKey {
        binding_id: Uuid,
        sealed: SealedSecret,
    },
}

pub(crate) fn credential_error() -> CoreError {
    CoreError::InvalidTransport {
        field: "credential",
        reason: "managed credential is unavailable or violates its scope/policy".into(),
    }
}

impl CoreService {
    pub(crate) async fn enroll_credential(
        &self,
        transaction: &mut StorageTransaction<'_>,
        project_id: ProjectId,
        secret_id: Uuid,
        binding_id: Uuid,
        kind: &str,
        source_file: &str,
    ) -> Result<(), CoreError> {
        let store = self.secret_store.as_ref().ok_or_else(credential_error)?;
        let plaintext = PrivateMaterialization::open(Path::new(source_file))
            .and_then(|file| file.read(1024 * 1024))
            .map_err(|_| credential_error())?;
        let record = match kind {
            "codex_chatgpt" => CredentialRecord::CodexChatgpt {
                snapshot: enroll_codex_auth(
                    store,
                    binding_id,
                    project_id.as_uuid(),
                    secret_id,
                    &plaintext,
                )
                .map_err(|_| credential_error())?,
            },
            "api_key" => {
                if plaintext.expose().is_empty() || plaintext.expose().len() > 16 * 1024 {
                    return Err(credential_error());
                }
                CredentialRecord::ApiKey {
                    binding_id,
                    sealed: store
                        .seal(
                            SecretScope {
                                secret_id,
                                project_id: project_id.as_uuid(),
                                version: 1,
                                purpose: format!("api_key:{binding_id}"),
                            },
                            &plaintext,
                        )
                        .map_err(|_| credential_error())?,
                }
            }
            _ => return Err(credential_error()),
        };
        transaction
            .insert_credential(&StoredCredential {
                secret_id,
                project_id,
                version: 1,
                sealed_record: serde_json::to_value(record).map_err(|_| credential_error())?,
            })
            .await?;
        Ok(())
    }

    pub(crate) fn open_runtime_credential(
        &self,
        record: &StoredCredential,
        binding: &forge_domain::CredentialBinding,
    ) -> Result<SecretBytes, CoreError> {
        let store = self.secret_store.as_ref().ok_or_else(credential_error)?;
        if record.secret_id != binding.secret_id || record.project_id != binding.project_id {
            return Err(credential_error());
        }
        let value: CredentialRecord =
            serde_json::from_value(record.sealed_record.clone()).map_err(|_| credential_error())?;
        let (sealed, purpose) = match value {
            CredentialRecord::CodexChatgpt { snapshot } => {
                if snapshot.binding_id != binding.id
                    || binding.account_id.as_deref() != Some(snapshot.account_id.as_str())
                {
                    return Err(credential_error());
                }
                let purpose = format!("codex_auth:{}:{}", snapshot.binding_id, snapshot.session_id);
                (snapshot.auth, purpose)
            }
            CredentialRecord::ApiKey { binding_id, sealed } => {
                if binding_id != binding.id {
                    return Err(credential_error());
                }
                (sealed, format!("api_key:{binding_id}"))
            }
        };
        store
            .open(
                &sealed,
                &SecretScope {
                    secret_id: binding.secret_id,
                    project_id: binding.project_id.as_uuid(),
                    version: record.version,
                    purpose,
                },
            )
            .map_err(|_| credential_error())
    }
}
