//! Byte-preserving Artifact bodies share only the maintained object-store client
//! with logs. They never pass through EvidenceSpool or its redaction pipeline.

use std::time::Duration;

use forge_domain::file_snapshot::{MAX_SNAPSHOT_BYTES, SnapshotFile};
use object_store::{ObjectStoreExt, PutMode, PutOptions, path::Path};

use super::{EvidenceError, ObjectEvidenceStore, digest};

impl ObjectEvidenceStore {
    pub async fn upload_snapshot_file(
        &self,
        file: &SnapshotFile,
        bytes: Vec<u8>,
    ) -> Result<(), EvidenceError> {
        validate_bytes(file, &bytes)?;
        let key = snapshot_key(file)?;
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            self.store.put_opts(
                &key,
                bytes.into(),
                PutOptions {
                    mode: PutMode::Create,
                    ..PutOptions::default()
                },
            ),
        )
        .await
        .map_err(|_| EvidenceError::ObjectStoreUnavailable)?;
        match result {
            Ok(_) => Ok(()),
            Err(object_store::Error::AlreadyExists { .. }) => {
                self.read_snapshot_file(file).await.map(|_| ())
            }
            Err(_) => Err(EvidenceError::ObjectStoreUnavailable),
        }
    }

    pub async fn read_snapshot_file(&self, file: &SnapshotFile) -> Result<Vec<u8>, EvidenceError> {
        let key = snapshot_key(file)?;
        tokio::time::timeout(Duration::from_secs(15), async {
            let object = self
                .store
                .get(&key)
                .await
                .map_err(|_| EvidenceError::ObjectStoreUnavailable)?;
            if object.meta.size != file.size_bytes || object.meta.size > MAX_SNAPSHOT_BYTES {
                return Err(EvidenceError::IntegrityMismatch);
            }
            let bytes = object
                .bytes()
                .await
                .map_err(|_| EvidenceError::ObjectStoreUnavailable)?;
            validate_bytes(file, &bytes)?;
            Ok(bytes.to_vec())
        })
        .await
        .map_err(|_| EvidenceError::ObjectStoreUnavailable)?
    }
}

fn snapshot_key(file: &SnapshotFile) -> Result<Path, EvidenceError> {
    if !file.object_key.starts_with("file-snapshots/") || file.size_bytes > MAX_SNAPSHOT_BYTES {
        return Err(EvidenceError::InvalidMetadata);
    }
    Path::parse(&file.object_key).map_err(|_| EvidenceError::InvalidMetadata)
}

fn validate_bytes(file: &SnapshotFile, bytes: &[u8]) -> Result<(), EvidenceError> {
    if bytes.len() as u64 != file.size_bytes || digest(bytes) != file.sha256 {
        return Err(EvidenceError::IntegrityMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_domain::{ArtifactId, ProjectId, file_snapshot::snapshot_object_key};
    use std::sync::Arc;

    #[tokio::test]
    async fn binary_and_empty_snapshots_are_immutable_and_byte_exact() {
        let store =
            ObjectEvidenceStore::from_store(Arc::new(object_store::memory::InMemory::new()));
        for bytes in [Vec::new(), vec![0, 255, 1, 0]] {
            let sha256 = digest(&bytes);
            let file = SnapshotFile {
                path: "input.dat".into(),
                executable: false,
                size_bytes: bytes.len() as u64,
                object_key: snapshot_object_key(ProjectId::new(), ArtifactId::new(), &sha256),
                sha256,
            };
            store
                .upload_snapshot_file(&file, bytes.clone())
                .await
                .unwrap();
            store
                .upload_snapshot_file(&file, bytes.clone())
                .await
                .unwrap();
            assert_eq!(store.read_snapshot_file(&file).await.unwrap(), bytes);
            assert!(store.upload_snapshot_file(&file, vec![42]).await.is_err());
        }
    }
}
