use std::{fs, path::PathBuf};

use forge_protocol::supervisor::v1::{
    AcknowledgementDisposition, CoreAcknowledgement, EnvironmentPresence, ObservedRunEvent,
    ProvisionRun, RunEventKind, SupervisorToCore, supervisor_to_core,
};

use crate::{SupervisorConfig, SupervisorError, hello_for, journal::Journal, new_id};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("forge-supervisor-{}", new_id())))
    }

    fn open(&self) -> Journal {
        Journal::open(&self.0, "test-host", 1024 * 1024).unwrap()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn provision() -> ProvisionRun {
    ProvisionRun {
        command_id: new_id(),
        run_id: new_id(),
        task_id: new_id(),
        employee_id: new_id(),
        stage_id: "work".into(),
        attempt: 1,
        lease_fencing_token: 42,
        environment_epoch: 7,
        context_snapshot_id: new_id(),
        run_spec_json: r#"{"stage_outcome":{"outcome":"passed"}}"#.into(),
        run_spec_version: 1,
        traceparent: String::new(),
        assignment: None,
    }
}

fn observation(provision: &ProvisionRun, sequence: u64) -> SupervisorToCore {
    SupervisorToCore {
        message: Some(supervisor_to_core::Message::ObservedRunEvent(
            ObservedRunEvent {
                message_id: new_id(),
                run_id: provision.run_id.clone(),
                lease_fencing_token: provision.lease_fencing_token,
                environment_epoch: provision.environment_epoch,
                sequence,
                occurred_at_unix_ms: 1,
                kind: RunEventKind::Running as i32,
                details_json: "{}".into(),
            },
        )),
    }
}

#[test]
fn reconnect_hellos_retain_the_process_identity() {
    let config = SupervisorConfig::new(
        PathBuf::from("/tmp/forge-supervisor-test.sock"),
        "test-host".into(),
        "test-boot".into(),
    );
    let first = hello_for(&config);
    let second = hello_for(&config);
    assert_eq!(first.supervisor_instance_id, second.supervisor_instance_id);
    assert_eq!(first.started_at_unix_ms, second.started_at_unix_ms);
    assert_ne!(first.message_id, second.message_id);
}

#[test]
fn duplicate_provision_does_not_restart_active_or_finished_work() {
    let directory = TestDirectory::new();
    let mut journal = directory.open();
    let mut provision = provision();
    assert!(journal.register(&provision, "boot-1").unwrap());
    provision.command_id = new_id();
    provision.traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into();
    assert!(!journal.register(&provision, "boot-1").unwrap());
    journal.finish(&provision, true).unwrap();
    assert!(!journal.register(&provision, "boot-1").unwrap());
}

#[test]
fn old_journal_provision_without_trace_context_still_decodes() {
    let mut value = serde_json::to_value(provision()).unwrap();
    value.as_object_mut().unwrap().remove("traceparent");
    let restored: ProvisionRun = serde_json::from_value(value).unwrap();
    assert!(restored.traceparent.is_empty());
}

#[test]
fn conflicting_provision_never_replaces_active_execution() {
    let directory = TestDirectory::new();
    let mut journal = directory.open();
    let provision = provision();
    journal.register(&provision, "boot-1").unwrap();
    let mut changed = provision.clone();
    changed.environment_epoch += 1;
    assert!(matches!(
        journal.register(&changed, "boot-1"),
        Err(SupervisorError::ConflictingProvision)
    ));
    assert_eq!(
        journal.inventory()[0].environment_epoch,
        provision.environment_epoch
    );
}

#[test]
fn same_scope_with_changed_spec_is_rejected() {
    let directory = TestDirectory::new();
    let mut journal = directory.open();
    let mut provision = provision();
    journal.register(&provision, "boot-1").unwrap();
    provision.run_spec_json = "{}".into();
    assert!(matches!(
        journal.register(&provision, "boot-1"),
        Err(SupervisorError::ConflictingProvision)
    ));
}

#[test]
fn restart_preserves_exact_unacknowledged_envelope_and_sequence() {
    let directory = TestDirectory::new();
    let provision = provision();
    let message = observation(&provision, 1);
    {
        let mut journal = directory.open();
        journal.register(&provision, "boot-1").unwrap();
        journal.append(message.clone()).unwrap();
    }
    let mut reopened = directory.open();
    assert_eq!(reopened.pending().collect::<Vec<_>>(), vec![&message]);
    let entry = &reopened.inventory()[0];
    assert_eq!(entry.last_sequence, 1);
    assert_eq!(entry.presence, EnvironmentPresence::Unknown as i32);
    assert_eq!(entry.provision_boot_id, "boot-1");
    assert!(!reopened.register(&provision, "boot-2").unwrap());
}

#[test]
fn acknowledgement_compacts_pending_but_keeps_execution_tombstone() {
    let directory = TestDirectory::new();
    let mut journal = directory.open();
    let provision = provision();
    journal.register(&provision, "boot-1").unwrap();
    let message = observation(&provision, 1);
    journal.append(message.clone()).unwrap();
    journal
        .acknowledge(&CoreAcknowledgement {
            message_id: new_id(),
            acknowledged_message_id: crate::journal::message_id(&message).unwrap().clone(),
            disposition: AcknowledgementDisposition::Accepted as i32,
            reason_code: String::new(),
            message: String::new(),
        })
        .unwrap();
    assert_eq!(journal.pending().count(), 0);
    assert_eq!(journal.inventory()[0].last_sequence, 1);
    assert!(!journal.register(&provision, "boot-1").unwrap());
}

#[test]
fn full_journal_rejects_append_without_advancing_sequence() {
    let directory = TestDirectory::new();
    let mut journal = Journal::open(&directory.0, "test-host", 1600).unwrap();
    let provision = provision();
    journal.register(&provision, "boot-1").unwrap();
    let mut message = observation(&provision, 1);
    if let Some(supervisor_to_core::Message::ObservedRunEvent(event)) = message.message.as_mut() {
        event.details_json = "x".repeat(1600);
    }
    assert!(matches!(
        journal.append(message),
        Err(SupervisorError::JournalFull)
    ));
    assert_eq!(journal.inventory()[0].last_sequence, 0);
    assert_eq!(journal.pending().count(), 0);
}

#[test]
fn journal_lock_excludes_second_supervisor() {
    let directory = TestDirectory::new();
    let _journal = directory.open();
    assert!(matches!(
        Journal::open(&directory.0, "test-host", 1024 * 1024),
        Err(SupervisorError::JournalInUse)
    ));
}

#[test]
fn rejected_sequence_does_not_change_durable_state() {
    let directory = TestDirectory::new();
    let mut journal = directory.open();
    let provision = provision();
    journal.register(&provision, "boot-1").unwrap();
    assert!(matches!(
        journal.append(observation(&provision, 2)),
        Err(SupervisorError::InvalidJournalMessage)
    ));
    assert_eq!(journal.inventory()[0].last_sequence, 0);
}

#[test]
fn temporary_core_storage_failure_retains_original_message() {
    let directory = TestDirectory::new();
    let mut journal = directory.open();
    let provision = provision();
    journal.register(&provision, "boot-1").unwrap();
    let message = observation(&provision, 1);
    journal.append(message.clone()).unwrap();
    journal
        .acknowledge(&CoreAcknowledgement {
            message_id: new_id(),
            acknowledged_message_id: crate::journal::message_id(&message).unwrap().clone(),
            disposition: AcknowledgementDisposition::Rejected as i32,
            reason_code: "canonical_storage_unavailable".into(),
            message: String::new(),
        })
        .unwrap();
    assert_eq!(journal.pending().collect::<Vec<_>>(), vec![&message]);
}

#[test]
fn journal_refuses_reuse_by_different_host() {
    let directory = TestDirectory::new();
    drop(directory.open());
    assert!(matches!(
        Journal::open(&directory.0, "another-host", 1024 * 1024),
        Err(SupervisorError::JournalIdentity)
    ));
}

#[test]
fn journal_refuses_symlink_storage_directory() {
    let directory = TestDirectory::new();
    let link = TestDirectory::new();
    drop(directory.open());
    std::os::unix::fs::symlink(&directory.0, &link.0).unwrap();
    assert!(matches!(
        Journal::open(&link.0, "test-host", 1024 * 1024),
        Err(SupervisorError::UnsafeJournal)
    ));
    std::fs::remove_file(&link.0).unwrap();
}

#[test]
fn admission_rejects_invalid_scope_without_creating_tombstone() {
    let directory = TestDirectory::new();
    let mut journal = directory.open();
    let mut provision = provision();
    provision.run_id = "not:a:run:scope".into();
    assert!(matches!(
        journal.register(&provision, "boot-1"),
        Err(SupervisorError::InvalidJournalMessage)
    ));
    assert!(journal.inventory().is_empty());
}
