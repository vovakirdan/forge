use std::fmt;

use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::MasterKeyBinding;

/// Plaintext with explicit access, redacted diagnostics, and zeroization on drop.
/// It intentionally cannot be serialized into a domain payload.
///
/// ```compile_fail
/// let secret = forge_provider_common::SecretBytes::new(b"synthetic".to_vec());
/// let payload = serde_json::to_value(&secret);
/// ```
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
    pub fn random(length: usize) -> Result<Self, SecretStoreError> {
        let mut secret = Self::new(vec![0; length]);
        getrandom::fill(&mut secret.0).map_err(|_| SecretStoreError::RandomUnavailable)?;
        Ok(secret)
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretBytes([REDACTED])")
    }
}

/// Associated data prevents moving a ciphertext to another project or version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretScope {
    pub secret_id: Uuid,
    pub project_id: Uuid,
    pub version: u64,
    pub purpose: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedSecret {
    pub format_version: u32,
    pub master_key_id: Uuid,
    pub scope: SecretScope,
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
}

impl fmt::Debug for SealedSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedSecret")
            .field("master_key_id", &self.master_key_id)
            .field("scope", &self.scope)
            .field("ciphertext", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("secret store I/O failed")]
    Io(#[from] std::io::Error),
    #[error("secret store metadata is malformed")]
    InvalidMetadata,
    #[error("secret store key is unavailable; restore the original key")]
    KeyUnavailable,
    #[error("secret store path must be owner-only, non-symlink, and owned by this user")]
    UnsafePermissions,
    #[error("secret authentication failed or its scope/key does not match")]
    Authentication,
    #[error("secret scope is invalid")]
    InvalidScope,
    #[error("operating system random source failed")]
    RandomUnavailable,
    #[error("managed auth must contain a valid ChatGPT account and refresh timestamp")]
    InvalidAuth,
    #[error("managed auth exceeds the maximum supported size")]
    AuthTooLarge,
}

/// Encrypts records; PostgreSQL and canonical version updates belong to Core.
pub struct SecretStore {
    pub(crate) binding: MasterKeyBinding,
    pub(crate) key: SecretBytes,
    pub(crate) directory: std::path::PathBuf,
}

impl fmt::Debug for SecretStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretStore")
            .field("binding", &self.binding)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl SecretStore {
    /// Host-side import guards must exclude all files in the configured store,
    /// including installations outside the normal execution directory.
    pub fn contains_path(&self, path: &std::path::Path) -> bool {
        path.starts_with(&self.directory)
    }

    pub fn binding(&self) -> &MasterKeyBinding {
        &self.binding
    }

    pub fn seal(
        &self,
        scope: SecretScope,
        plaintext: &SecretBytes,
    ) -> Result<SealedSecret, SecretStoreError> {
        validate_scope(&scope)?;
        let mut nonce = [0; 24];
        getrandom::fill(&mut nonce).map_err(|_| SecretStoreError::RandomUnavailable)?;
        let aad = associated_data(self.binding.key_id, &scope)?;
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.expose())
            .map_err(|_| SecretStoreError::KeyUnavailable)?;
        let ciphertext = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext.expose(),
                    aad: &aad,
                },
            )
            .map_err(|_| SecretStoreError::Authentication)?;
        Ok(SealedSecret {
            format_version: 1,
            master_key_id: self.binding.key_id,
            scope,
            nonce,
            ciphertext,
        })
    }

    pub fn open(
        &self,
        record: &SealedSecret,
        expected_scope: &SecretScope,
    ) -> Result<SecretBytes, SecretStoreError> {
        if record.format_version != 1
            || record.master_key_id != self.binding.key_id
            || &record.scope != expected_scope
        {
            return Err(SecretStoreError::Authentication);
        }
        validate_scope(expected_scope)?;
        let aad = associated_data(record.master_key_id, expected_scope)?;
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.expose())
            .map_err(|_| SecretStoreError::KeyUnavailable)?;
        let bytes = cipher
            .decrypt(
                &XNonce::from(record.nonce),
                Payload {
                    msg: &record.ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| SecretStoreError::Authentication)?;
        Ok(SecretBytes::new(bytes))
    }
}

fn validate_scope(scope: &SecretScope) -> Result<(), SecretStoreError> {
    if scope.secret_id.is_nil()
        || scope.project_id.is_nil()
        || scope.version == 0
        || scope.purpose.trim().is_empty()
    {
        return Err(SecretStoreError::InvalidScope);
    }
    Ok(())
}

fn associated_data(key_id: Uuid, scope: &SecretScope) -> Result<Vec<u8>, SecretStoreError> {
    serde_json::to_vec(&("forge-secret-store", 1_u32, key_id, scope))
        .map_err(|_| SecretStoreError::InvalidMetadata)
}
