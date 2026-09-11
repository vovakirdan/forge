use super::*;

async fn empty_target(fixture: &Fixture) -> PathBuf {
    let target = fixture.root.join("unborn.git");
    git(
        &fixture.backend,
        &fixture.root,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            target.to_str().unwrap(),
        ],
    )
    .await;
    target
}

#[tokio::test]
async fn first_publication_requires_explicit_unborn_and_creates_exact_candidate_once() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("feature").await;
    let target = empty_target(&fixture).await;
    let staging = fixture.root.join("first");
    assert!(matches!(
        fixture
            .backend
            .prepare_integration(fixture.request(&candidate, &staging, &target))
            .await,
        Err(GitBackendError::InvalidIntent)
    ));
    let mut request = fixture.request(&candidate, &staging, &target);
    request.allow_initial_publication = true;
    let intent = fixture.backend.prepare_integration(request).await.unwrap();
    assert!(intent.expected_target.is_none());
    assert_eq!(intent.merge_commit, candidate.commit);
    assert_eq!(
        fixture
            .backend
            .reconcile_integration(&target, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Retryable
    );
    assert_eq!(
        fixture
            .backend
            .apply_integration(&target, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Applied
    );
    assert_eq!(
        fixture
            .backend
            .apply_integration(&target, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Applied
    );
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&target, fixture.branch.as_str())
            .await
            .unwrap(),
        candidate.commit
    );
}

#[tokio::test]
async fn competing_first_publication_preserves_winner_and_returns_stale_base() {
    let fixture = Fixture::new().await;
    let first = fixture.deliver("first").await;
    let second = fixture.deliver("second").await;
    let target = empty_target(&fixture).await;
    let stage_a = fixture.root.join("publication-a");
    let stage_b = fixture.root.join("publication-b");
    let mut a = fixture.request(&first, &stage_a, &target);
    a.allow_initial_publication = true;
    let mut b = fixture.request(&second, &stage_b, &target);
    b.allow_initial_publication = true;
    let intent_a = fixture.backend.prepare_integration(a).await.unwrap();
    let intent_b = fixture.backend.prepare_integration(b).await.unwrap();
    fixture
        .backend
        .apply_integration(&target, &stage_a, &intent_a)
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .backend
            .apply_integration(&target, &stage_b, &intent_b)
            .await,
        Err(GitBackendError::StaleBase)
    ));
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&target, fixture.branch.as_str())
            .await
            .unwrap(),
        first.commit
    );
}

#[tokio::test]
async fn contradictory_first_publication_evidence_is_unknown_not_safe_rework() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("candidate").await;
    let competing = fixture.deliver("competing").await;
    let target = empty_target(&fixture).await;
    let staging = fixture.root.join("contradictory-first");
    let mut request = fixture.request(&candidate, &staging, &target);
    request.allow_initial_publication = true;
    let intent = fixture.backend.prepare_integration(request).await.unwrap();
    fixture
        .backend
        .fetch_object(&target, &fixture.worker, &competing.commit)
        .await
        .unwrap();
    git(
        &fixture.backend,
        &target,
        &[
            "update-ref",
            fixture.branch.as_str(),
            competing.commit.as_str(),
        ],
    )
    .await;
    git(
        &fixture.backend,
        &target,
        &[
            "update-ref",
            &intent.receipt_ref(),
            competing.commit.as_str(),
        ],
    )
    .await;
    assert_eq!(
        fixture
            .backend
            .apply_integration(&target, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Unknown
    );
    git(
        &fixture.backend,
        &target,
        &["update-ref", "-d", &intent.receipt_ref()],
    )
    .await;
    git(
        &fixture.backend,
        &target,
        &["update-ref", "refs/heads/other", competing.commit.as_str()],
    )
    .await;
    git(
        &fixture.backend,
        &target,
        &["symbolic-ref", fixture.branch.as_str(), "refs/heads/other"],
    )
    .await;
    assert_eq!(
        fixture
            .backend
            .apply_integration(&target, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Unknown
    );
}
