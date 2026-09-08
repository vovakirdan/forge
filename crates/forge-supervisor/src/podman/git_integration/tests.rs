use super::*;
use crate::podman::git_inspection::tests::fixture::{Fixture, git};
use forge_domain::{
    PipelineVersionId, StageId, TaskGitBinding, TaskId, Timestamp,
    git::{GitBranchRef, GitCandidate, GitObjectId, LocalGitPath},
    git_integration::GitIntegrationOperation,
    runtime::{RunScope, SurfaceSpec},
};
use forge_protocol::supervisor::v1::supervisor_to_core;
use uuid::Uuid;

async fn request(fixture: &Fixture) -> IntegrationRequest {
    let SurfaceSpec::GitWorktree {
        repository,
        base_ref,
    } = &fixture.spec.binding.surface
    else {
        panic!("Git fixture");
    };
    git(std::path::Path::new(repository), &["checkout", "--detach"]).await;
    IntegrationRequest {
        command_id: Uuid::now_v7(),
        phase: IntegrationPhase::Prepare,
        intent: None,
        host_id: fixture.config.host_id.clone(),
        boot_id: fixture.config.boot_id.clone(),
        operation: GitIntegrationOperation {
            id: Uuid::now_v7(),
            fence: 1,
            project_id: fixture.spec.project_id,
            task_id: TaskId::from(Uuid::parse_str(&fixture.provision.task_id).unwrap()),
            pipeline_version_id: PipelineVersionId::new(),
            stage_id: StageId::new("publish_here").unwrap(),
            stage_visit: 3,
            task_revision: 10,
            candidate_proposal_id: Uuid::now_v7(),
            candidate: GitCandidate {
                commit: GitObjectId::new(&fixture.request.expected_commit).unwrap(),
                tree: GitObjectId::new(git(&fixture.worktree, &["rev-parse", "HEAD^{tree}"]).await)
                    .unwrap(),
            },
            binding: TaskGitBinding {
                repository_id: Uuid::now_v7(),
                source: LocalGitPath::new(repository).unwrap(),
                target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
                initial_base: GitObjectId::new(base_ref).unwrap(),
                surface_id: fixture.spec.surface_id,
            },
            writer: RunScope {
                run_id: Uuid::parse_str(&fixture.provision.run_id).unwrap(),
                fencing_token: fixture.provision.lease_fencing_token,
                environment_epoch: fixture.provision.environment_epoch,
            },
            created_at: Timestamp::now_utc(),
        },
    }
}
async fn send(fixture: &Fixture, request: &IntegrationRequest) -> IntegrationResult {
    fixture
        .backend
        .integrate_git(
            GitIntegrationCommand {
                command_id: request.command_id.to_string(),
                request_json: serde_json::to_string(request).unwrap(),
            },
            fixture.registry.clone(),
        )
        .await
        .unwrap();
    fixture
        .registry
        .journal
        .lock()
        .await
        .pending()
        .find_map(|message| {
            let Some(supervisor_to_core::Message::GitIntegration(reply)) = &message.message else {
                return None;
            };
            let result: IntegrationResult = serde_json::from_str(&reply.result_json).unwrap();
            (result.command_id == request.command_id).then_some(result)
        })
        .expect("durable reply")
}

#[tokio::test]
async fn prepare_does_not_publish_apply_preserves_exact_tree_and_replay_after_restart_is_readonly()
{
    let mut fixture = Fixture::new().await;
    let prepare = request(&fixture).await;
    let target = prepare.operation.binding.source.as_path();
    let before = git(target, &["rev-parse", "refs/heads/main"]).await;
    let prepared = send(&fixture, &prepare).await;
    assert_eq!(prepared.code, IntegrationCode::Prepared);
    let intent = prepared.intent.clone().unwrap();
    assert_eq!(git(target, &["rev-parse", "refs/heads/main"]).await, before);
    let mut apply = prepare.clone();
    apply.command_id = Uuid::now_v7();
    apply.phase = IntegrationPhase::Apply;
    apply.intent = Some(intent.clone());
    let result = send(&fixture, &apply).await;
    assert_eq!(result.code, IntegrationCode::Applied);
    assert_eq!(
        git(target, &["rev-parse", "refs/heads/main"]).await,
        intent.merge_commit.as_str()
    );
    assert_eq!(
        git(target, &["rev-parse", "refs/heads/main^{tree}"]).await,
        intent.candidate.tree.as_str()
    );
    fixture.acknowledge(&result.message_id.to_string()).await;
    fixture.restart().await;
    assert_eq!(
        send(&fixture, &apply).await,
        result,
        "same effect receipt must replay after restart"
    );
    let mut reconcile = apply;
    reconcile.command_id = Uuid::now_v7();
    reconcile.phase = IntegrationPhase::Reconcile;
    assert_eq!(
        send(&fixture, &reconcile).await.code,
        IntegrationCode::Applied
    );
}

#[tokio::test]
async fn incomplete_apply_admission_only_reconciles_and_target_movement_is_never_overwritten() {
    let fixture = Fixture::new().await;
    let prepare = request(&fixture).await;
    let intent = send(&fixture, &prepare).await.intent.unwrap();
    let mut apply = prepare;
    apply.command_id = Uuid::now_v7();
    apply.phase = IntegrationPhase::Apply;
    apply.intent = Some(intent.clone());
    fixture
        .registry
        .journal
        .lock()
        .await
        .begin_integration(&apply)
        .unwrap();
    assert_eq!(
        send(&fixture, &apply).await.code,
        IntegrationCode::Retryable,
        "crash before CAS cannot implicitly retry it"
    );
    assert_eq!(
        git(
            apply.operation.binding.source.as_path(),
            &["rev-parse", "refs/heads/main"]
        )
        .await,
        intent.expected_target.as_str()
    );
    let target = apply.operation.binding.source.as_path();
    git(
        target,
        &["commit", "--allow-empty", "-m", "External owner movement"],
    )
    .await;
    let moved = git(target, &["rev-parse", "HEAD"]).await;
    git(target, &["update-ref", "refs/heads/main", &moved]).await;
    apply.command_id = Uuid::now_v7();
    assert_eq!(send(&fixture, &apply).await.code, IntegrationCode::Unknown);
    assert_eq!(git(target, &["rev-parse", "refs/heads/main"]).await, moved);
}

#[tokio::test]
async fn live_task_or_mismatched_source_refuses_before_staging_and_unknown_intent_never_applies() {
    let fixture = Fixture::new().await;
    let mut prepare = request(&fixture).await;
    fixture.physical_state(true, "running", &"a".repeat(64));
    assert_eq!(
        send(&fixture, &prepare).await.code,
        IntegrationCode::Refused
    );
    assert!(!fixture.config.state_directory.join("integrations").exists());
    fixture.physical_state(false, "exited", &"a".repeat(64));
    prepare.command_id = Uuid::now_v7();
    prepare.operation.id = Uuid::now_v7();
    prepare.operation.binding.source = LocalGitPath::new("/tmp/not-this-project").unwrap();
    assert_eq!(
        send(&fixture, &prepare).await.code,
        IntegrationCode::Refused
    );
    assert!(!fixture.config.state_directory.join("integrations").exists());
}

#[tokio::test]
async fn no_preparation_requires_retained_exact_provenance_and_never_replaces_a_prepared_intent() {
    let fixture = Fixture::new().await;
    let prepare = request(&fixture).await;
    let mut reconcile = prepare.clone();
    reconcile.command_id = Uuid::now_v7();
    reconcile.phase = IntegrationPhase::Reconcile;
    assert_eq!(
        send(&fixture, &reconcile).await.code,
        IntegrationCode::NoPreparation
    );
    let prepared = send(&fixture, &prepare).await;
    assert_eq!(prepared.code, IntegrationCode::Prepared);
    reconcile.command_id = Uuid::now_v7();
    let recovered = send(&fixture, &reconcile).await;
    assert_eq!(recovered.code, IntegrationCode::Prepared);
    assert_eq!(recovered.intent, prepared.intent);
    reconcile.command_id = Uuid::now_v7();
    reconcile.operation.id = Uuid::now_v7();
    reconcile.operation.writer.run_id = Uuid::now_v7();
    assert_eq!(
        send(&fixture, &reconcile).await.code,
        IntegrationCode::Unknown,
        "missing journal ownership is not proof of absent preparation"
    );
}
