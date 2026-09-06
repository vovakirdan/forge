use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{SealedSecret, SecretBytes, SecretScope, SecretStore, SecretStoreError};

const MAX_AUTH_BYTES: usize = 1024 * 1024;

/// Encrypted canonical auth state. Session identity changes only on enrollment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedAuthSnapshot {
    pub binding_id: Uuid,
    pub session_id: Uuid,
    pub account_id: String,
    pub auth: SealedSecret,
}

/// Host-side record of exactly which auth version was materialized for a Run.
/// This is never accepted from the worker as authorization to update credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthSnapshotLease {
    pub run_id: Uuid,
    pub environment_epoch: u64,
    pub base: ManagedAuthSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthWritebackConflict {
    BindingChanged,
    SessionChanged,
    AccountChanged,
    ConcurrentRefresh,
    RefreshNotNewer,
    InvalidCandidate,
}

/// The caller must persist recovery evidence, or apply the replacement with CAS,
/// before cleaning the Run auth directory. A failed CAS becomes a conflict.
#[derive(Debug)]
pub enum AuthWriteback {
    Unchanged,
    Replace {
        expected_version: u64,
        snapshot: ManagedAuthSnapshot,
    },
    Conflict {
        reason: AuthWritebackConflict,
        recovery: SealedSecret,
    },
}

pub fn enroll_codex_auth(
    store: &SecretStore,
    binding_id: Uuid,
    project_id: Uuid,
    secret_id: Uuid,
    auth: &SecretBytes,
) -> Result<ManagedAuthSnapshot, SecretStoreError> {
    if binding_id.is_nil() {
        return Err(SecretStoreError::InvalidScope);
    }
    let identity = parse_auth(auth)?;
    let session_id = Uuid::now_v7();
    let scope = SecretScope {
        secret_id,
        project_id,
        version: 1,
        purpose: auth_purpose(binding_id, session_id),
    };
    Ok(ManagedAuthSnapshot {
        binding_id,
        session_id,
        account_id: identity.account_id.to_owned(),
        auth: store.seal(scope, auth)?,
    })
}

/// Decides writeback against the current row while Core holds its row lock.
/// Concurrent Run refreshes do not serialize execution and cannot overwrite one
/// another by wall-clock completion time. No OAuth requests occur here.
pub fn evaluate_auth_writeback(
    store: &SecretStore,
    current: &ManagedAuthSnapshot,
    lease: &AuthSnapshotLease,
    candidate: &SecretBytes,
) -> Result<AuthWriteback, SecretStoreError> {
    if lease.run_id.is_nil() || lease.environment_epoch == 0 {
        return Err(SecretStoreError::InvalidScope);
    }
    if candidate.expose().len() > MAX_AUTH_BYTES {
        return Err(SecretStoreError::AuthTooLarge);
    }
    let conflict = |reason| -> Result<AuthWriteback, SecretStoreError> {
        let scope = SecretScope {
            secret_id: Uuid::now_v7(),
            project_id: lease.base.auth.scope.project_id,
            version: 1,
            purpose: format!("codex_auth_recovery:{}", lease.base.binding_id),
        };
        Ok(AuthWriteback::Conflict {
            reason,
            recovery: store.seal(scope, candidate)?,
        })
    };
    let base = &lease.base;
    if current.binding_id != base.binding_id
        || current.auth.scope.project_id != base.auth.scope.project_id
        || current.auth.scope.secret_id != base.auth.scope.secret_id
    {
        return conflict(AuthWritebackConflict::BindingChanged);
    }
    if current.session_id != base.session_id {
        return conflict(AuthWritebackConflict::SessionChanged);
    }
    if current.account_id != base.account_id {
        return conflict(AuthWritebackConflict::AccountChanged);
    }
    let base_plaintext = open_snapshot(store, base)?;
    if base_plaintext.expose() == candidate.expose() {
        return Ok(AuthWriteback::Unchanged);
    }
    let current_plaintext = open_snapshot(store, current)?;
    if current_plaintext.expose() == candidate.expose() {
        return Ok(AuthWriteback::Unchanged);
    }
    let Ok(candidate_identity) = parse_auth(candidate) else {
        return conflict(AuthWritebackConflict::InvalidCandidate);
    };
    if candidate_identity.account_id != base.account_id {
        return conflict(AuthWritebackConflict::AccountChanged);
    }
    if current.auth.scope.version != base.auth.scope.version {
        return conflict(AuthWritebackConflict::ConcurrentRefresh);
    }
    let original = parse_auth(&base_plaintext)?;
    if candidate_identity.refreshed_at <= original.refreshed_at {
        return conflict(AuthWritebackConflict::RefreshNotNewer);
    }
    let expected_version = current.auth.scope.version;
    let mut scope = current.auth.scope.clone();
    scope.version = expected_version
        .checked_add(1)
        .ok_or(SecretStoreError::InvalidScope)?;
    let snapshot = ManagedAuthSnapshot {
        binding_id: current.binding_id,
        session_id: current.session_id,
        account_id: current.account_id.clone(),
        auth: store.seal(scope, candidate)?,
    };
    Ok(AuthWriteback::Replace {
        expected_version,
        snapshot,
    })
}

fn open_snapshot(
    store: &SecretStore,
    snapshot: &ManagedAuthSnapshot,
) -> Result<SecretBytes, SecretStoreError> {
    let mut expected = snapshot.auth.scope.clone();
    expected.purpose = auth_purpose(snapshot.binding_id, snapshot.session_id);
    let bytes = store.open(&snapshot.auth, &expected)?;
    if parse_auth(&bytes)?.account_id != snapshot.account_id {
        return Err(SecretStoreError::Authentication);
    }
    Ok(bytes)
}

fn auth_purpose(binding: Uuid, session: Uuid) -> String {
    format!("codex_auth:{binding}:{session}")
}

struct AuthIdentity<'a> {
    account_id: &'a str,
    refreshed_at: OffsetDateTime,
}

#[derive(Deserialize)]
struct AuthDocument<'a> {
    #[serde(borrow)]
    tokens: AuthTokens<'a>,
    last_refresh: &'a str,
    #[serde(default)]
    auth_mode: Option<&'a str>,
    #[serde(default, rename = "OPENAI_API_KEY")]
    api_key: Option<&'a str>,
}

#[derive(Deserialize)]
struct AuthTokens<'a> {
    account_id: &'a str,
    access_token: &'a str,
    refresh_token: &'a str,
    id_token: &'a str,
}

fn parse_auth(bytes: &SecretBytes) -> Result<AuthIdentity<'_>, SecretStoreError> {
    if bytes.expose().len() > MAX_AUTH_BYTES {
        return Err(SecretStoreError::AuthTooLarge);
    }
    // Borrow token text so deserialization does not leave unzeroized token copies.
    let document: AuthDocument<'_> =
        serde_json::from_slice(bytes.expose()).map_err(|_| SecretStoreError::InvalidAuth)?;
    if document.api_key.is_some()
        || document.auth_mode.is_some_and(|mode| mode != "chatgpt")
        || [
            &document.tokens.account_id,
            &document.tokens.access_token,
            &document.tokens.refresh_token,
            &document.tokens.id_token,
        ]
        .iter()
        .any(|value| value.is_empty())
    {
        return Err(SecretStoreError::InvalidAuth);
    }
    let refreshed_at = OffsetDateTime::parse(document.last_refresh, &Rfc3339)
        .map_err(|_| SecretStoreError::InvalidAuth)?;
    Ok(AuthIdentity {
        account_id: document.tokens.account_id,
        refreshed_at,
    })
}
