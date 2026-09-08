use std::sync::Arc;

use forge_domain::{EvidenceLocation, EvidenceScope, EvidenceStream, ProjectId, TaskId};
use object_store::{ObjectStoreExt, memory::InMemory, path::Path};
use uuid::Uuid;

use super::*;

fn scope() -> EvidenceScope {
    EvidenceScope {
        project_id: ProjectId::new(),
        task_id: Some(TaskId::new()),
        run_id: Uuid::now_v7(),
    }
}

fn redaction() -> EvidenceRedaction {
    EvidenceRedaction::new(
        "exact-literals/v1".to_owned(),
        vec![b"synthetic-secret".to_vec()],
    )
    .expect("valid synthetic policy")
}

fn setup(limits: SpoolLimits) -> (tempfile::TempDir, EvidenceSpool) {
    let root = tempfile::tempdir().expect("temporary directory");
    let spool = EvidenceSpool::open(root.path().join("spool"), limits).expect("safe spool");
    (root, spool)
}

#[test]
fn redacts_secrets_across_every_read_and_object_boundary() {
    let bytes = b"before synthetic-secret after";
    for split in 0..=bytes.len() {
        let (_root, spool) = setup(SpoolLimits {
            chunk_bytes: 5,
            ..SpoolLimits::default()
        });
        let mut collector = spool
            .start_stream(scope(), EvidenceStream::Stdout, redaction())
            .expect("collector");
        let mut chunks = collector.push(&bytes[..split]).chunks;
        chunks.extend(collector.push(&bytes[split..]).chunks);
        chunks.extend(collector.finish().chunks);
        let mut result = Vec::new();
        for chunk in chunks {
            result.extend(chunk.read_verified().expect("redacted chunk"));
        }
        assert_eq!(result, b"before [redacted] after", "split {split}");
    }
}

#[test]
fn overlapping_secret_prefix_waits_for_longer_secret() {
    let mut redact = EvidenceRedaction::new(
        "test/v1".to_owned(),
        vec![b"synthetic".to_vec(), b"synthetic-secret".to_vec()],
    )
    .expect("policy");
    assert!(redact.push(b"synthetic-", false).is_empty());
    assert_eq!(redact.push(b"secret", true), b"[redacted]");
}

#[test]
fn overflow_signals_stop_once_and_keeps_consuming_without_growing_spool() {
    let (_root, spool) = setup(SpoolLimits {
        per_run_bytes: 1100,
        global_bytes: 2000,
        chunk_bytes: 256,
    });
    let mut collector = spool
        .start_stream(scope(), EvidenceStream::Stderr, redaction())
        .expect("collector");
    let first = collector.push(&vec![b'x'; 4096]);
    assert!(first.evidence_incomplete && first.stop_required);
    assert_eq!(first.incident, Some(SpoolIncident::CapacityExhausted));
    assert_eq!(first.consumed_bytes, 4096);
    let used = spool.used_bytes().expect("usage");
    for _ in 0..8 {
        let drained = collector.push(&vec![b'x'; 4096]);
        assert!(drained.chunks.is_empty());
        assert_eq!(drained.consumed_bytes, 4096);
    }
    assert_eq!(spool.used_bytes().expect("usage"), used);
    assert!(used <= 1100);
}

#[test]
fn global_cap_applies_to_independent_runs_and_survives_restart() {
    let limits = SpoolLimits {
        per_run_bytes: 2000,
        global_bytes: 2000,
        chunk_bytes: 256,
    };
    let (root, spool) = setup(limits);
    for _ in 0..3 {
        let mut collector = spool
            .start_stream(scope(), EvidenceStream::Stdout, redaction())
            .expect("collector");
        collector.push(&[b'x'; 256]);
    }
    let used = spool.used_bytes().expect("usage");
    assert!(used <= 2000);
    drop(spool);
    let recovered = EvidenceSpool::open(root.path().join("spool"), limits).expect("recover spool");
    assert_eq!(recovered.used_bytes().expect("usage"), used);
    let mut collector = recovered
        .start_stream(scope(), EvidenceStream::Stdout, redaction())
        .expect("collector");
    assert!(collector.push(&[b'x'; 256]).stop_required);
    assert!(!recovered.pending().expect("pending receipts").is_empty());
}

#[test]
fn second_spool_owner_and_symlink_roots_are_rejected() {
    let (root, _spool) = setup(SpoolLimits::default());
    assert!(matches!(
        EvidenceSpool::open(root.path().join("spool"), SpoolLimits::default()),
        Err(EvidenceError::SpoolBusy)
    ));
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(root.path().join("spool"), &alias).expect("test symlink");
    assert!(matches!(
        EvidenceSpool::open(alias, SpoolLimits::default()),
        Err(EvidenceError::UnsafeSpool)
    ));
}

#[tokio::test]
async fn upload_creates_immutable_object_and_preserves_pending_receipt() {
    let (_root, spool) = setup(SpoolLimits::default());
    let mut collector = spool
        .start_stream(scope(), EvidenceStream::Stdout, redaction())
        .expect("collector");
    collector.push(b"before synthetic-secret after");
    let chunks = collector.finish().chunks;
    let pending = &chunks[0];
    let store = Arc::new(InMemory::new());
    let uploader = ObjectEvidenceStore::from_store(store.clone());
    let stored = uploader.upload(pending).await.expect("upload");
    assert!(matches!(
        stored.data().location,
        EvidenceLocation::Stored { .. }
    ));
    assert_eq!(
        pending.receipt().data().location,
        EvidenceLocation::PendingUpload
    );
    assert_eq!(
        uploader.upload(pending).await.expect("idempotent retry"),
        stored
    );
    let key = Path::parse(stored.object_key()).expect("scoped key");
    assert_eq!(
        store
            .get(&key)
            .await
            .expect("object")
            .bytes()
            .await
            .expect("bytes"),
        b"before [redacted] after".as_slice()
    );
}

#[tokio::test]
async fn upload_rejects_existing_object_with_different_bytes() {
    let (_root, spool) = setup(SpoolLimits::default());
    let mut collector = spool
        .start_stream(scope(), EvidenceStream::Stdout, redaction())
        .expect("collector");
    collector.push(b"good");
    let chunks = collector.finish().chunks;
    let pending = &chunks[0];
    let store = Arc::new(InMemory::new());
    let key = Path::parse(pending.receipt().object_key()).expect("scoped key");
    store
        .put(&key, b"evil".as_slice().into())
        .await
        .expect("conflicting test object");
    let uploader = ObjectEvidenceStore::from_store(store);
    assert!(matches!(
        uploader.upload(pending).await,
        Err(EvidenceError::IntegrityMismatch)
    ));
}

#[tokio::test]
async fn missing_spool_body_stays_pending_and_never_becomes_stored() {
    let (root, spool) = setup(SpoolLimits::default());
    let mut collector = spool
        .start_stream(scope(), EvidenceStream::Stdout, redaction())
        .expect("collector");
    collector.push(b"body");
    let chunks = collector.finish().chunks;
    let pending = &chunks[0];
    let path = root
        .path()
        .join("spool")
        .join(pending.receipt().data().scope.run_id.to_string())
        .join(format!("{}.chunk", pending.receipt().data().id));
    std::fs::rename(&path, path.with_extension("moved")).expect("simulate lost body");
    let uploader = ObjectEvidenceStore::from_store(Arc::new(InMemory::new()));
    assert!(matches!(
        uploader.upload(pending).await,
        Err(EvidenceError::SpoolIo)
    ));
    assert_eq!(
        pending.receipt().data().location,
        EvidenceLocation::PendingUpload
    );
}

#[tokio::test]
async fn postcommit_release_frees_local_capacity_and_preserves_remote_evidence() {
    let (_root, spool) = setup(SpoolLimits::default());
    let mut collector = spool
        .start_stream(scope(), EvidenceStream::Stdout, redaction())
        .expect("collector");
    collector.push(b"retained remotely");
    let chunks = collector.finish().chunks;
    let pending = &chunks[0];
    let store = Arc::new(InMemory::new());
    let uploader = ObjectEvidenceStore::from_store(store.clone());
    assert!(spool.release_uploaded(pending, pending.receipt()).is_err());
    let stored = uploader.upload(pending).await.expect("upload");
    assert!(spool.used_bytes().expect("usage") > 0);
    spool
        .release_uploaded(pending, &stored)
        .expect("release local duplicate");
    spool
        .release_uploaded(pending, &stored)
        .expect("repeat release");
    assert_eq!(spool.used_bytes().expect("usage"), 0);
    assert!(spool.pending().expect("pending").is_empty());
    let key = Path::parse(stored.object_key()).expect("key");
    assert_eq!(
        store
            .get(&key)
            .await
            .expect("retained object")
            .bytes()
            .await
            .expect("bytes")
            .as_ref(),
        b"retained remotely"
    );
}
