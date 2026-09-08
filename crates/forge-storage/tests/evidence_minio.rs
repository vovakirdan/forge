//! Requires a disposable bucket provided by the integration harness. No bucket
//! or credential is inferred from a developer's unrelated S3 configuration.

use forge_domain::{EvidenceLocation, EvidenceScope, EvidenceStream, ProjectId, TaskId};
use forge_storage::evidence::{
    EvidenceRedaction, EvidenceSpool, ObjectEvidenceStore, S3EvidenceConfig, SpoolLimits,
};
use object_store::{ObjectStoreExt, aws::AmazonS3Builder, path::Path};
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
