use super::{TaskKind, TaskPropertySchema, Timestamp, draft};
use crate::{
    ProjectId, ProjectRepository, Task, TaskWorkSurface,
    git::{GitBranchRef, GitObjectId, LocalGitPath},
};
use uuid::Uuid;

fn repository(task: &Task) -> ProjectRepository {
    ProjectRepository {
        id: Uuid::now_v7(),
        project_id: task.project_id(),
        name: "Mock source".into(),
        source: LocalGitPath::new("/tmp/explicit-mock-repo").expect("path"),
        target_ref: GitBranchRef::new("refs/heads/main").expect("branch"),
        created_by: task.created_by(),
        created_at: task.created_at(),
    }
}

#[test]
fn binding_is_once_draft_only_and_retained_without_changing_legacy_none() {
    let mut task = draft(TaskKind::Delivery);
    assert_eq!(
        serde_json::to_value(task.work_surface()).expect("surface"),
        "none"
    );
    let original = serde_json::to_value(&task).expect("task");
    let decoded: Task = serde_json::from_value(original.clone()).expect("legacy Task");
    assert_eq!(serde_json::to_value(decoded).expect("roundtrip"), original);
    let repository = repository(&task);
    let base = GitObjectId::new("a".repeat(40)).expect("base");
    task.bind_git_repository(
        &repository,
        base.clone(),
        Uuid::now_v7(),
        Timestamp::now_utc(),
    )
    .expect("bind");
    let bound = task.clone();
    assert!(
        task.bind_git_repository(
            &repository,
            base.clone(),
            Uuid::now_v7(),
            Timestamp::now_utc()
        )
        .is_err()
    );
    assert_eq!(task, bound);
    task.approve(
        &TaskPropertySchema::new([]).expect("schema"),
        Timestamp::now_utc(),
    )
    .expect("approve");
    assert_eq!(task.work_surface(), bound.work_surface());
    let snapshot = serde_json::to_value(&task).expect("snapshot");
    let decoded: Task = serde_json::from_value(snapshot).expect("snapshot roundtrip");
    decoded.validate_snapshot().expect("valid bound Task");
    assert_eq!(decoded, task);
}

#[test]
fn wrong_scope_and_approved_task_cannot_acquire_source() {
    let mut task = draft(TaskKind::Delivery);
    let mut repository = repository(&task);
    repository.project_id = ProjectId::new();
    let base = GitObjectId::new("a".repeat(40)).expect("base");
    assert!(
        task.bind_git_repository(
            &repository,
            base.clone(),
            Uuid::now_v7(),
            Timestamp::now_utc()
        )
        .is_err()
    );
    assert_eq!(task.work_surface(), &TaskWorkSurface::None);
    repository.project_id = task.project_id();
    task.approve(
        &TaskPropertySchema::new([]).expect("schema"),
        Timestamp::now_utc(),
    )
    .expect("approve");
    let approved = task.clone();
    assert!(
        task.bind_git_repository(&repository, base, Uuid::now_v7(), Timestamp::now_utc())
            .is_err()
    );
    assert_eq!(task, approved);
}

#[test]
fn malformed_snapshot_cannot_bypass_typed_git_identifiers() {
    let mut task = draft(TaskKind::Delivery);
    task.bind_git_repository(
        &repository(&task),
        GitObjectId::new("a".repeat(40)).expect("base"),
        Uuid::now_v7(),
        Timestamp::now_utc(),
    )
    .expect("bind");
    let snapshot = serde_json::to_value(&task).expect("snapshot");
    for (field, value) in [
        ("initial_base", "main"),
        ("source", "/"),
        ("target_ref", "HEAD"),
    ] {
        let mut invalid = snapshot.clone();
        invalid["work_surface"]["git"][field] = serde_json::json!(value);
        assert!(serde_json::from_value::<Task>(invalid).is_err());
    }
    let mut invalid = snapshot;
    invalid["work_surface"]["git"]["surface_id"] = serde_json::json!(Uuid::nil());
    let decoded: Task = serde_json::from_value(invalid).expect("UUID shape");
    assert!(decoded.validate_snapshot().is_err());
}

#[test]
fn analysis_without_git_can_reach_hook_applicability_but_not_integration() {
    let mut task = draft(TaskKind::Analysis);
    task.approve(
        &TaskPropertySchema::new([]).expect("schema"),
        Timestamp::now_utc(),
    )
    .expect("approve analysis");
    assert_eq!(task.work_surface(), &TaskWorkSurface::None);
    let outcome = crate::OutcomeKey::new("configured").expect("outcome");
    let hook = crate::git_integration::SystemStageAction::ProjectHook {
        hook_version_id: Uuid::now_v7(),
        outcomes: crate::HookOutcomes {
            passed: outcome.clone(),
            failed: outcome.clone(),
            timed_out: outcome.clone(),
            skipped: outcome.clone(),
        },
    };
    hook.validate_task(&task)
        .expect("Core evaluates applicability");
    let integration = crate::git_integration::SystemStageAction::GitIntegration {
        outcomes: crate::git_integration::IntegrationOutcomes {
            applied: outcome.clone(),
            no_changes: outcome.clone(),
            stale_base: outcome,
        },
        required_review_stages: Default::default(),
    };
    assert!(integration.validate_task(&task).is_err());
}
