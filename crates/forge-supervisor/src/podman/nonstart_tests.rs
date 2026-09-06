use super::*;
use crate::{journal::Journal, surface::private_directory};
use forge_protocol::supervisor::v1::{EnvironmentPresence, StopMode, StopRun, supervisor_to_core};

fn fixture() -> (SupervisorConfig, ProvisionRun, RunRegistry) {
    let root = std::env::temp_dir().join(format!("forge-nonstart-{}", crate::new_id()));
    private_directory(&root).unwrap();
    let mut config = SupervisorConfig::new(root.join("core.sock"), "fixture".into(), "boot".into());
    config.state_directory = root.clone();
    let registry =
        RunRegistry::new(Journal::open(&root.join("journal"), "fixture", 1024 * 1024).unwrap());
    let mut provision = crate::tests::provision();
    provision.run_spec_version = 2;
    provision.run_spec_json = json!({"surface_id":crate::new_id()}).to_string();
    (config, provision, registry)
}

async fn assert_terminal_proof(registry: &RunRegistry) {
    let journal = registry.journal.lock().await;
    let events: Vec<_> = journal
        .pending()
        .filter_map(|message| match &message.message {
            Some(supervisor_to_core::Message::ObservedRunEvent(event)) => Some(event),
            _ => None,
        })
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, RunEventKind::StartFailed as i32);
    assert_eq!(events[1].kind, RunEventKind::Stopped as i32);
    assert_eq!(events[1].sequence, events[0].sequence + 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&events[1].details_json).unwrap()["evidence_not_started"],
        true
    );
    assert_eq!(
        journal.inventory()[0].presence,
        EnvironmentPresence::Quiescent as i32
    );
}

#[tokio::test]
async fn invalid_new_spec_reports_failure_then_positive_quiescence() {
    let (config, provision, registry) = fixture();
    let control = registry
        .register(&provision, &config.boot_id)
        .await
        .unwrap()
        .unwrap();
    let result = PodmanBackend::new(&config)
        .run(provision, registry.clone(), control, false)
        .await;
    assert!(matches!(result, Err(SupervisorError::InvalidRunSpec)));
    assert_terminal_proof(&registry).await;
    assert!(!config.state_directory.join("surfaces").exists());
    std::fs::remove_dir_all(config.state_directory).unwrap();
}

#[tokio::test]
async fn invalid_adopted_spec_remains_unknown_without_nonstart_evidence() {
    let (config, provision, registry) = fixture();
    let control = registry
        .register(&provision, &config.boot_id)
        .await
        .unwrap()
        .unwrap();
    let result = PodmanBackend::new(&config)
        .run(provision, registry.clone(), control, true)
        .await;
    assert!(matches!(result, Err(SupervisorError::InvalidRunSpec)));
    let journal = registry.journal.lock().await;
    assert_eq!(
        journal.inventory()[0].presence,
        EnvironmentPresence::Unknown as i32
    );
    assert_eq!(journal.pending().count(), 0);
    std::fs::remove_dir_all(config.state_directory).unwrap();
}

#[tokio::test]
async fn full_journal_retains_nonstart_reporting_until_terminal_proof_is_durable() {
    let (config, provision, registry) = fixture();
    let _control = registry
        .register(&provision, &config.boot_id)
        .await
        .unwrap()
        .unwrap();
    registry.journal.lock().await.set_test_capacity(1);
    let backend = PodmanBackend::new(&config);
    let worker_registry = registry.clone();
    let worker_provision = provision.clone();
    let worker = tokio::spawn(async move {
        backend
            .finish_without_start(
                &worker_provision,
                &worker_registry,
                Some("runtime_image_preflight_failed"),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while registry.journal.lock().await.is_healthy() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!worker.is_finished());
    assert!(
        registry
            .request_stop(&StopRun {
                command_id: crate::new_id(),
                run_id: provision.run_id,
                lease_fencing_token: provision.lease_fencing_token,
                environment_epoch: provision.environment_epoch,
                mode: StopMode::Force as i32,
                grace_period_ms: 0,
                reason_code: "fixture_stop".into(),
            })
            .await
    );
    registry.journal.lock().await.set_test_capacity(1024 * 1024);
    tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_terminal_proof(&registry).await;
    std::fs::remove_dir_all(config.state_directory).unwrap();
}
