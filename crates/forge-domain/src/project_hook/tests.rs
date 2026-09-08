use super::*;
use crate::{
    HookAssignmentRef, PipelineVersionId, StageId, TaskGitBinding, TaskId,
    git::{GitBranchRef, GitCandidate, GitObjectId, LocalGitPath},
    runtime::{HookRunSpec, SandboxLaunchSpec},
};

pub(super) fn version() -> ProjectHookVersion {
    ProjectHookVersion {
        id: Uuid::now_v7(),
        project_id: ProjectId::new(),
        name: "Explicit checks".into(),
        image: format!("forge-hook@sha256:{}", "a".repeat(64)),
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf 'owner configured'".into(),
        ],
        workdir: ".".into(),
        limits: ResourceLimits::default(),
        max_output_bytes: 1024 * 1024,
        applicable_task_kinds: BTreeSet::new(),
        required: true,
        created_at: Timestamp::now_utc(),
    }
}
#[test]
fn hook_versions_are_explicit_bounded_and_applicability_is_not_discovery() {
    let mut hook = version();
    hook.validate_snapshot().unwrap();
    assert!(hook.applies_to(TaskKind::Analysis));
    hook.applicable_task_kinds.insert(TaskKind::Delivery);
    assert!(!hook.applies_to(TaskKind::Analysis));
    for workdir in ["/tmp", "../target", "a/../../target", "a/./../b", ""] {
        let mut bad = hook.clone();
        bad.workdir = workdir.into();
        assert!(bad.validate_snapshot().is_err());
    }
    for image in ["forge-hook:latest", "-image", "forge@sha256:bad"] {
        let mut bad = hook.clone();
        bad.image = image.into();
        assert!(bad.validate_snapshot().is_err());
    }
    let mut bad = hook.clone();
    bad.command.clear();
    assert!(bad.validate_snapshot().is_err());
    let mut bad = hook;
    bad.command.push("secret\0injection".into());
    assert!(bad.validate_snapshot().is_err());
}
#[test]
fn provider_free_spec_has_no_employee_or_provider_and_keeps_task_context_separate() {
    let hook = version();
    let spec = HookRunSpec {
        schema_version: 5,
        project_id: hook.project_id,
        run_id: Uuid::now_v7(),
        assignment: HookAssignmentRef {
            invocation_id: Uuid::now_v7(),
            task_id: TaskId::new(),
            pipeline_version_id: PipelineVersionId::new(),
            stage_id: StageId::new("configured_check").unwrap(),
            stage_visit: 2,
            candidate_proposal_id: Uuid::now_v7(),
        },
        hook,
        binding: TaskGitBinding {
            repository_id: Uuid::now_v7(),
            source: LocalGitPath::new("/operator/toy-repository").unwrap(),
            target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
            initial_base: GitObjectId::new("1".repeat(40)).unwrap(),
            surface_id: Uuid::now_v7(),
        },
        candidate: GitCandidate {
            commit: GitObjectId::new("2".repeat(40)).unwrap(),
            tree: GitObjectId::new("3".repeat(40)).unwrap(),
        },
    };
    spec.validate().unwrap();
    let owner = crate::ExecutionAssignment::Hook(spec.assignment.clone());
    assert!(owner.task_stage().is_none());
    let value = serde_json::to_value(&spec).unwrap();
    let decoded: SandboxLaunchSpec = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(decoded.surface_id(), spec.run_id);
    assert!(
        serde_json::from_value::<super::super::runtime::RuntimeLaunchSpec>(value.clone()).is_err()
    );
    for (key, extra) in [
        ("employee_id", serde_json::json!(Uuid::now_v7())),
        ("execution_profile", serde_json::json!({})),
    ] {
        let mut bad = value.clone();
        bad[key] = extra;
        assert!(serde_json::from_value::<SandboxLaunchSpec>(bad).is_err());
    }
    let mut bad = spec.clone();
    bad.hook.project_id = ProjectId::new();
    assert!(bad.validate().is_err());
    let mut bad = spec;
    bad.assignment.stage_visit = 0;
    assert!(bad.validate().is_err());
}
