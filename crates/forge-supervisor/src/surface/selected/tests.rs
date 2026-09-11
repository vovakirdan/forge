use super::*;
use crate::surface::pinned_fixture::git;
use crate::{
    git::GitBackend,
    surface::{self, pinned_fixture::Fixture},
};
use forge_domain::{
    git::{
        GitBranchRef, GitInitialRevision, GitObjectFormat, GitObjectId, GitSourceRequest,
        LocalGitPath, TaskGitSourcePolicy,
    },
    runtime::{SurfaceAccess, SurfaceSpec},
};
use uuid::Uuid;

async fn export(fixture: &Fixture, request: GitSourceRequest, name: &str) -> PreparedSource {
    let backend = GitBackend::default();
    let selection = backend.select_source(&request, false).await.unwrap();
    let directory = fixture.root.join(name);
    let descriptor = backend.export_source(&selection, &directory).await.unwrap();
    PreparedSource {
        directory,
        descriptor,
    }
}

#[tokio::test]
async fn new_source_delivery_preserves_retained_head_index_and_dirty_files() {
    let mut fixture = Fixture::new().await;
    let base = GitObjectId::new(git(&fixture.source, &["rev-parse", "HEAD"]).await).unwrap();
    let request = GitSourceRequest {
        repository_id: Uuid::now_v7(),
        source: LocalGitPath::new(fixture.source.to_str().unwrap()).unwrap(),
        target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
        initial_revision: base.clone().into(),
        policy_revision: 1,
        policy: TaskGitSourcePolicy::LatestTarget,
    };
    fixture.spec.schema_version = 6;
    fixture.spec.source_request = Some(request.clone());
    fixture.spec.binding.surface = SurfaceSpec::GitWorktree {
        repository: fixture.source.to_str().unwrap().into(),
        base_ref: base.as_str().into(),
    };
    fixture.spec.binding.access = SurfaceAccess::ReadWrite;
    fs::write(fixture.writer.join("staged"), "staged employee content").unwrap();
    git(&fixture.writer, &["add", "staged"]).await;
    fs::write(fixture.writer.join("tracked.txt"), "dirty employee content").unwrap();
    fs::write(
        fixture.writer.join("untracked"),
        "untracked employee content",
    )
    .unwrap();
    let head = git(&fixture.writer, &["rev-parse", "HEAD"]).await;
    let index_path = fixture
        .writer
        .parent()
        .unwrap()
        .join("repository.git/worktrees/worktree/index");
    let index = fs::read(&index_path).unwrap();
    let status = git(&fixture.writer, &["status", "--porcelain"]).await;
    let source = export(&fixture, request, "retained-export").await;
    let prepared = surface::prepare_selected(
        &fixture.root,
        &fixture.provision,
        &fixture.spec,
        Some(&source),
    )
    .await
    .unwrap();
    assert_eq!(prepared.mount.unwrap().join("worktree"), fixture.writer);
    assert_eq!(git(&fixture.writer, &["rev-parse", "HEAD"]).await, head);
    assert_eq!(fs::read(index_path).unwrap(), index);
    assert_eq!(
        git(&fixture.writer, &["status", "--porcelain"]).await,
        status
    );
    assert_eq!(
        fs::read_to_string(fixture.writer.join("tracked.txt")).unwrap(),
        "dirty employee content"
    );
}

#[tokio::test]
async fn unborn_surface_has_no_head_until_employee_commits_and_can_be_reviewed() {
    for format in [GitObjectFormat::Sha1, GitObjectFormat::Sha256] {
        unborn_surface_roundtrip(format).await;
    }
}

async fn unborn_surface_roundtrip(object_format: GitObjectFormat) {
    let mut fixture = Fixture::new().await;
    let source = fixture.root.join("empty.git");
    git(
        &fixture.root,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            &format!("--object-format={}", crate::git::format_name(object_format)),
            source.to_str().unwrap(),
        ],
    )
    .await;
    let request = GitSourceRequest {
        repository_id: Uuid::now_v7(),
        source: LocalGitPath::new(source.to_str().unwrap()).unwrap(),
        target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
        initial_revision: GitInitialRevision::Unborn { object_format },
        policy_revision: 1,
        policy: TaskGitSourcePolicy::LatestTarget,
    };
    fixture.spec.schema_version = 6;
    fixture.spec.surface_id = Uuid::now_v7();
    fixture.spec.source_request = Some(request.clone());
    fixture.spec.binding.surface = SurfaceSpec::GitUnborn {
        repository: source.to_str().unwrap().into(),
        object_format,
    };
    fixture.spec.binding.access = SurfaceAccess::ReadWrite;
    fixture.provision.run_spec_version = 6;
    fixture.provision.assignment = Some(
        forge_protocol::supervisor::v1::provision_run::Assignment::TaskStage(
            forge_protocol::supervisor::v1::TaskStageExecutionAssignment {
                task_id: fixture.provision.task_id.clone(),
                stage_id: fixture.provision.stage_id.clone(),
                queue_entry_id: Uuid::now_v7().to_string(),
            },
        ),
    );
    let prepared_source = export(&fixture, request, "unborn-export").await;
    let prepared = surface::prepare_selected(
        &fixture.root,
        &fixture.provision,
        &fixture.spec,
        Some(&prepared_source),
    )
    .await
    .unwrap();
    let writer = prepared.mount.unwrap().join("worktree");
    let absent = tokio::process::Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(&writer)
        .output()
        .await
        .unwrap();
    assert!(!absent.status.success());
    fs::write(writer.join("result"), "actual employee output").unwrap();
    git(&writer, &["add", "result"]).await;
    git(&writer, &["commit", "-m", "Employee root commit"]).await;
    let commit = GitObjectId::new(git(&writer, &["rev-parse", "HEAD"]).await).unwrap();
    let candidate = GitBackend::default()
        .verify_candidate(&writer, &commit, true)
        .await
        .unwrap();
    assert_eq!(
        git(&writer, &["rev-list", "--parents", "-n", "1", "HEAD"]).await,
        commit.as_str()
    );
    let staging = fixture.root.join("first-publication");
    let branch = GitBranchRef::new("refs/heads/main").unwrap();
    let backend = GitBackend::default();
    let intent = backend
        .prepare_integration(crate::git::GitIntegrationRequest {
            target: &source,
            branch: &branch,
            candidate_source: &writer,
            candidate: &candidate,
            staging_directory: &staging,
            operation_id: Uuid::now_v7(),
            committed_at: forge_domain::Timestamp::now_utc(),
            allow_initial_publication: true,
        })
        .await
        .unwrap();
    assert!(intent.expected_target.is_none());
    assert_eq!(
        backend
            .apply_integration(&source, &staging, &intent)
            .await
            .unwrap(),
        forge_domain::git::GitIntegrationState::Applied
    );
    fixture.spec.source_request = None;
    fixture.spec.binding.access = SurfaceAccess::ReadOnly;
    fixture.spec.binding.surface = SurfaceSpec::GitUnbornCandidateSnapshot {
        repository: source.to_str().unwrap().into(),
        object_format,
        candidate,
    };
    fixture.provision.run_id = Uuid::now_v7().to_string();
    let snapshot =
        surface::prepare_selected(&fixture.root, &fixture.provision, &fixture.spec, None)
            .await
            .unwrap();
    assert!(snapshot.read_only);
    assert_eq!(
        fs::read_to_string(snapshot.mount.unwrap().join("worktree/result")).unwrap(),
        "actual employee output"
    );
}

#[tokio::test]
async fn fresh_surface_starts_from_selected_latest_not_immutable_task_initial_revision() {
    let mut fixture = Fixture::new().await;
    let initial = GitObjectId::new(git(&fixture.source, &["rev-parse", "HEAD"]).await).unwrap();
    fs::write(
        fixture.source.join("later"),
        "source progressed before first writer",
    )
    .unwrap();
    git(&fixture.source, &["add", "later"]).await;
    git(
        &fixture.source,
        &["commit", "-m", "Move target before writer"],
    )
    .await;
    let latest = git(&fixture.source, &["rev-parse", "HEAD"]).await;
    let request = GitSourceRequest {
        repository_id: Uuid::now_v7(),
        source: LocalGitPath::new(fixture.source.to_str().unwrap()).unwrap(),
        target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
        initial_revision: initial.clone().into(),
        policy_revision: 1,
        policy: TaskGitSourcePolicy::LatestTarget,
    };
    fixture.spec.schema_version = 6;
    fixture.spec.surface_id = Uuid::now_v7();
    fixture.spec.source_request = Some(request.clone());
    fixture.spec.binding.surface = SurfaceSpec::GitWorktree {
        repository: fixture.source.to_str().unwrap().into(),
        base_ref: initial.as_str().into(),
    };
    fixture.spec.binding.access = SurfaceAccess::ReadWrite;
    let source = export(&fixture, request, "latest-export").await;
    let prepared = surface::prepare_selected(
        &fixture.root,
        &fixture.provision,
        &fixture.spec,
        Some(&source),
    )
    .await
    .unwrap();
    let worktree = prepared.mount.unwrap().join("worktree");
    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]).await, latest);
    assert!(worktree.join("later").is_file());
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            fixture
                .root
                .join("surface-manifests")
                .join(format!("{}.json", fixture.spec.surface_id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["source"]["base_ref"], initial.as_str());
    assert_eq!(manifest["base_commit"], latest);
}
