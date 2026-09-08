use super::*;
use forge_protocol::{
    SUPERVISOR_INVENTORY_LIMIT,
    supervisor::v1::{ObservedRunEvent, RunEventKind},
};

#[test]
fn dropping_journal_releases_lock_even_while_a_duplicate_descriptor_survives() {
    let directory = Directory::new();
    let journal = directory.open();
    // try_clone shares the open-file description just as fork inheritance does,
    // making the close-before-child-exec race deterministic without forking.
    let inherited = journal._lock.try_clone().unwrap();
    assert!(matches!(
        Journal::open(&directory.0, "test-host", 16 * 1024 * 1024),
        Err(SupervisorError::JournalInUse)
    ));
    drop(journal);
    let successor = directory.open();
    drop(inherited);
    assert!(matches!(
        Journal::open(&directory.0, "test-host", 16 * 1024 * 1024),
        Err(SupervisorError::JournalInUse)
    ));
    drop(successor);
    let _next_owner = directory.open();
}

#[test]
fn legacy_task_provision_serialization_and_dedupe_hash_are_unchanged() {
    let canonical = r#"{"command_id":"","run_id":"01900000-0000-7000-8000-000000000001","task_id":"01900000-0000-7000-8000-000000000002","employee_id":"01900000-0000-7000-8000-000000000003","stage_id":"work","attempt":1,"lease_fencing_token":42,"environment_epoch":7,"context_snapshot_id":"01900000-0000-7000-8000-000000000004","run_spec_json":"{}","run_spec_version":1,"traceparent":""}"#;
    let mut provision: ProvisionRun = serde_json::from_str(canonical).expect("legacy provision");
    assert!(provision.assignment.is_none());
    assert_eq!(serde_json::to_string(&provision).unwrap(), canonical);
    let expected = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    provision.command_id = new_id();
    assert_eq!(provision_hash(&provision).unwrap(), expected);
    provision.assignment = Some(
        forge_protocol::supervisor::v1::provision_run::Assignment::TaskStage(
            forge_protocol::supervisor::v1::TaskStageExecutionAssignment {
                task_id: provision.task_id.clone(),
                stage_id: provision.stage_id.clone(),
                queue_entry_id: new_id(),
            },
        ),
    );
    assert_ne!(provision_hash(&provision).unwrap(), expected);
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("forge-inventory-{}", new_id())))
    }
    fn open(&self) -> Journal {
        Journal::open(&self.0, "test-host", 16 * 1024 * 1024).expect("private test journal")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn observation(provision: &ProvisionRun, kind: RunEventKind) -> SupervisorToCore {
    SupervisorToCore {
        message: Some(supervisor_to_core::Message::ObservedRunEvent(
            ObservedRunEvent {
                message_id: new_id(),
                run_id: provision.run_id.clone(),
                lease_fencing_token: provision.lease_fencing_token,
                environment_epoch: provision.environment_epoch,
                sequence: 1,
                occurred_at_unix_ms: 1,
                kind: kind as i32,
                details_json: "{}".into(),
            },
        )),
    }
}

fn record(presence: EnvironmentPresence, confirmed: bool) -> RunRecord {
    let provision = crate::tests::provision();
    let payload_hash = provision_hash(&provision).expect("fixture hash");
    let mut record = RunRecord {
        git_source: None,
        provision,
        payload_hash,
        environment_id: "a".repeat(64),
        boot_id: "boot".into(),
        presence,
        last_sequence: 1,
        pending: Vec::new(),
        quiescence_confirmed: confirmed,
        stop_reason: None,
    };
    record.compact();
    record
}

#[test]
fn only_accepted_stopped_ack_can_retire_an_inventory_entry() {
    for kind in [
        RunEventKind::Stopped,
        RunEventKind::StartFailed,
        RunEventKind::Running,
    ] {
        for disposition in [
            AcknowledgementDisposition::Accepted,
            AcknowledgementDisposition::IgnoredStale,
            AcknowledgementDisposition::Rejected,
        ] {
            let directory = Directory::new();
            let provision = crate::tests::provision();
            let mut journal = directory.open();
            journal.register(&provision, "boot").expect("register");
            let message = observation(&provision, kind);
            let id = message_id(&message).expect("message id").clone();
            journal.append(message).expect("append");
            journal
                .finish(&provision, true)
                .expect("physical quiescence");
            journal
                .acknowledge(&CoreAcknowledgement {
                    message_id: new_id(),
                    acknowledged_message_id: id,
                    disposition: disposition as i32,
                    reason_code: String::new(),
                    message: String::new(),
                })
                .expect("ack");
            let omitted = kind == RunEventKind::Stopped
                && disposition == AcknowledgementDisposition::Accepted;
            assert_eq!(journal.inventory().is_empty(), omitted);
            assert_eq!(journal.records().len(), 1, "tombstone retained");
            assert!(!journal.register(&provision, "boot").expect("dedupe"));
            drop(journal);
            assert_eq!(
                directory.open().inventory().is_empty(),
                omitted,
                "confirmation is durable"
            );
        }
    }
}

#[test]
fn more_than_4096_confirmed_historical_runs_do_not_fill_inventory_or_delete_evidence() {
    let directory = Directory::new();
    let mut journal = directory.open();
    let retained = directory.0.join("retained-stdout.log");
    fs::write(&retained, b"retained synthetic technical evidence").expect("fixture evidence");
    journal
        .change(|snapshot| {
            for _ in 0..=SUPERVISOR_INVENTORY_LIMIT {
                let record = record(EnvironmentPresence::Quiescent, true);
                snapshot.runs.insert(scope_key(&record.provision), record);
            }
            for presence in [EnvironmentPresence::Active, EnvironmentPresence::Unknown] {
                let record = record(presence, true);
                snapshot.runs.insert(scope_key(&record.provision), record);
            }
            let legacy = record(EnvironmentPresence::Quiescent, false);
            snapshot.runs.insert(scope_key(&legacy.provision), legacy);
            let mut unacknowledged = record(EnvironmentPresence::Quiescent, true);
            unacknowledged.pending.push(observation(
                &unacknowledged.provision,
                RunEventKind::Stopped,
            ));
            snapshot
                .runs
                .insert(scope_key(&unacknowledged.provision), unacknowledged);
            Ok(())
        })
        .expect("persist historical fixture in one transaction");
    assert_eq!(journal.inventory().len(), 4);
    assert!(
        journal
            .inventory()
            .iter()
            .any(|entry| entry.presence == EnvironmentPresence::Active as i32)
    );
    assert!(
        journal
            .inventory()
            .iter()
            .any(|entry| entry.presence == EnvironmentPresence::Unknown as i32)
    );
    let count = journal.records().len();
    drop(journal);
    let mut reopened = directory.open();
    assert_eq!(reopened.inventory().len(), 4);
    assert_eq!(reopened.records().len(), count);
    assert!(
        reopened
            .register(&crate::tests::provision(), "boot")
            .expect("history does not block admission")
    );
    assert_eq!(
        fs::read(retained).expect("retained evidence"),
        b"retained synthetic technical evidence"
    );
}

#[test]
fn unresolved_capacity_refuses_new_scope_but_preserves_every_existing_identity() {
    let directory = Directory::new();
    let mut journal = directory.open();
    journal
        .change(|snapshot| {
            for _ in 0..SUPERVISOR_INVENTORY_LIMIT {
                let record = record(EnvironmentPresence::Unknown, false);
                snapshot.runs.insert(scope_key(&record.provision), record);
            }
            Ok(())
        })
        .expect("persist unresolved fixture");
    let duplicate = journal.records()[0].provision.clone();
    assert!(
        !journal
            .register(&duplicate, "boot")
            .expect("duplicate remains idempotent")
    );
    assert!(matches!(
        journal.register(&crate::tests::provision(), "boot"),
        Err(SupervisorError::InventoryCapacity)
    ));
    assert_eq!(journal.inventory().len(), SUPERVISOR_INVENTORY_LIMIT);
    assert_eq!(journal.records().len(), SUPERVISOR_INVENTORY_LIMIT);
}

#[test]
fn legacy_tombstone_without_confirmation_is_not_silently_omitted() {
    let mut value =
        serde_json::to_value(record(EnvironmentPresence::Quiescent, true)).expect("encode");
    value
        .as_object_mut()
        .expect("object")
        .remove("quiescence_confirmed");
    let legacy: RunRecord = serde_json::from_value(value).expect("legacy record");
    assert!(legacy.inventory_required());
}
