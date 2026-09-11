use super::*;
use forge_domain::git::{
    GitInitialRevision, GitObjectFormat, GitSourceRequest, LocalGitPath, TaskGitSourcePolicy,
};

fn request(fixture: &Fixture, policy: TaskGitSourcePolicy) -> GitSourceRequest {
    GitSourceRequest {
        repository_id: Uuid::now_v7(),
        source: LocalGitPath::new(fixture.source.to_str().unwrap()).unwrap(),
        target_ref: fixture.branch.clone(),
        initial_revision: fixture.initial.clone().into(),
        policy_revision: 1,
        policy,
    }
}

#[tokio::test]
async fn selected_revision_and_bundle_replay_do_not_follow_a_later_target() {
    let fixture = Fixture::new().await;
    let request = request(&fixture, TaskGitSourcePolicy::LatestTarget);
    let selected = fixture
        .backend
        .select_source(&request, false)
        .await
        .unwrap();
    fs::write(fixture.source.join("later"), "new target\n").unwrap();
    git(&fixture.backend, &fixture.source, &["add", "later"]).await;
    git(
        &fixture.backend,
        &fixture.source,
        &["commit", "-m", "later"],
    )
    .await;
    let newer = fixture.backend.select_source(&request, true).await.unwrap();
    assert_ne!(selected.selected_revision, newer.selected_revision);
    let output = fixture.root.join("export");
    let first = fixture
        .backend
        .export_source(&selected, &output)
        .await
        .unwrap();
    let bytes = fs::read(output.join("source.bundle")).unwrap();
    assert_eq!(
        first.selected_revision,
        GitInitialRevision::Commit(fixture.initial.clone())
    );
    assert_eq!(
        fixture
            .backend
            .export_source(&selected, &output)
            .await
            .unwrap(),
        first
    );
    assert_eq!(fs::read(output.join("source.bundle")).unwrap(), bytes);
    assert!(
        fixture
            .backend
            .export_source(&newer, &output)
            .await
            .is_err()
    );
    assert!(
        !fs::read_to_string(output.join("descriptor.json"))
            .unwrap()
            .contains(fixture.source.to_str().unwrap())
    );
}

#[tokio::test]
async fn pin_selects_exact_object_and_missing_established_target_fails_closed() {
    let fixture = Fixture::new().await;
    let request = request(
        &fixture,
        TaskGitSourcePolicy::PinnedCommit {
            commit: fixture.initial.clone(),
        },
    );
    fs::write(fixture.source.join("later"), "new target\n").unwrap();
    git(&fixture.backend, &fixture.source, &["add", "later"]).await;
    git(
        &fixture.backend,
        &fixture.source,
        &["commit", "-m", "later"],
    )
    .await;
    assert_eq!(
        fixture
            .backend
            .select_source(&request, true)
            .await
            .unwrap()
            .selected_revision,
        GitInitialRevision::Commit(fixture.initial.clone())
    );
    git(
        &fixture.backend,
        &fixture.source,
        &["update-ref", "-d", fixture.branch.as_str()],
    )
    .await;
    assert!(matches!(
        fixture.backend.select_source(&request, false).await,
        Err(GitBackendError::TargetMissing)
    ));
}

#[tokio::test]
async fn unborn_export_has_no_bundle_or_synthetic_commit() {
    let fixture = Fixture::new().await;
    let source = fixture.root.join("unborn.git");
    git(
        &fixture.backend,
        &fixture.root,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            source.to_str().unwrap(),
        ],
    )
    .await;
    let request = GitSourceRequest {
        repository_id: Uuid::now_v7(),
        source: LocalGitPath::new(source.to_str().unwrap()).unwrap(),
        target_ref: fixture.branch.clone(),
        initial_revision: GitInitialRevision::Unborn {
            object_format: GitObjectFormat::Sha1,
        },
        policy_revision: 1,
        policy: TaskGitSourcePolicy::LatestTarget,
    };
    let selected = fixture
        .backend
        .select_source(&request, false)
        .await
        .unwrap();
    let destination = fixture.root.join("unborn-export");
    let descriptor = fixture
        .backend
        .export_source(&selected, &destination)
        .await
        .unwrap();
    assert_eq!(descriptor.selected_revision, request.initial_revision);
    assert!(descriptor.bundle_file.is_none());
    assert!(!destination.join("source.bundle").exists());
    assert!(
        fixture
            .backend
            .read_ref(&source, "refs/heads/main")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        fixture.backend.select_source(&request, true).await,
        Err(GitBackendError::TargetMissing)
    ));
}
