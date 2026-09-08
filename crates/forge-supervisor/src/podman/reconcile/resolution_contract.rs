use super::*;
use forge_domain::{
    ExecutionAssignment, ResolutionAssignmentRef,
    runtime::{ResolutionRunSpec, RuntimeLaunchSpec, SurfaceSpec},
};
use forge_protocol::supervisor::v1::{
    DeliverRuntimeInput, ResolutionExecutionAssignment, provision_run::Assignment,
};
use uuid::Uuid;

pub(super) fn provision() -> ProvisionRun {
    let mut provision = crate::tests::provision();
    let mut base = valid_spec();
    base.binding.surface = SurfaceSpec::None;
    let owner = ResolutionAssignmentRef {
        assignment_id: Uuid::now_v7(),
        escalation_id: Uuid::now_v7(),
        lease_generation: 3,
    };
    let spec = ResolutionRunSpec {
        schema_version: 4,
        project_id: base.project_id,
        run_id: provision.run_id.parse().unwrap(),
        assignment: owner.clone(),
        binding: base.binding,
        instruction: "Resolve the exact leased question; context grants no Task authority".into(),
    };
    provision.run_spec_version = 4;
    provision.task_id.clear();
    provision.stage_id.clear();
    provision.run_spec_json = serde_json::to_string(&spec).unwrap();
    provision.assignment = Some(Assignment::Resolution(ResolutionExecutionAssignment {
        assignment_id: owner.assignment_id.to_string(),
        escalation_id: owner.escalation_id.to_string(),
        lease_generation: owner.lease_generation,
    }));
    provision
}

#[test]
fn resolution_v4_exact_owner_and_private_scratch_are_mandatory() {
    let provision = provision();
    crate::execution_assignment::validate(&provision).unwrap();
    let spec: RuntimeLaunchSpec = serde_json::from_str(&provision.run_spec_json).unwrap();
    assert_eq!(spec.surface_id, provision.run_id.parse::<Uuid>().unwrap());
    assert!(spec.communication.is_none());
    let owner = ExecutionAssignment::Resolution(spec.resolution.clone().unwrap());
    assert!(owner.task_stage().is_none());
    assert!(owner.communication().is_none());
    owner.validate().unwrap();
    let value: serde_json::Value = serde_json::from_str(&provision.run_spec_json).unwrap();
    for (field, replacement) in [
        ("schema_version", serde_json::json!(3)),
        ("surface_id", serde_json::json!(new_id())),
        ("task_id", serde_json::json!(new_id())),
    ] {
        let mut invalid = value.clone();
        invalid[field] = replacement;
        assert!(serde_json::from_value::<RuntimeLaunchSpec>(invalid).is_err());
    }
    let mut invalid = value.clone();
    invalid["binding"]["surface"] = serde_json::json!({"mode":"filesystem_sandbox"});
    assert!(serde_json::from_value::<RuntimeLaunchSpec>(invalid).is_err());
    let mut invalid = value;
    invalid["assignment"]["lease_generation"] = serde_json::json!(0);
    assert!(serde_json::from_value::<RuntimeLaunchSpec>(invalid).is_err());
    let mut wrong = provision.clone();
    wrong.task_id = new_id();
    assert!(crate::execution_assignment::validate(&wrong).is_err());
    wrong = provision.clone();
    if let Some(Assignment::Resolution(owner)) = &mut wrong.assignment {
        owner.lease_generation += 1;
    }
    assert!(crate::execution_assignment::validate(&wrong).is_err());
    let input = DeliverRuntimeInput {
        command_id: new_id(),
        run_id: provision.run_id.clone(),
        lease_fencing_token: provision.lease_fencing_token,
        environment_epoch: provision.environment_epoch,
        sequence: 1,
        action: None,
    };
    assert!(
        crate::runtime_input::validate(&input, &provision).is_err(),
        "v4 has no live-input lane"
    );
}

#[test]
fn resolution_v4_journal_replay_preserves_owner_and_legacy_serialization() {
    let directory =
        Directory(std::env::temp_dir().join(format!("forge-resolution-journal-{}", new_id())));
    let provision = provision();
    let mut journal = Journal::open(&directory.0, "test-host", 1024 * 1024).unwrap();
    assert!(journal.register(&provision, "boot").unwrap());
    assert!(!journal.register(&provision, "boot").unwrap());
    drop(journal);
    let mut journal = Journal::open(&directory.0, "test-host", 1024 * 1024).unwrap();
    assert!(!journal.register(&provision, "boot").unwrap());
    assert_eq!(journal.records()[0].provision, provision);
    let mut conflicting = provision;
    conflicting.run_spec_json = conflicting
        .run_spec_json
        .replace("leased question", "different question");
    assert!(matches!(
        journal.register(&conflicting, "boot"),
        Err(SupervisorError::ConflictingProvision)
    ));
}

#[test]
fn resolution_v4_profile_live_input_capability_does_not_enable_duplex() {
    let mut provision = provision();
    let mut value: serde_json::Value = serde_json::from_str(&provision.run_spec_json).unwrap();
    value["binding"]["execution_profile"]["capability_profile"]["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("live_input"));
    provision.run_spec_json = serde_json::to_string(&value).unwrap();
    crate::execution_assignment::validate(&provision).unwrap();
    let input = DeliverRuntimeInput {
        command_id: new_id(),
        run_id: provision.run_id.clone(),
        lease_fencing_token: provision.lease_fencing_token,
        environment_epoch: provision.environment_epoch,
        sequence: 1,
        action: Some(
            forge_protocol::supervisor::v1::deliver_runtime_input::Action::CloseAfterTurn(
                forge_protocol::supervisor::v1::CloseRuntimeInput {
                    reason_code: "assignment_completed".into(),
                },
            ),
        ),
    };
    assert!(crate::runtime_input::validate(&input, &provision).is_err());
}
