use super::*;
use crate::podman::git_inspection::tests::fixture::{Fixture, git};
use forge_domain::{
    git::{GitBranchRef, GitObjectId, GitSourceRequest, LocalGitPath, TaskGitSourcePolicy},
    runtime::SurfaceSpec,
};
use forge_protocol::supervisor::v1::{TaskStageExecutionAssignment, provision_run::Assignment};
use uuid::Uuid;

async fn next_writer(fixture: &Fixture) -> (ProvisionRun, RuntimeLaunchSpec) {
    let mut provision = fixture.provision.clone();
    provision.command_id = crate::new_id();
    provision.run_id = crate::new_id();
    provision.run_spec_version = 6;
    provision.assignment = Some(Assignment::TaskStage(TaskStageExecutionAssignment {
        task_id: provision.task_id.clone(),
        stage_id: provision.stage_id.clone(),
        queue_entry_id: crate::new_id(),
    }));
    let mut spec: RuntimeLaunchSpec = fixture.spec.clone().into();
    spec.schema_version = 6;
    let SurfaceSpec::GitWorktree {
        repository,
        base_ref,
    } = &spec.binding.surface
    else {
        panic!("Git fixture");
    };
    spec.source_request = Some(GitSourceRequest {
        repository_id: Uuid::now_v7(),
        source: LocalGitPath::new(repository).unwrap(),
        target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
        initial_revision: GitObjectId::new(base_ref).unwrap().into(),
        policy_revision: 1,
        policy: TaskGitSourcePolicy::LatestTarget,
    });
    provision.run_spec_json = serde_json::to_string(&forge_domain::runtime::TaskRunSpecV6 {
        schema_version: spec.schema_version,
        project_id: spec.project_id,
        surface_id: spec.surface_id,
        binding: spec.binding.clone(),
        instruction: spec.instruction.clone(),
        source_request: spec.source_request.clone(),
        file_inputs: spec.file_inputs.clone(),
    })
    .unwrap();
    fixture
        .registry
        .register(&provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    (provision, spec)
}

#[tokio::test]
async fn source_export_requires_physical_exit_even_after_durable_logical_stop() {
    let fixture = Fixture::new().await;
    let (provision, spec) = next_writer(&fixture).await;
    fixture.physical_state(true, "running", &"a".repeat(64));
    assert!(matches!(
        fixture
            .backend
            .prepare_source(&provision, &spec, &fixture.registry)
            .await,
        Err(SupervisorError::ConflictingProvision)
    ));
    assert!(!fixture.config.state_directory.join("git-sources").exists());
    assert!(
        fixture
            .registry
            .journal
            .lock()
            .await
            .source_selection(&provision)
            .is_none()
    );
    fixture.physical_state(false, "exited", &"a".repeat(64));
    let prepared = fixture
        .backend
        .prepare_source(&provision, &spec, &fixture.registry)
        .await
        .unwrap()
        .unwrap();
    assert!(prepared.directory.join("source.bundle").is_file());
    let request = spec.source_request.as_ref().unwrap();
    std::fs::write(request.source.as_path().join("later"), "new source").unwrap();
    git(request.source.as_path(), &["add", "later"]).await;
    git(request.source.as_path(), &["commit", "-m", "Later source"]).await;
    let replay = fixture
        .backend
        .prepare_source(&provision, &spec, &fixture.registry)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(prepared.directory, replay.directory);
    assert_eq!(prepared.descriptor, replay.descriptor);
}

#[tokio::test]
async fn source_refresh_rejects_unknown_or_mismatched_previous_environment() {
    let fixture = Fixture::new().await;
    let (provision, spec) = next_writer(&fixture).await;
    fixture.physical_state(false, "exited", &"b".repeat(64));
    assert!(
        fixture
            .backend
            .prepare_source(&provision, &spec, &fixture.registry)
            .await
            .is_err()
    );
    fixture.mark("absent");
    assert!(
        fixture
            .backend
            .prepare_source(&provision, &spec, &fixture.registry)
            .await
            .is_ok()
    );
    fixture
        .registry
        .mark_unknown(&fixture.provision)
        .await
        .unwrap();
    assert!(
        fixture
            .backend
            .prepare_source(&provision, &spec, &fixture.registry)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn exact_physical_exit_allows_new_writer_before_stopped_ack_arrives() {
    let fixture = Fixture::new().await;
    let (provision, spec) = next_writer(&fixture).await;
    fixture
        .registry
        .mark_unknown(&fixture.provision)
        .await
        .unwrap();
    fixture
        .registry
        .finish(&fixture.provision, true)
        .await
        .unwrap();
    let record = fixture
        .registry
        .journal
        .lock()
        .await
        .records()
        .into_iter()
        .find(|record| record.provision.run_id == fixture.provision.run_id)
        .unwrap();
    assert_eq!(record.presence, EnvironmentPresence::Quiescent);
    assert!(!record.quiescence_confirmed);
    assert!(
        fixture
            .backend
            .prepare_source(&provision, &spec, &fixture.registry)
            .await
            .is_ok()
    );
    fixture.mark("absent");
    assert!(matches!(
        fixture
            .backend
            .prepare_source(&provision, &spec, &fixture.registry)
            .await,
        Err(SupervisorError::ConflictingProvision)
    ));
}
