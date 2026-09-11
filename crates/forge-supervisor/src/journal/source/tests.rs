use super::*;
use crate::{
    git::GitBackend,
    surface::pinned_fixture::{Fixture, git},
};
use forge_domain::git::{
    GitBranchRef, GitObjectId, GitSourceRequest, LocalGitPath, TaskGitSourcePolicy,
};
use forge_domain::runtime::{SurfaceAccess, SurfaceSpec};
use uuid::Uuid;

async fn inputs(fixture: &mut Fixture) -> GitSourceRequest {
    let commit = GitObjectId::new(git(&fixture.source, &["rev-parse", "HEAD"]).await).unwrap();
    let request = GitSourceRequest {
        repository_id: Uuid::now_v7(),
        source: LocalGitPath::new(fixture.source.to_str().unwrap()).unwrap(),
        target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
        initial_revision: commit.clone().into(),
        policy_revision: 1,
        policy: TaskGitSourcePolicy::LatestTarget,
    };
    fixture.spec.binding.access = SurfaceAccess::ReadWrite;
    fixture.spec.binding.surface = SurfaceSpec::GitWorktree {
        repository: fixture.source.to_str().unwrap().into(),
        base_ref: commit.as_str().into(),
    };
    fixture.provision.run_spec_version = 6;
    fixture.provision.run_spec_json = serde_json::json!({"schema_version":6,"project_id":fixture.spec.project_id,"surface_id":fixture.spec.surface_id,
        "binding":fixture.spec.binding,"instruction":fixture.spec.instruction,"source_request":request,"file_inputs":[]}).to_string();
    fixture.provision.assignment = Some(
        forge_protocol::supervisor::v1::provision_run::Assignment::TaskStage(
            forge_protocol::supervisor::v1::TaskStageExecutionAssignment {
                task_id: fixture.provision.task_id.clone(),
                stage_id: fixture.provision.stage_id.clone(),
                queue_entry_id: Uuid::now_v7().to_string(),
            },
        ),
    );
    request
}

#[tokio::test]
async fn frozen_source_survives_restart_before_export_and_rejects_new_selection() {
    let mut fixture = Fixture::new().await;
    let request = inputs(&mut fixture).await;
    let backend = GitBackend::default();
    let selected = backend.select_source(&request, false).await.unwrap();
    let directory = fixture.root.join("journal");
    let mut journal = Journal::open(&directory, "source-test", 1024 * 1024).unwrap();
    journal.register(&fixture.provision, "source-boot").unwrap();
    journal
        .record_source_selection(&fixture.provision, &selected)
        .unwrap();
    drop(journal);
    std::fs::write(fixture.source.join("later"), "moved source").unwrap();
    git(&fixture.source, &["add", "later"]).await;
    git(&fixture.source, &["commit", "-m", "Move target"]).await;
    let mut journal = Journal::open(&directory, "source-test", 1024 * 1024).unwrap();
    assert_eq!(
        journal.source_selection(&fixture.provision),
        Some(selected.clone())
    );
    assert!(journal.target_established(&selected.target_key));
    let newer = backend.select_source(&request, true).await.unwrap();
    assert!(
        journal
            .record_source_selection(&fixture.provision, &newer)
            .is_err()
    );
    journal
        .record_source_selection(&fixture.provision, &selected)
        .unwrap();
    let descriptor = backend
        .export_source(
            &journal.source_selection(&fixture.provision).unwrap(),
            &fixture.root.join("export"),
        )
        .await
        .unwrap();
    assert_eq!(descriptor.selected_revision, selected.selected_revision);
    git(
        &fixture.source,
        &["update-ref", "-d", request.target_ref.as_str()],
    )
    .await;
    let mut originally_unborn = request.clone();
    originally_unborn.initial_revision = forge_domain::git::GitInitialRevision::Unborn {
        object_format: forge_domain::git::GitObjectFormat::Sha1,
    };
    assert!(matches!(
        backend
            .select_source(
                &originally_unborn,
                journal.target_established(&selected.target_key)
            )
            .await,
        Err(crate::git::GitBackendError::TargetMissing)
    ));
}

#[tokio::test]
async fn journal_capacity_failure_does_not_accept_source_selection() {
    let mut fixture = Fixture::new().await;
    let request = inputs(&mut fixture).await;
    let selected = GitBackend::default()
        .select_source(&request, false)
        .await
        .unwrap();
    let mut journal =
        Journal::open(&fixture.root.join("journal"), "source-test", 1024 * 1024).unwrap();
    journal.register(&fixture.provision, "source-boot").unwrap();
    journal.max_bytes = serde_json::to_vec(&journal.snapshot).unwrap().len();
    assert!(matches!(
        journal.record_source_selection(&fixture.provision, &selected),
        Err(SupervisorError::JournalFull)
    ));
    assert!(journal.source_selection(&fixture.provision).is_none());
    assert!(!journal.target_established(&selected.target_key));
}
