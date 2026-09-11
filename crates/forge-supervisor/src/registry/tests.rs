use super::*;
use forge_protocol::supervisor::v1::EnvironmentPresence;
use std::{fs, path::PathBuf, time::Duration};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("forge-stopped-journal-{}", crate::new_id())))
    }

    fn open(&self) -> Journal {
        Journal::open(&self.0, "stopped-test", 1024 * 1024).unwrap()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn provision() -> ProvisionRun {
    let mut provision = crate::tests::provision();
    provision.run_spec_version = 2;
    // Admission needs only the surface identity; no runtime is launched here.
    provision.run_spec_json = serde_json::json!({"surface_id":crate::new_id()}).to_string();
    provision
}

fn successor(previous: &ProvisionRun) -> ProvisionRun {
    let mut successor = previous.clone();
    successor.run_id = crate::new_id();
    successor.command_id = crate::new_id();
    successor.context_snapshot_id = crate::new_id();
    successor.attempt += 1;
    successor.lease_fencing_token += 1;
    successor
}

#[tokio::test]
async fn stopped_notification_already_contains_durable_quiescence_before_finish() {
    let directory = Directory::new();
    let registry = RunRegistry::new(directory.open());
    let provision = provision();
    registry.register(&provision, "boot").await.unwrap();
    {
        let notified = registry.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        registry
            .emit(&provision, RunEventKind::Stopped, serde_json::json!({}))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), notified)
            .await
            .unwrap();
    }
    {
        let journal = registry.journal.lock().await;
        let record = &journal.records()[0];
        assert_eq!(record.presence, EnvironmentPresence::Quiescent);
        assert!(!record.quiescence_confirmed, "Core ACK is still separate");
        assert_eq!(journal.pending().count(), 1);
    }
    assert!(
        registry
            .active
            .lock()
            .await
            .contains_key(&scope_key(&provision))
    );
    assert!(
        registry
            .register(&successor(&provision), "boot")
            .await
            .unwrap()
            .is_some(),
        "handoff must not depend on the previous worker calling finish"
    );
    drop(registry);
    let journal = directory.open();
    let records = journal.records();
    let stopped = records
        .iter()
        .find(|record| record.provision.run_id == provision.run_id)
        .unwrap();
    assert_eq!(stopped.presence, EnvironmentPresence::Quiescent);
    assert_eq!(
        stopped.pending.len(),
        1,
        "the exact envelope survives restart"
    );
}

#[tokio::test]
async fn failed_stopped_persistence_does_not_release_the_previous_surface() {
    let directory = Directory::new();
    let registry = RunRegistry::new(directory.open());
    let provision = provision();
    registry.register(&provision, "boot").await.unwrap();
    registry.journal.lock().await.set_test_capacity(1);
    assert!(matches!(
        registry
            .emit(&provision, RunEventKind::Stopped, serde_json::json!({}))
            .await,
        Err(SupervisorError::JournalFull)
    ));
    {
        let journal = registry.journal.lock().await;
        let record = &journal.records()[0];
        assert_eq!(record.presence, EnvironmentPresence::Active);
        assert_eq!(record.last_sequence, 0);
        assert_eq!(journal.pending().count(), 0);
    }
    assert!(matches!(
        registry.register(&successor(&provision), "boot").await,
        Err(SupervisorError::ConflictingProvision)
    ));
}

#[tokio::test]
async fn failure_observations_without_stopped_do_not_release_the_surface() {
    for kind in [
        RunEventKind::StartFailed,
        RunEventKind::ProviderFailed,
        RunEventKind::Stopping,
    ] {
        let directory = Directory::new();
        let registry = RunRegistry::new(directory.open());
        let provision = provision();
        registry.register(&provision, "boot").await.unwrap();
        registry
            .emit(&provision, kind, serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(
            registry.journal.lock().await.records()[0].presence,
            EnvironmentPresence::Active
        );
        assert!(matches!(
            registry.register(&successor(&provision), "boot").await,
            Err(SupervisorError::ConflictingProvision)
        ));
    }
}
