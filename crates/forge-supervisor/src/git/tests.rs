use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

use forge_domain::{
    Timestamp,
    git::{GitBranchRef, GitCandidate, GitIntegrationState, GitObjectId},
};
use uuid::Uuid;

use super::{GitBackend, GitBackendError, GitIntegrationRequest};

mod integration_failures;
mod security;
mod source;
mod unborn;
mod unborn_workflow;

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    target: PathBuf,
    worker: PathBuf,
    backend: GitBackend,
    branch: GitBranchRef,
    initial: GitObjectId,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    async fn new() -> Self {
        let root = PathBuf::from(format!("/tmp/forge-git-test-{}", Uuid::now_v7()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let source = root.join("source");
        let target = root.join("target.git");
        let worker = root.join("worker");
        let backend = GitBackend::default();
        backend
            .text(
                &root,
                &["init", "--initial-branch=main", source.to_str().unwrap()],
                "fixture init",
            )
            .await
            .unwrap();
        fs::write(source.join("README.md"), "Initial project\n").unwrap();
        git(&backend, &source, &["add", "README.md"]).await;
        git(&backend, &source, &["commit", "-m", "Initial commit"]).await;
        let initial = backend.resolve_commit(&source, "HEAD").await.unwrap();
        backend
            .text(
                &root,
                &[
                    "clone",
                    "--bare",
                    "--no-local",
                    "--",
                    source.to_str().unwrap(),
                    target.to_str().unwrap(),
                ],
                "fixture target",
            )
            .await
            .unwrap();
        backend
            .text(
                &root,
                &[
                    "clone",
                    "--no-local",
                    "--",
                    target.to_str().unwrap(),
                    worker.to_str().unwrap(),
                ],
                "fixture worker",
            )
            .await
            .unwrap();
        Self {
            root,
            source,
            target,
            worker,
            backend,
            branch: GitBranchRef::new("refs/heads/main").unwrap(),
            initial,
        }
    }

    async fn deliver(&self, name: &str) -> GitCandidate {
        fs::write(
            self.worker.join(name),
            format!("Employee delivered {name}\n"),
        )
        .unwrap();
        git(&self.backend, &self.worker, &["add", name]).await;
        git(&self.backend, &self.worker, &["commit", "-m", name]).await;
        let head = self
            .backend
            .resolve_commit(&self.worker, "HEAD")
            .await
            .unwrap();
        self.backend
            .verify_candidate(&self.worker, &head, true)
            .await
            .unwrap()
    }

    async fn prepare(
        &self,
        candidate: &GitCandidate,
        staging: &Path,
    ) -> forge_domain::git::GitIntegrationIntent {
        self.backend
            .prepare_integration(self.request(candidate, staging, &self.target))
            .await
            .unwrap()
    }

    fn request<'a>(
        &'a self,
        candidate: &'a GitCandidate,
        staging: &'a Path,
        target: &'a Path,
    ) -> GitIntegrationRequest<'a> {
        GitIntegrationRequest {
            target,
            branch: &self.branch,
            candidate_source: &self.worker,
            candidate,
            staging_directory: staging,
            operation_id: Uuid::now_v7(),
            committed_at: Timestamp::from_offset_date_time(
                time::OffsetDateTime::from_unix_timestamp(1_788_710_400).unwrap(),
            ),
            allow_initial_publication: false,
        }
    }
}

async fn git(backend: &GitBackend, directory: &Path, args: &[&str]) -> String {
    let output = backend
        .command(
            directory,
            args,
            &[],
            &[
                ("GIT_AUTHOR_NAME", "Fixture Employee"),
                ("GIT_AUTHOR_EMAIL", "employee@example.invalid"),
                ("GIT_COMMITTER_NAME", "Fixture Employee"),
                ("GIT_COMMITTER_EMAIL", "employee@example.invalid"),
                ("GIT_AUTHOR_DATE", "@1788710400 +0000"),
                ("GIT_COMMITTER_DATE", "@1788710400 +0000"),
            ],
        )
        .await
        .unwrap();
    String::from_utf8(output.require_success("test fixture").unwrap()).unwrap()
}

#[tokio::test]
async fn candidate_requires_quiescence_exact_head_and_committed_clean_work() {
    let fixture = Fixture::new().await;
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &fixture.initial, false)
            .await,
        Err(GitBackendError::WriterActive)
    ));
    let candidate = fixture.deliver("employee.txt").await;
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &fixture.initial, true)
            .await,
        Err(GitBackendError::HeadChanged)
    ));
    fs::write(fixture.worker.join("untracked.txt"), "retained work").unwrap();
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &candidate.commit, true)
            .await,
        Err(GitBackendError::DirtyCandidate)
    ));
    assert_eq!(
        fs::read_to_string(fixture.worker.join("untracked.txt")).unwrap(),
        "retained work"
    );
}

#[tokio::test]
async fn candidate_rejects_staged_and_hidden_tracked_changes() {
    let fixture = Fixture::new().await;
    fs::write(fixture.worker.join("README.md"), "changed").unwrap();
    git(&fixture.backend, &fixture.worker, &["add", "README.md"]).await;
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &fixture.initial, true)
            .await,
        Err(GitBackendError::DirtyCandidate)
    ));
    git(
        &fixture.backend,
        &fixture.worker,
        &["commit", "-m", "Changed"],
    )
    .await;
    let head = fixture
        .backend
        .resolve_commit(&fixture.worker, "HEAD")
        .await
        .unwrap();
    git(
        &fixture.backend,
        &fixture.worker,
        &["update-index", "--assume-unchanged", "README.md"],
    )
    .await;
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &head, true)
            .await,
        Err(GitBackendError::DirtyCandidate)
    ));
}

#[tokio::test]
async fn snapshots_pin_old_commit_and_do_not_copy_ignored_or_later_changes() {
    let fixture = Fixture::new().await;
    fs::write(fixture.worker.join(".gitignore"), "ignored.txt\n").unwrap();
    git(&fixture.backend, &fixture.worker, &["add", ".gitignore"]).await;
    git(
        &fixture.backend,
        &fixture.worker,
        &["commit", "-m", "Ignore local files"],
    )
    .await;
    let candidate = fixture.deliver("first.txt").await;
    fs::write(fixture.worker.join("ignored.txt"), "private scratch").unwrap();
    fixture.deliver("later.txt").await;
    let destination = fixture.root.join("snapshot");
    let snapshot = fixture
        .backend
        .snapshot_candidate(&fixture.worker, &candidate, &destination)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&snapshot, "HEAD")
            .await
            .unwrap(),
        candidate.commit
    );
    assert!(snapshot.join("first.txt").is_file());
    assert!(!snapshot.join("later.txt").exists());
    assert!(!snapshot.join("ignored.txt").exists());
    fs::write(snapshot.join("first.txt"), "QA scratch edit").unwrap();
    assert_ne!(
        fs::read(snapshot.join("first.txt")).unwrap(),
        fs::read(fixture.worker.join("first.txt")).unwrap()
    );
    assert!(matches!(
        fixture
            .backend
            .snapshot_candidate(&fixture.worker, &candidate, &destination)
            .await,
        Err(GitBackendError::DestinationExists)
    ));
}

#[tokio::test]
async fn integration_preserves_worker_history_and_exact_approved_tree() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("implementation.txt").await;
    let source_refs = git(&fixture.backend, &fixture.source, &["show-ref"]).await;
    let target_refs = git(&fixture.backend, &fixture.target, &["show-ref"]).await;
    let staging = fixture.root.join("prepared");
    let intent = fixture.prepare(&candidate, &staging).await;
    assert_eq!(
        git(&fixture.backend, &fixture.source, &["show-ref"]).await,
        source_refs
    );
    assert_eq!(
        git(&fixture.backend, &fixture.target, &["show-ref"]).await,
        target_refs
    );
    assert_eq!(
        fixture
            .backend
            .reconcile_integration(&fixture.target, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Retryable
    );
    assert_eq!(
        fixture
            .backend
            .apply_integration(&fixture.target, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Applied
    );
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&fixture.target, fixture.branch.as_str())
            .await
            .unwrap(),
        intent.merge_commit
    );
    assert_eq!(
        fixture
            .backend
            .tree(&fixture.target, &intent.merge_commit)
            .await
            .unwrap(),
        candidate.tree
    );
    let parents = git(
        &fixture.backend,
        &fixture.target,
        &[
            "rev-list",
            "--parents",
            "-n",
            "1",
            intent.merge_commit.as_str(),
        ],
    )
    .await;
    assert_eq!(
        parents.trim(),
        format!(
            "{} {} {}",
            intent.merge_commit.as_str(),
            fixture.initial.as_str(),
            candidate.commit.as_str()
        )
    );
    let refs = git(&fixture.backend, &fixture.target, &["show-ref"]).await;
    assert_eq!(
        fixture
            .backend
            .apply_integration(&fixture.target, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Applied
    );
    assert_eq!(
        git(&fixture.backend, &fixture.target, &["show-ref"]).await,
        refs
    );
}

#[tokio::test]
async fn ordinary_target_is_allowed_only_when_branch_is_not_checked_out() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("implementation.txt").await;
    let staging = fixture.root.join("ordinary-prepared");
    assert!(matches!(
        fixture
            .backend
            .prepare_integration(fixture.request(&candidate, &staging, &fixture.source))
            .await,
        Err(GitBackendError::TargetCheckedOut)
    ));
    git(&fixture.backend, &fixture.source, &["checkout", "--detach"]).await;
    let intent = fixture
        .backend
        .prepare_integration(fixture.request(&candidate, &staging, &fixture.source))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .backend
            .apply_integration(&fixture.source, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Applied
    );
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&fixture.source, "HEAD")
            .await
            .unwrap(),
        fixture.initial
    );
    assert!(!fixture.source.join("implementation.txt").exists());
}

#[tokio::test]
async fn linked_worktree_blocks_target_even_for_a_bare_repository() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("implementation.txt").await;
    let staging = fixture.root.join("prepared");
    let intent = fixture.prepare(&candidate, &staging).await;
    let checkout = fixture.root.join("linked");
    git(
        &fixture.backend,
        &fixture.target,
        &["worktree", "add", checkout.to_str().unwrap(), "main"],
    )
    .await;
    assert!(matches!(
        fixture
            .backend
            .apply_integration(&fixture.target, &staging, &intent)
            .await,
        Err(GitBackendError::TargetCheckedOut)
    ));
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&fixture.target, "main")
            .await
            .unwrap(),
        fixture.initial
    );
}
