//! Requires a disposable bucket provided by the integration harness. No bucket
//! or credential is inferred from a developer's unrelated S3 configuration.

use forge_domain::{
    ArtifactId, EvidenceLocation, EvidenceScope, EvidenceStream, ProjectId, TaskId,
    file_snapshot::{SnapshotFile, snapshot_object_key},
};
use forge_storage::evidence::{
    EvidenceError, EvidenceRedaction, EvidenceSpool, ObjectEvidenceStore, S3EvidenceConfig,
    SpoolLimits,
};
use object_store::{ObjectStoreExt, aws::AmazonS3Builder, path::Path};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn setting(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("required test setting {name} is missing"))
}

#[tokio::test]
#[ignore = "requires the local MinIO topology and a disposable evidence test bucket"]
async fn uploads_redacted_evidence_to_minio_and_reads_immutable_bytes_back() {
    let endpoint = setting("FORGE_MINIO_ENDPOINT");
    let bucket = setting("FORGE_MINIO_EVIDENCE_TEST_BUCKET");
    let access = setting("FORGE_MINIO_ROOT_USER");
    let secret = setting("FORGE_MINIO_ROOT_PASSWORD");
    assert!(
        bucket.starts_with("forge-it-"),
        "requires an explicit disposable forge-it- bucket"
    );
    let uploader = ObjectEvidenceStore::s3(S3EvidenceConfig {
        endpoint: endpoint.clone(),
        bucket: bucket.clone(),
        region: "us-east-1".to_owned(),
        access_key_id: access.clone(),
        secret_access_key: secret.clone(),
        allow_http: true,
    })
    .expect("valid local MinIO config");
    assert!(
        uploader.healthcheck().await,
        "configured bucket must be ready"
    );
    let denied = ObjectEvidenceStore::s3(S3EvidenceConfig {
        endpoint: endpoint.clone(),
        bucket: bucket.clone(),
        region: "us-east-1".to_owned(),
        access_key_id: "synthetic-invalid-access".into(),
        secret_access_key: "synthetic-invalid-secret".into(),
        allow_http: true,
    })
    .expect("syntactically valid but unauthorized config");
    assert!(
        !denied.healthcheck().await,
        "readiness must not accept rejected credentials"
    );
    let store = AmazonS3Builder::new()
        .with_endpoint(endpoint)
        .with_bucket_name(bucket)
        .with_region("us-east-1")
        .with_access_key_id(access)
        .with_secret_access_key(secret)
        .with_allow_http(true)
        .build()
        .expect("MinIO reader");
    let directory = tempfile::tempdir().expect("test spool");
    let spool = EvidenceSpool::open(directory.path().join("spool"), SpoolLimits::default())
        .expect("open spool");
    let scope = EvidenceScope {
        project_id: ProjectId::new(),
        task_id: Some(TaskId::new()),
        run_id: Uuid::now_v7(),
    };
    let redaction = EvidenceRedaction::new(
        "minio-test/v1".to_owned(),
        vec![b"synthetic-split-secret".to_vec()],
    )
    .expect("policy");
    let mut collector = spool
        .start_stream(scope, EvidenceStream::Stdout, redaction)
        .expect("collector");
    collector.push(b"result synthetic-split-");
    collector.push(b"secret complete");
    let chunks = collector.finish().chunks;
    assert_eq!(chunks.len(), 1);
    let pending = &chunks[0];
    let stored = uploader.upload(pending).await.expect("upload to MinIO");
    let key = Path::parse(stored.object_key()).expect("scoped key");
    let retry = uploader.upload(pending).await;
    let read = store.get(&key).await;
    let bytes = match read {
        Ok(object) => object.bytes().await.map_err(|_| "body read failed"),
        Err(_) => Err("object read failed"),
    };
    // Only this uniquely scoped test object is deleted; the harness owns the
    // disposable bucket cleanup if an assertion fails after provisioning.
    store.delete(&key).await.expect("delete exact test object");
    assert_eq!(retry.expect("idempotent upload"), stored);
    assert!(matches!(
        stored.data().location,
        EvidenceLocation::Stored { .. }
    ));
    assert_eq!(
        pending.receipt().data().location,
        EvidenceLocation::PendingUpload
    );
    assert_eq!(
        bytes.expect("stored evidence bytes").as_ref(),
        b"result [redacted] complete"
    );
}

#[tokio::test]
#[ignore = "requires the local MinIO topology and a disposable evidence test bucket"]
async fn snapshot_files_roundtrip_empty_and_binary_bytes_with_create_only_replay() {
    let endpoint = setting("FORGE_MINIO_ENDPOINT");
    let bucket = setting("FORGE_MINIO_EVIDENCE_TEST_BUCKET");
    let access = setting("FORGE_MINIO_ROOT_USER");
    let secret = setting("FORGE_MINIO_ROOT_PASSWORD");
    assert!(
        bucket.starts_with("forge-it-"),
        "requires a disposable forge-it- bucket"
    );
    let uploader = ObjectEvidenceStore::s3(S3EvidenceConfig {
        endpoint: endpoint.clone(),
        bucket: bucket.clone(),
        region: "us-east-1".into(),
        access_key_id: access.clone(),
        secret_access_key: secret.clone(),
        allow_http: true,
    })
    .expect("valid local MinIO config");
    let reader = AmazonS3Builder::new()
        .with_endpoint(endpoint)
        .with_bucket_name(bucket)
        .with_region("us-east-1")
        .with_access_key_id(access)
        .with_secret_access_key(secret)
        .with_allow_http(true)
        .build()
        .expect("MinIO snapshot reader");
    assert!(
        uploader.healthcheck().await,
        "configured disposable bucket must be ready"
    );
    let project = ProjectId::new();
    let artifact = ArtifactId::new();
    let binary = [b"synthetic-file-secret\n".as_slice(), &[0, 255, 128, 0, 1]].concat();
    for (path, bytes) in [("empty.dat", Vec::new()), ("binary.dat", binary)] {
        let sha256 = snapshot_digest(&bytes);
        let file = SnapshotFile {
            path: path.into(),
            executable: false,
            size_bytes: bytes.len() as u64,
            object_key: snapshot_object_key(project, artifact, &sha256),
            sha256,
        };
        uploader
            .upload_snapshot_file(&file, bytes.clone())
            .await
            .expect("upload exact snapshot bytes");
        uploader
            .upload_snapshot_file(&file, bytes.clone())
            .await
            .expect("conditional-create identical replay");
        assert_eq!(
            uploader
                .read_snapshot_file(&file)
                .await
                .expect("snapshot read"),
            bytes
        );
        // Valid alternate bytes at the same key must encounter the existing
        // object and fail integrity validation, not overwrite it via a normal PUT.
        let changed_bytes = b"conflicting snapshot bytes".to_vec();
        let conflicting = SnapshotFile {
            size_bytes: changed_bytes.len() as u64,
            sha256: snapshot_digest(&changed_bytes),
            ..file.clone()
        };
        assert!(matches!(
            uploader
                .upload_snapshot_file(&conflicting, changed_bytes)
                .await,
            Err(EvidenceError::IntegrityMismatch)
        ));
        let key = Path::parse(&file.object_key).expect("scoped snapshot key");
        let raw = reader
            .get(&key)
            .await
            .expect("read actual MinIO object")
            .bytes()
            .await
            .expect("read actual MinIO bytes");
        assert_eq!(
            raw.as_ref(),
            bytes.as_slice(),
            "snapshot content must never be redacted or replaced"
        );
        uploader
            .upload_snapshot_file(&file, bytes)
            .await
            .expect("original replay still succeeds after rejected overwrite");
        // Only this test's project/artifact-scoped object; the harness owns the bucket.
        reader
            .delete(&key)
            .await
            .expect("delete exact snapshot test object");
    }
}

fn snapshot_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
