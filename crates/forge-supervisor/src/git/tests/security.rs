use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;

#[tokio::test]
async fn two_candidates_share_a_physical_target_lock_but_not_a_workspace() {
    let fixture = Fixture::new().await;
    let first = fixture.deliver("first.txt").await;
    let first_staging = fixture.root.join("first-integration");
    let first_intent = fixture.prepare(&first, &first_staging).await;
    let second_worker = fixture.root.join("second-worker");
    git(
        &fixture.backend,
        &fixture.root,
        &[
            "clone",
            "--no-local",
            fixture.target.to_str().unwrap(),
            second_worker.to_str().unwrap(),
        ],
    )
    .await;
    fs::write(second_worker.join("second.txt"), "Other employee\n").unwrap();
    git(&fixture.backend, &second_worker, &["add", "second.txt"]).await;
    git(
        &fixture.backend,
        &second_worker,
        &["commit", "-m", "Other employee"],
    )
    .await;
    let head = fixture
        .backend
        .resolve_commit(&second_worker, "HEAD")
        .await
        .unwrap();
    let second = fixture
        .backend
        .verify_candidate(&second_worker, &head, true)
        .await
        .unwrap();
    let second_staging = fixture.root.join("second-integration");
    let mut request = fixture.request(&second, &second_staging, &fixture.target);
    request.candidate_source = &second_worker;
    let second_intent = fixture.backend.prepare_integration(request).await.unwrap();
    let (left, right) = tokio::join!(
        fixture
            .backend
            .apply_integration(&fixture.target, &first_staging, &first_intent),
        fixture
            .backend
            .apply_integration(&fixture.target, &second_staging, &second_intent),
    );
    let states = [left, right];
    assert_eq!(
        states
            .iter()
            .filter(|state| matches!(state, Ok(GitIntegrationState::Applied)))
            .count(),
        1
    );
    assert!(states.iter().all(|state| matches!(
        state,
        Ok(GitIntegrationState::Applied | GitIntegrationState::Unknown)
            | Err(GitBackendError::IntegrationBusy)
    )));
    assert!(!fixture.worker.join("second.txt").exists());
    assert!(!second_worker.join("first.txt").exists());
}

#[tokio::test]
async fn filesystem_monitor_and_git_hooks_never_execute_implicitly() {
    let fixture = Fixture::new().await;
    let candidate = fixture.deliver("task.txt").await;
    let hook = fixture.root.join("forbidden-hook");
    let marker = fixture.root.join("hook-ran");
    fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    git(
        &fixture.backend,
        &fixture.worker,
        &["config", "core.fsmonitor", hook.to_str().unwrap()],
    )
    .await;
    let hooks = fixture.target.join("hooks");
    fs::create_dir(&hooks).unwrap();
    fs::copy(&hook, hooks.join("reference-transaction")).unwrap();
    fs::set_permissions(
        hooks.join("reference-transaction"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert_eq!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &candidate.commit, true)
            .await
            .unwrap(),
        candidate
    );
    let staging = fixture.root.join("prepared");
    let intent = fixture.prepare(&candidate, &staging).await;
    assert_eq!(
        fixture
            .backend
            .apply_integration(&fixture.target, &staging, &intent)
            .await
            .unwrap(),
        GitIntegrationState::Applied
    );
    assert!(!marker.exists());
}

#[tokio::test]
async fn private_git_metadata_cannot_redirect_host_object_reads() {
    let fixture = Fixture::new().await;
    let alternates = fixture.worker.join(".git/objects/info/alternates");
    fs::write(
        &alternates,
        fixture.source.join(".git/objects").to_str().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &fixture.initial, true)
            .await,
        Err(GitBackendError::UnsafePath)
    ));
    // The test owns this one injected file; no production operation cleans it up.
    fs::remove_file(alternates).unwrap();
    symlink(
        fixture.source.join("README.md"),
        fixture.worker.join(".git/host-link"),
    )
    .unwrap();
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &fixture.initial, true)
            .await,
        Err(GitBackendError::UnsafePath)
    ));
}

#[tokio::test]
async fn protocol_output_is_bounded_without_exposing_repository_content() {
    let fixture = Fixture::new().await;
    fs::write(
        fixture.worker.join("large.txt"),
        vec![b'x'; 1024 * 1024 + 1],
    )
    .unwrap();
    git(&fixture.backend, &fixture.worker, &["add", "large.txt"]).await;
    git(
        &fixture.backend,
        &fixture.worker,
        &["commit", "-m", "Large fixture"],
    )
    .await;
    assert!(matches!(
        fixture
            .backend
            .command(&fixture.worker, &["show", "HEAD:large.txt"], &[], &[])
            .await,
        Err(GitBackendError::OutputLimit)
    ));
}

#[tokio::test]
async fn command_timeout_kills_git_and_its_process_group() {
    let fixture = Fixture::new().await;
    let backend = GitBackend::with_timeout(std::time::Duration::from_millis(30)).unwrap();
    // This explicit test-only alias supplies a deterministic slow subprocess;
    // public production methods invoke only fixed built-in Git operations.
    let started = std::time::Instant::now();
    assert!(matches!(
        backend
            .command(
                &fixture.root,
                &["-c", "alias.fixture-wait=!sleep 5", "fixture-wait"],
                &[],
                &[]
            )
            .await,
        Err(GitBackendError::Timeout)
    ));
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}

#[tokio::test]
async fn worker_clean_and_process_filters_never_execute_on_host() {
    for filter in ["clean", "process"] {
        let fixture = Fixture::new().await;
        fs::write(
            fixture.worker.join(".gitattributes"),
            "README.md filter=host-command\n",
        )
        .unwrap();
        git(
            &fixture.backend,
            &fixture.worker,
            &["add", ".gitattributes"],
        )
        .await;
        git(
            &fixture.backend,
            &fixture.worker,
            &["commit", "-m", "Project attributes"],
        )
        .await;
        let head = fixture
            .backend
            .resolve_commit(&fixture.worker, "HEAD")
            .await
            .unwrap();
        let marker = fixture.root.join("host-filter-ran");
        let key = format!("filter.host-command.{filter}");
        let command = format!("touch {}; cat", marker.display());
        git(
            &fixture.backend,
            &fixture.worker,
            &["config", &key, &command],
        )
        .await;
        fs::write(fixture.worker.join("README.md"), "Force index refresh\n").unwrap();
        let before = fs::read(fixture.worker.join(".git/config")).unwrap();
        assert!(matches!(
            fixture
                .backend
                .verify_candidate(&fixture.worker, &head, true)
                .await,
            Err(GitBackendError::ExternalFilter)
        ));
        assert!(!marker.exists());
        assert_eq!(
            fs::read(fixture.worker.join(".git/config")).unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn filters_from_config_includes_and_worktree_config_are_rejected() {
    let fixture = Fixture::new().await;
    let included = fixture.root.join("included-config");
    fs::write(
        &included,
        "[filter \"included\"]\nclean = host-command-must-not-run\n",
    )
    .unwrap();
    git(
        &fixture.backend,
        &fixture.worker,
        &["config", "include.path", included.to_str().unwrap()],
    )
    .await;
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &fixture.initial, true)
            .await,
        Err(GitBackendError::ExternalFilter)
    ));
    git(
        &fixture.backend,
        &fixture.worker,
        &["config", "--unset", "include.path"],
    )
    .await;
    git(
        &fixture.backend,
        &fixture.worker,
        &["config", "extensions.worktreeConfig", "true"],
    )
    .await;
    git(
        &fixture.backend,
        &fixture.worker,
        &[
            "config",
            "--worktree",
            "filter.worktree.process",
            "host-command-must-not-run",
        ],
    )
    .await;
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &fixture.initial, true)
            .await,
        Err(GitBackendError::ExternalFilter)
    ));
}

#[tokio::test]
async fn configured_worktree_cannot_redirect_status_to_another_directory() {
    let fixture = Fixture::new().await;
    git(
        &fixture.backend,
        &fixture.worker,
        &["config", "core.worktree", fixture.source.to_str().unwrap()],
    )
    .await;
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &fixture.initial, true)
            .await,
        Err(GitBackendError::UnsafePath)
    ));
}

#[tokio::test]
async fn nested_repository_status_is_rejected_before_loading_submodule_config() {
    let fixture = Fixture::new().await;
    git(
        &fixture.backend,
        &fixture.worker,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},nested", fixture.initial.as_str()),
        ],
    )
    .await;
    git(
        &fixture.backend,
        &fixture.worker,
        &["commit", "-m", "Gitlink fixture"],
    )
    .await;
    let head = fixture
        .backend
        .resolve_commit(&fixture.worker, "HEAD")
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .backend
            .verify_candidate(&fixture.worker, &head, true)
            .await,
        Err(GitBackendError::SubmodulesUnsupported)
    ));
}
