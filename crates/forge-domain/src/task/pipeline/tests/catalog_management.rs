use super::*;
use crate::{
    Pipeline, StageWorkspaceKind, StageWorkspaceRequirements,
    runtime::{SurfaceAccess, SurfaceSpec},
};

fn graph(pipeline: PipelineId, project: ProjectId, number: u32) -> PipelineVersion {
    PipelineVersion::new(PipelineVersionInput {
        id: PipelineVersionId::new(),
        pipeline_id: pipeline,
        project_id: project,
        version: number,
        task_kinds: BTreeSet::from([TaskKind::Delivery]),
        entry_stage_id: StageId::new("work").unwrap(),
        max_stage_visits: None,
        stages: vec![stage(
            "work",
            ExecutorKind::Employee,
            vec![transition("done", PipelineTransitionTarget::Done)],
        )],
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
    })
    .unwrap()
}

#[test]
fn catalog_default_and_publication_numbers_are_independent() {
    let id = PipelineId::new();
    let project = ProjectId::new();
    let first = graph(id, project, 1);
    let mut catalog = Pipeline::new(id, project, "Delivery", first.id()).unwrap();
    let second = graph(id, project, 2);
    catalog.publish_version(&second, false).unwrap();
    assert_eq!(
        (
            catalog.revision(),
            catalog.latest_version(),
            catalog.default_version_id()
        ),
        (2, 2, first.id())
    );
    catalog.set_default_version(&second).unwrap();
    catalog.set_default_version(&first).unwrap();
    assert_eq!(catalog.next_version().unwrap(), 3);
    catalog.soft_delete(Timestamp::now_utc()).unwrap();
    let frozen = catalog.clone();
    assert!(
        catalog
            .publish_version(&graph(id, project, 3), true)
            .is_err()
    );
    assert!(catalog.set_default_version(&first).is_err());
    assert!(catalog.soft_delete(Timestamp::now_utc()).is_err());
    assert_eq!(catalog, frozen);
    catalog.validate_snapshot().unwrap();
}

#[test]
fn catalog_invalid_scope_version_and_overflow_are_atomic() {
    let id = PipelineId::new();
    let project = ProjectId::new();
    let first = graph(id, project, 1);
    let mut catalog = Pipeline::new(id, project, "Delivery", first.id()).unwrap();
    let original = catalog.clone();
    for invalid in [
        graph(PipelineId::new(), project, 2),
        graph(id, ProjectId::new(), 2),
        graph(id, project, 3),
    ] {
        assert!(catalog.publish_version(&invalid, true).is_err());
        assert_eq!(catalog, original);
    }
    let mut snapshot = serde_json::to_value(&catalog).unwrap();
    snapshot["revision"] = serde_json::json!(u64::MAX);
    let mut exhausted: Pipeline = serde_json::from_value(snapshot).unwrap();
    let frozen = exhausted.clone();
    assert!(
        exhausted
            .publish_version(&graph(id, project, 2), true)
            .is_err()
    );
    assert_eq!(exhausted, frozen);
}

#[test]
fn legacy_pipeline_and_stage_defaults_preserve_m1() {
    let first = graph(PipelineId::new(), ProjectId::new(), 1);
    let catalog = Pipeline::new(
        first.pipeline_id(),
        first.project_id(),
        "Legacy",
        first.id(),
    )
    .unwrap();
    let mut old = serde_json::to_value(&catalog).unwrap();
    old.as_object_mut().unwrap().remove("revision");
    old.as_object_mut().unwrap().remove("latest_version");
    let restored: Pipeline = serde_json::from_value(old).unwrap();
    assert_eq!(restored, catalog);
    restored.validate_snapshot().unwrap();
    let mut old = serde_json::to_value(&first).unwrap();
    let stage = old["stages"]["work"].as_object_mut().unwrap();
    stage.remove("instructions");
    stage.remove("workspace");
    let restored: PipelineVersion = serde_json::from_value(old).unwrap();
    assert_eq!(restored, first);
    restored.validate_snapshot().unwrap();
}

#[test]
fn stage_requirements_are_explicit_bounded_and_do_not_grant_writes() {
    let stage = stage(
        "review",
        ExecutorKind::Employee,
        vec![transition("done", PipelineTransitionTarget::Done)],
    );
    assert!(
        stage
            .clone()
            .with_requirements("x".repeat(65537), None)
            .is_err()
    );
    assert!(
        stage
            .clone()
            .with_requirements("a\0b".into(), None)
            .is_err()
    );
    let requirement = StageWorkspaceRequirements {
        kind: StageWorkspaceKind::Git,
        access: SurfaceAccess::ReadOnly,
    };
    let configured = stage
        .with_requirements("Review the pinned commit".into(), Some(requirement))
        .unwrap();
    configured.validate_snapshot().unwrap();
    let git = SurfaceSpec::GitWorktree {
        repository: "/private/repo".into(),
        base_ref: "main".into(),
    };
    assert!(requirement.is_compatible(&git, SurfaceAccess::ReadOnly));
    assert!(!requirement.is_compatible(&git, SurfaceAccess::ReadWrite));
    assert!(!requirement.is_compatible(&SurfaceSpec::FilesystemSandbox, SurfaceAccess::ReadOnly));
}
