use super::*;
use forge_domain::{
    EmployeeId,
    runtime::{SurfaceSpec, SystemJobRunSpec},
    system_job::{SystemJobAssignmentRef, SystemJobInput, SystemJobKind},
};
use forge_protocol::supervisor::v1::{
    DeliverRuntimeInput, SystemJobExecutionAssignment, provision_run::Assignment,
};
use uuid::Uuid;

fn provision() -> ProvisionRun {
    let mut provision = crate::tests::provision();
    let mut base = valid_spec();
    base.binding.surface = SurfaceSpec::None;
    let owner = SystemJobAssignmentRef {
        job_id: Uuid::now_v7(),
        attempt_id: Uuid::now_v7(),
        generation: 3,
        kind: SystemJobKind::Onboarding,
    };
    let spec = SystemJobRunSpec {
        schema_version: 7,
        project_id: base.project_id,
        run_id: provision.run_id.parse().unwrap(),
        assignment: owner.clone(),
        input: SystemJobInput {
            source_task_id: None,
            target_employee_id: Some(EmployeeId::new()),
            covered_sequence: 0,
            source_digest: "a".repeat(64),
            context: serde_json::json!({}),
        },
        max_result_bytes: 16 * 1024,
        binding: base.binding,
        instruction: "Frozen SystemJob input".into(),
    };
    provision.run_spec_version = 7;
    provision.employee_id.clear();
    provision.task_id.clear();
    provision.stage_id.clear();
    provision.run_spec_json = serde_json::to_string(&spec).unwrap();
    provision.assignment = Some(Assignment::SystemJob(SystemJobExecutionAssignment {
        job_id: owner.job_id.to_string(),
        attempt_id: owner.attempt_id.to_string(),
        generation: owner.generation,
        kind: "onboarding".into(),
    }));
    provision
}

#[test]
fn system_job_schema_seven_requires_exact_wire_owner_and_has_no_live_input() {
    let good = provision();
    crate::execution_assignment::validate(&good).unwrap();
    for case in 0..9 {
        let mut invalid = good.clone();
        match case {
            0 => invalid.employee_id = new_id(),
            1 => invalid.task_id = new_id(),
            2 => invalid.stage_id = "work".into(),
            3 => invalid.run_id = new_id(),
            4 => invalid.assignment = None,
            _ => {
                if let Some(Assignment::SystemJob(owner)) = &mut invalid.assignment {
                    match case {
                        5 => owner.generation += 1,
                        6 => owner.job_id = new_id(),
                        7 => owner.attempt_id = new_id(),
                        _ => owner.kind = "summarization".into(),
                    }
                }
            }
        }
        assert!(
            crate::execution_assignment::validate(&invalid).is_err(),
            "case {case}"
        );
    }
    let input = DeliverRuntimeInput {
        command_id: new_id(),
        run_id: good.run_id.clone(),
        lease_fencing_token: good.lease_fencing_token,
        environment_epoch: good.environment_epoch,
        sequence: 1,
        action: None,
    };
    assert!(crate::runtime_input::validate(&input, &good).is_err());
}

#[test]
fn system_job_journal_replay_keeps_attempt_identity_and_rejects_conflicts() {
    let directory =
        Directory(std::env::temp_dir().join(format!("forge-system-job-journal-{}", new_id())));
    let good = provision();
    let mut journal = Journal::open(&directory.0, "test-host", 1024 * 1024).unwrap();
    assert!(journal.register(&good, "boot").unwrap());
    assert!(!journal.register(&good, "boot").unwrap());
    drop(journal);
    let mut journal = Journal::open(&directory.0, "test-host", 1024 * 1024).unwrap();
    assert_eq!(journal.records()[0].provision, good);
    assert!(!journal.register(&good, "boot").unwrap());
    let mut conflicting = good;
    if let Some(Assignment::SystemJob(owner)) = &mut conflicting.assignment {
        owner.generation += 1;
    }
    assert!(matches!(
        journal.register(&conflicting, "boot"),
        Err(SupervisorError::InvalidRunSpec)
    ));
    let mut spec: serde_json::Value = serde_json::from_str(&conflicting.run_spec_json).unwrap();
    spec["assignment"]["generation"] = serde_json::json!(4);
    conflicting.run_spec_json = serde_json::to_string(&spec).unwrap();
    assert!(matches!(
        journal.register(&conflicting, "boot"),
        Err(SupervisorError::ConflictingProvision)
    ));
}
