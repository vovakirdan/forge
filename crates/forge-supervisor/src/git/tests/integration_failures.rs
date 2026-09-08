use super::*;

#[tokio::test]
async fn stale_target_requires_employee_merge_and_no_changes_is_explicit() {
    let fixture = Fixture::new().await;
    let unchanged = fixture
        .backend
        .verify_candidate(&fixture.worker, &fixture.initial, true)
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .backend
            .prepare_integration(fixture.request(
                &unchanged,
                &fixture.root.join("unchanged"),
                &fixture.target
            ))
            .await,
        Err(GitBackendError::NoChanges)
    ));
    let candidate = fixture.deliver("task.txt").await;
    fs::write(fixture.source.join("other.txt"), "concurrent Task\n").unwrap();
    git(&fixture.backend, &fixture.source, &["add", "other.txt"]).await;
    git(
        &fixture.backend,
        &fixture.source,
        &["commit", "-m", "Concurrent Task"],
    )
    .await;
    git(
        &fixture.backend,
        &fixture.source,
        &[
            "push",
            fixture.target.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    assert!(matches!(
        fixture
            .backend
            .prepare_integration(fixture.request(
                &candidate,
                &fixture.root.join("stale"),
                &fixture.target
            ))
            .await,
        Err(GitBackendError::StaleBase)
    ));
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&fixture.worker, "HEAD")
            .await
            .unwrap(),
        candidate.commit
    );
}

#[tokio::test]
async fn target_movement_after_prepare_never_gets_overwritten() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("task.txt").await;
    let staging = fixture.root.join("prepared");
    let intent = fixture.prepare(&candidate, &staging).await;
    fs::write(fixture.source.join("other.txt"), "concurrent Task\n").unwrap();
    git(&fixture.backend, &fixture.source, &["add", "other.txt"]).await;
    git(
        &fixture.backend,
        &fixture.source,
        &["commit", "-m", "Concurrent Task"],
    )
    .await;
    git(
        &fixture.backend,
        &fixture.source,
        &[
            "push",
            fixture.target.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    let target = fixture
        .backend
        .resolve_commit(&fixture.target, "main")
        .await
        .unwrap();
    assert_eq!(
        fixture
            .backend
            .apply_integration(&fixture.target, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Unknown
    );
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&fixture.target, "main")
            .await
            .unwrap(),
        target
    );
}

#[tokio::test]
async fn receipt_without_target_update_is_not_completion_or_safe_retry() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("task.txt").await;
    let staging = fixture.root.join("prepared");
    let intent = fixture.prepare(&candidate, &staging).await;
    fixture
        .backend
        .fetch_object(
            &fixture.target,
            &staging.join("repository.git"),
            &intent.merge_commit,
        )
        .await
        .unwrap();
    git(
        &fixture.backend,
        &fixture.target,
        &[
            "update-ref",
            &intent.receipt_ref(),
            intent.merge_commit.as_str(),
        ],
    )
    .await;
    assert_eq!(
        fixture
            .backend
            .reconcile_integration(&fixture.target, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Unknown
    );
    assert_eq!(
        fixture
            .backend
            .apply_integration(&fixture.target, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Unknown
    );
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&fixture.target, "main")
            .await
            .unwrap(),
        fixture.initial
    );
}

#[tokio::test]
async fn target_without_receipt_and_later_descendant_prove_applied_merge() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("task.txt").await;
    let staging = fixture.root.join("prepared");
    let intent = fixture.prepare(&candidate, &staging).await;
    fixture
        .backend
        .fetch_object(
            &fixture.target,
            &staging.join("repository.git"),
            &intent.merge_commit,
        )
        .await
        .unwrap();
    git(
        &fixture.backend,
        &fixture.target,
        &[
            "update-ref",
            fixture.branch.as_str(),
            intent.merge_commit.as_str(),
            fixture.initial.as_str(),
        ],
    )
    .await;
    assert_eq!(
        fixture
            .backend
            .reconcile_integration(&fixture.target, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Applied
    );
    git(
        &fixture.backend,
        &fixture.source,
        &["fetch", fixture.target.to_str().unwrap(), "main"],
    )
    .await;
    git(
        &fixture.backend,
        &fixture.source,
        &["merge", "--ff-only", "FETCH_HEAD"],
    )
    .await;
    fs::write(fixture.source.join("later.txt"), "later change\n").unwrap();
    git(&fixture.backend, &fixture.source, &["add", "later.txt"]).await;
    git(
        &fixture.backend,
        &fixture.source,
        &["commit", "-m", "Later change"],
    )
    .await;
    git(
        &fixture.backend,
        &fixture.source,
        &[
            "push",
            fixture.target.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    assert_eq!(
        fixture
            .backend
            .reconcile_integration(&fixture.target, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Applied
    );
}

#[tokio::test]
async fn tampered_intent_cannot_publish_a_different_tree() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("task.txt").await;
    let staging = fixture.root.join("prepared");
    let mut intent = fixture.prepare(&candidate, &staging).await;
    intent.candidate.tree = fixture
        .backend
        .tree(&fixture.worker, &fixture.initial)
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .backend
            .apply_integration(&fixture.target, &staging, &intent)
            .await,
        Err(GitBackendError::InvalidIntent)
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

#[tokio::test]
async fn symbolic_target_is_rejected_instead_of_following_another_branch() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("task.txt").await;
    git(
        &fixture.backend,
        &fixture.target,
        &["update-ref", "refs/heads/other", fixture.initial.as_str()],
    )
    .await;
    git(
        &fixture.backend,
        &fixture.target,
        &["symbolic-ref", fixture.branch.as_str(), "refs/heads/other"],
    )
    .await;
    assert!(matches!(
        fixture
            .backend
            .prepare_integration(fixture.request(
                &candidate,
                &fixture.root.join("prepared"),
                &fixture.target
            ))
            .await,
        Err(GitBackendError::SymbolicTarget)
    ));
}
