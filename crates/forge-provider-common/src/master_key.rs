use std::{fs::File, io::Read, path::Path};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::private_file::{
    create_private_directory, open_private, verify_directory, write_new_private,
};
use crate::{SecretBytes, SecretStore, SecretStoreError};

/// Source selection is explicit and permanently recorded during initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MasterKeySource {
    /// Persistent Linux Secret Service keyring, not an ephemeral kernel key.
    LinuxKeyring,
    OwnerOnlyFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MasterKeyBinding {
    pub format_version: u32,
    pub key_id: Uuid,
    pub source: MasterKeySource,
}

impl SecretStore {
    /// Installer-only operation. Existing metadata or keys are never replaced.
    /// A partial initialization remains visible and requires explicit repair.
    pub fn initialize(directory: &Path, source: MasterKeySource) -> Result<Self, SecretStoreError> {
        create_private_directory(directory)?;
        let binding = MasterKeyBinding {
            format_version: 1,
            key_id: Uuid::now_v7(),
            source,
        };
        let serialized =
            serde_json::to_vec(&binding).map_err(|_| SecretStoreError::InvalidMetadata)?;
        // Persist identity first. A crash must not choose a different provider/key.
        write_new_private(&directory.join("master-key-binding.json"), &serialized)?;
        let generated = SecretBytes::random(32)?;
        match source {
            MasterKeySource::OwnerOnlyFile => {
                write_new_private(&directory.join("master.key"), generated.expose())?
            }
            MasterKeySource::LinuxKeyring => {
                let entry = keyring_entry(&binding)?;
                match entry.get_secret() {
                    Err(keyring::Error::NoEntry) => {}
                    _ => return Err(SecretStoreError::KeyUnavailable),
                }
                entry
                    .set_secret(generated.expose())
                    .map_err(|_| SecretStoreError::KeyUnavailable)?;
            }
        }
        File::open(directory)?.sync_all()?;
        Ok(Self {
            binding,
            key: generated,
        })
    }

    /// Opens only the source/key already selected. No automatic fallback or login.
    pub fn load(directory: &Path) -> Result<Self, SecretStoreError> {
        verify_directory(directory)?;
        let mut bytes = Vec::new();
        open_private(&directory.join("master-key-binding.json"))?
            .take(4097)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(SecretStoreError::InvalidMetadata);
        }
        let binding: MasterKeyBinding =
            serde_json::from_slice(&bytes).map_err(|_| SecretStoreError::InvalidMetadata)?;
        if binding.format_version != 1 || binding.key_id.is_nil() {
            return Err(SecretStoreError::InvalidMetadata);
        }
        let key = match binding.source {
            MasterKeySource::OwnerOnlyFile => {
                let mut bytes = Zeroizing::new(Vec::new());
                open_private(&directory.join("master.key"))
                    .map_err(|error| match error {
                        SecretStoreError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                            SecretStoreError::KeyUnavailable
                        }
                        other => other,
                    })?
                    .take(33)
                    .read_to_end(&mut bytes)?;
                SecretBytes::new(std::mem::take(&mut bytes))
            }
            MasterKeySource::LinuxKeyring => SecretBytes::new(
                keyring_entry(&binding)?
                    .get_secret()
                    .map_err(|_| SecretStoreError::KeyUnavailable)?,
            ),
        };
        if key.expose().len() != 32 {
            return Err(SecretStoreError::KeyUnavailable);
        }
        Ok(Self { binding, key })
    }
}

fn keyring_entry(binding: &MasterKeyBinding) -> Result<keyring::Entry, SecretStoreError> {
    keyring::Entry::new("forge-secret-store", &binding.key_id.to_string())
        .map_err(|_| SecretStoreError::KeyUnavailable)
}
