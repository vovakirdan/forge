//! Core owns proxy credentials; a Run receives only its restricted virtual key.

use crate::{CoreError, CoreService, credentials::credential_error};
use forge_domain::runtime::SandboxRunSpec;
use forge_provider_common::{PrivateMaterialization, SealedSecret, SecretBytes, SecretScope};
use forge_provider_litellm::{LiteLlmClient, RouteSpec, RunKeySpec, generate_virtual_key};
use forge_storage::{RunProjection, StoredCredential};
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    sync::{Arc, Weak},
};
use time::{Duration, OffsetDateTime};
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

pub(crate) struct InferenceProxy {
    pub admin: LiteLlmClient,
    pub endpoint: String,
    pub http: reqwest::Client,
    issuance_locks: Mutex<HashMap<Uuid, Weak<Mutex<()>>>>,
}
impl InferenceProxy {
    pub fn new(endpoint: &str, master_key: &Path) -> Result<Self, CoreError> {
        let key = PrivateMaterialization::open(master_key)
            .and_then(|file| file.read(16 * 1024))
            .map_err(|_| credential_error())?;
        let admin = LiteLlmClient::new(endpoint, &key).map_err(|_| credential_error())?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| credential_error())?;
        Ok(Self {
            admin,
            endpoint: endpoint.trim_end_matches('/').into(),
            http,
            issuance_locks: Mutex::new(HashMap::new()),
        })
    }

    async fn lock_issuance(&self, run_id: Uuid) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.issuance_locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(&run_id).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(run_id, Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }
}

impl CoreService {
    pub(crate) async fn prepare_proxy_key(
        &self,
        run: &RunProjection,
        spec: &SandboxRunSpec,
        record: &StoredCredential,
        upstream: &SecretBytes,
    ) -> Result<(SecretBytes, String), CoreError> {
        let proxy = self.inference.as_ref().ok_or_else(credential_error)?;
        // Serialize only external issuance for this Run, never execution or a
        // shared subscription. Revocation remains free to run concurrently.
        let _issuance = proxy.lock_issuance(run.id).await;
        let store = self.secret_store.as_ref().ok_or_else(credential_error)?;
        let profile = &spec.binding.execution_profile;
        let route = RouteSpec {
            project_id: run.project_id.as_uuid(),
            credential_binding_id: profile.credential_binding().id,
            secret_id: record.secret_id,
            secret_version: record.version,
            provider_id: profile.provider_id().into(),
            model: profile.model().into(),
        };
        let alias = route.alias().map_err(|_| credential_error())?;
        let mut transaction = self.store.begin().await?;
        let scope = forge_domain::runtime::RunScope {
            run_id: run.id,
            fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
        };
        if !transaction.gateway_scope_is_active(scope).await? {
            return Err(credential_error());
        }
        let (key_spec, key, owns_pending) =
            if let Some(saved) = transaction.run_proxy_key(run.id).await? {
                if saved.revoked {
                    return Err(credential_error());
                }
                let key_spec: RunKeySpec =
                    serde_json::from_value(saved.spec).map_err(|_| credential_error())?;
                let key = self.open_proxy_key(run, &saved.sealed_key)?;
                transaction.begin_proxy_issuance(run.id).await?;
                self.proxy_audit(&mut transaction, run, "issuance_started")
                    .await?;
                // A preexisting pending attempt may outlive a Core crash/network
                // timeout. A later successful attempt cannot settle that uncertainty.
                (key_spec, key, !saved.issuance_pending)
            } else {
                let key = generate_virtual_key().map_err(|_| credential_error())?;
                let key_spec = RunKeySpec {
                    run_id: run.id,
                    project_id: run.project_id.as_uuid(),
                    environment_epoch: run.environment_epoch,
                    fencing_token: run.lease_fencing_token,
                    execution_profile_id: profile.id(),
                    execution_profile_revision: profile.revision(),
                    models: BTreeSet::from([alias.clone()]),
                    expires_at: OffsetDateTime::now_utc()
                        + Duration::seconds(i64::from(spec.binding.limits.wall_seconds))
                        + Duration::minutes(5),
                    requests_per_minute: spec.binding.budget.requests_per_minute,
                    tokens_per_minute: spec.binding.budget.tokens_per_minute,
                    max_budget_usd: spec
                        .binding
                        .budget
                        .max_spend_microusd
                        .map(|amount| amount as f64 / 1_000_000.0),
                };
                let sealed = store
                    .seal(
                        SecretScope {
                            secret_id: run.id,
                            project_id: run.project_id.as_uuid(),
                            version: 1,
                            purpose: proxy_purpose(run),
                        },
                        &key,
                    )
                    .map_err(|_| credential_error())?;
                transaction
                    .insert_proxy_key(
                        run.id,
                        &serde_json::to_value(&key_spec).map_err(|_| credential_error())?,
                        &serde_json::to_value(sealed).map_err(|_| credential_error())?,
                    )
                    .await?;
                self.proxy_audit(&mut transaction, run, "requested").await?;
                (key_spec, key, true)
            };
        transaction.commit().await?;
        let route = match proxy.admin.ensure_route(&route, upstream).await {
            Ok(route) => route,
            Err(_) => {
                if owns_pending {
                    self.settle_owned_proxy_issuance(run, "issuance_not_sent")
                        .await?;
                }
                return Err(credential_error());
            }
        };
        if route.alias != alias || key_spec.models != BTreeSet::from([alias.clone()]) {
            if owns_pending {
                self.settle_owned_proxy_issuance(run, "issuance_not_sent")
                    .await?;
            }
            return Err(credential_error());
        }
        let issued = proxy
            .admin
            .create_or_reconcile(&key_spec, &key, OffsetDateTime::now_utc())
            .await
            .map_err(|_| credential_error())?;
        let mut transaction = self.store.begin().await?;
        let _ = transaction
            .lock_project(run.project_id)
            .await?
            .ok_or_else(credential_error)?;
        let previous = transaction
            .run_proxy_key(run.id)
            .await?
            .ok_or_else(credential_error)?;
        let active = !previous.revoked && transaction.gateway_scope_is_active(scope).await?;
        transaction
            .mark_proxy_key(run.id, &issued.key_hash, false, None)
            .await?;
        if previous.key_hash.is_none() {
            self.proxy_audit(&mut transaction, run, "issued").await?;
        }
        if active && owns_pending {
            transaction.settle_proxy_issuance(run.id).await?;
            self.proxy_audit(&mut transaction, run, "issuance_confirmed")
                .await?;
        }
        transaction.commit().await?;
        if !active {
            // An earlier revoke may have observed absence before this request
            // created the key. issuance_pending forbids that observation from
            // suppressing this compensating delete or crash-recovery retries.
            self.revoke_run_proxy_key(run).await?;
            if owns_pending {
                self.settle_owned_proxy_issuance(run, "issuance_settled_after_revoke")
                    .await?;
            }
            return Err(credential_error());
        }
        Ok((key, alias))
    }

    /// Caller holds this Run's issuance lock and owns the only pending request.
    /// Never used after an ambiguous key request or to settle a pre-crash attempt.
    async fn settle_owned_proxy_issuance(
        &self,
        run: &RunProjection,
        state: &str,
    ) -> Result<(), CoreError> {
        let mut transaction = self.store.begin().await?;
        transaction
            .lock_project(run.project_id)
            .await?
            .ok_or_else(credential_error)?;
        transaction.settle_proxy_issuance(run.id).await?;
        self.proxy_audit(&mut transaction, run, state).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) fn open_proxy_key(
        &self,
        run: &RunProjection,
        sealed: &serde_json::Value,
    ) -> Result<SecretBytes, CoreError> {
        let sealed: SealedSecret =
            serde_json::from_value(sealed.clone()).map_err(|_| credential_error())?;
        self.secret_store
            .as_ref()
            .ok_or_else(credential_error)?
            .open(
                &sealed,
                &SecretScope {
                    secret_id: run.id,
                    project_id: run.project_id.as_uuid(),
                    version: 1,
                    purpose: proxy_purpose(run),
                },
            )
            .map_err(|_| credential_error())
    }

    /// Idempotent and retryable after broker failure; never unseals upstream auth.
    pub(crate) async fn revoke_run_proxy_key(&self, run: &RunProjection) -> Result<(), CoreError> {
        let mut transaction = self.store.begin().await?;
        let _ = transaction
            .lock_project(run.project_id)
            .await?
            .ok_or_else(credential_error)?;
        let Some(saved) = transaction.run_proxy_key(run.id).await? else {
            return Ok(());
        };
        if saved.revoked && !saved.issuance_pending {
            return Ok(());
        }
        transaction.defer_proxy_revocation(run.id).await?;
        self.proxy_audit(&mut transaction, run, "revocation_retry_scheduled")
            .await?;
        transaction.commit().await?;
        let proxy = self.inference.as_ref().ok_or_else(credential_error)?;
        let spec: RunKeySpec =
            serde_json::from_value(saved.spec).map_err(|_| credential_error())?;
        let hash = match saved.key_hash {
            Some(hash) => hash,
            None => {
                use sha2::Digest;
                let key = self.open_proxy_key(run, &saved.sealed_key)?;
                sha2::Sha256::digest(key.expose())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            }
        };
        let usage = proxy
            .admin
            .usage(&spec, &hash, OffsetDateTime::now_utc())
            .await
            .ok();
        proxy
            .admin
            .revoke(&spec, &hash)
            .await
            .map_err(|_| credential_error())?;
        let usage = usage
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| credential_error())?;
        let mut transaction = self.store.begin().await?;
        let _ = transaction
            .lock_project(run.project_id)
            .await?
            .ok_or_else(credential_error)?;
        let previous = transaction
            .run_proxy_key(run.id)
            .await?
            .ok_or_else(credential_error)?;
        transaction
            .mark_proxy_key(run.id, &hash, true, usage.as_ref())
            .await?;
        if !previous.revoked {
            self.proxy_audit(&mut transaction, run, "revoked").await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn proxy_audit(
        &self,
        transaction: &mut forge_storage::StorageTransaction<'_>,
        run: &RunProjection,
        state: &str,
    ) -> Result<(), CoreError> {
        let project = transaction
            .lock_project(run.project_id)
            .await?
            .ok_or_else(credential_error)?;
        transaction
            .append_event_and_outbox(&crate::event::event(
                project.id(),
                forge_domain::AggregateRef::Project(project.id()),
                project.revision(),
                forge_domain::DomainEventKind::RunProxyKeyChanged,
                self.actors.core,
                forge_domain::CommandId::new(),
                None,
                crate::event::event_payload([
                    ("run_id", serde_json::json!(run.id)),
                    ("state", serde_json::json!(state)),
                ]),
                forge_domain::Timestamp::now_utc(),
            )?)
            .await?;
        Ok(())
    }
}
fn proxy_purpose(run: &RunProjection) -> String {
    format!("run_proxy_key:{}:{}", run.id, run.environment_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn issuance_lock_serializes_only_the_same_run_and_releases_unused_entries() {
        let endpoint = "http://127.0.0.1:1";
        let proxy = InferenceProxy {
            admin: LiteLlmClient::new(
                endpoint,
                &SecretBytes::new(b"sk-synthetic-admin-not-real".to_vec()),
            )
            .expect("local endpoint"),
            endpoint: endpoint.into(),
            http: reqwest::Client::new(),
            issuance_locks: Mutex::new(HashMap::new()),
        };
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        let held = proxy.lock_issuance(first).await;
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                proxy.lock_issuance(first)
            )
            .await
            .is_err()
        );
        let independent = proxy.lock_issuance(second).await;
        drop(held);
        drop(independent);
        let _next = proxy.lock_issuance(first).await;
        assert_eq!(proxy.issuance_locks.lock().await.len(), 1);
    }
}
