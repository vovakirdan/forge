//! Real local Git regression; shell actions below stand in for explicit Employee actions.
use super::*;
use forge_domain::git::{
    GitInitialRevision, GitObjectFormat, GitSourceRequest, LocalGitPath, TaskGitSourcePolicy,
};

async fn root_candidate(fixture: &Fixture, source: &Path, file: &str) -> GitCandidate {
    git(
        &fixture.backend,
        &fixture.root,
        &["init", "--initial-branch=main", source.to_str().unwrap()],
    )
    .await;
    fs::write(source.join(file), format!("Employee owned {file}\n")).unwrap();
    git(&fixture.backend, source, &["add", file]).await;
    git(
        &fixture.backend,
        source,
        &["commit", "-m", "Employee root commit"],
    )
    .await;
    let commit = fixture
        .backend
        .resolve_commit(source, "HEAD")
        .await
        .unwrap();
    assert_eq!(
        git(
            &fixture.backend,
            source,
            &["rev-list", "--parents", "-n", "1", "HEAD"]
        )
        .await
        .trim(),
        commit.as_str()
    );
    fixture
        .backend
        .verify_candidate(source, &commit, true)
        .await
        .unwrap()
}

async fn prepare(
    fixture: &Fixture,
    target: &Path,
    source: &Path,
    candidate: &GitCandidate,
    stage: &Path,
) -> Result<forge_domain::git::GitIntegrationIntent, GitBackendError> {
    fixture
        .backend
        .prepare_integration(GitIntegrationRequest {
            target,
            branch: &fixture.branch,
            candidate_source: source,
            candidate,
            staging_directory: stage,
            operation_id: Uuid::now_v7(),
            committed_at: Timestamp::now_utc(),
            allow_initial_publication: true,
        })
        .await
}

#[tokio::test]
async fn independent_root_candidates_require_explicit_employee_merge_before_second_publication() {
    let fixture = Fixture::new().await;
    let target = fixture.root.join("empty-target.git");
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
    let writer_a = fixture.root.join("writer-a");
    let writer_b = fixture.root.join("writer-b");
    let candidate_a = root_candidate(&fixture, &writer_a, "normalize-lines.sh").await;
    let candidate_b = root_candidate(&fixture, &writer_b, "README.md").await;
    let stage_a = fixture.root.join("first-stage");
    let intent_a = prepare(&fixture, &target, &writer_a, &candidate_a, &stage_a)
        .await
        .unwrap();
    assert_eq!(intent_a.expected_target, None);
    fixture
        .backend
        .apply_integration(&target, &stage_a, &intent_a)
        .await
        .unwrap();
    assert!(matches!(
        prepare(
            &fixture,
            &target,
            &writer_b,
            &candidate_b,
            &fixture.root.join("stale-stage")
        )
        .await,
        Err(GitBackendError::StaleBase)
    ));
    let head_before = fixture
        .backend
        .resolve_commit(&writer_b, "HEAD")
        .await
        .unwrap();
    let selection = fixture
        .backend
        .select_source(
            &GitSourceRequest {
                repository_id: Uuid::now_v7(),
                source: LocalGitPath::new(target.to_str().unwrap()).unwrap(),
                target_ref: fixture.branch.clone(),
                initial_revision: GitInitialRevision::Unborn {
                    object_format: GitObjectFormat::Sha1,
                },
                policy_revision: 1,
                policy: TaskGitSourcePolicy::LatestTarget,
            },
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        selection.selected_revision,
        GitInitialRevision::Commit(candidate_a.commit.clone())
    );
    let delivery = fixture.root.join("new-source");
    fixture
        .backend
        .export_source(&selection, &delivery)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .backend
            .resolve_commit(&writer_b, "HEAD")
            .await
            .unwrap(),
        head_before
    );
    // Core never executes these employee-owned merge choices or silently resets files.
    git(
        &fixture.backend,
        &writer_b,
        &[
            "fetch",
            delivery.join("source.bundle").to_str().unwrap(),
            "+refs/forge/source:refs/forge/source",
        ],
    )
    .await;
    git(
        &fixture.backend,
        &writer_b,
        &[
            "merge",
            "--allow-unrelated-histories",
            "--no-edit",
            candidate_a.commit.as_str(),
        ],
    )
    .await;
    let reworked_head = fixture
        .backend
        .resolve_commit(&writer_b, "HEAD")
        .await
        .unwrap();
    let reworked = fixture
        .backend
        .verify_candidate(&writer_b, &reworked_head, true)
        .await
        .unwrap();
    assert!(writer_b.join("normalize-lines.sh").is_file() && writer_b.join("README.md").is_file());
    let stage_b = fixture.root.join("reworked-stage");
    let intent_b = prepare(&fixture, &target, &writer_b, &reworked, &stage_b)
        .await
        .unwrap();
    assert_eq!(intent_b.expected_target, Some(candidate_a.commit.clone()));
    assert_eq!(
        fixture
            .backend
            .apply_integration(&target, &stage_b, &intent_b)
            .await
            .unwrap(),
        GitIntegrationState::Applied
    );
    assert_eq!(
        fixture
            .backend
            .tree(&target, &intent_b.merge_commit)
            .await
            .unwrap(),
        reworked.tree
    );
    assert!(
        fixture
            .backend
            .ancestor(&target, &candidate_a.commit, &intent_b.merge_commit)
            .await
            .unwrap()
    );
    assert!(
        fixture
            .backend
            .ancestor(&target, &candidate_b.commit, &intent_b.merge_commit)
            .await
            .unwrap()
    );
}
