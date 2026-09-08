use super::*;
use forge_domain::runtime::SurfaceAccess;
use std::fs;

pub(in crate::podman) mod fixture;
use fixture::{Fixture, git};

#[tokio::test]
async fn verified_receipt_survives_tombstone_ack_restart_and_does_not_reinspect() {
    let mut fixture = Fixture::new().await;
    let before = fixture.registry.journal.lock().await.records().remove(0);
    assert!(
        before.provision.run_spec_json.is_empty(),
        "prompt compacted"
    );
    assert!(before.git_source.is_some(), "source retained separately");
    let result = fixture.inspect(fixture.request.clone()).await;
    assert_eq!(result.result_code, Code::Verified as i32);
    assert_eq!(result.commit, fixture.request.expected_commit);
    assert_eq!(
        result.tree,
        git(&fixture.worktree, &["rev-parse", "HEAD^{tree}"]).await
    );
    assert_eq!(
        fixture.registry.journal.lock().await.records()[0].last_sequence,
        before.last_sequence
    );
    fixture.acknowledge(&result.message_id).await;
    assert_eq!(fixture.registry.journal.lock().await.pending().count(), 0);
    fs::write(
        fixture.worktree.join("later-change"),
        "Not part of this historical receipt",
    )
    .unwrap();
    fixture.restart().await;
    assert_eq!(fixture.inspect(fixture.request.clone()).await, result);
    let mut retry = fixture.request.clone();
    retry.command_id = crate::new_id();
    assert_eq!(
        fixture.inspect(retry).await.result_code,
        Code::DirtyCandidate as i32
    );
}

#[tokio::test]
async fn live_unknown_mismatched_and_removed_environments_are_distinct() {
    let fixture = Fixture::new().await;
    fixture.physical_state(true, "running", &"a".repeat(64));
    assert_eq!(
        fixture.inspect(fixture.request.clone()).await.result_code,
        Code::Busy as i32
    );
    fixture.physical_state(false, "exited", &"b".repeat(64));
    let mut request = fixture.request.clone();
    request.command_id = crate::new_id();
    assert_eq!(
        fixture.inspect(request).await.result_code,
        Code::StaleScope as i32
    );
    fixture.mark("error");
    let mut request = fixture.request.clone();
    request.command_id = crate::new_id();
    assert_eq!(
        fixture.inspect(request).await.result_code,
        Code::QuiescenceUnknown as i32
    );

    let removed = Fixture::new().await;
    removed.mark("absent");
    assert_eq!(
        removed.inspect(removed.request.clone()).await.result_code,
        Code::Verified as i32
    );
    removed
        .registry
        .attach_environment(&removed.provision, &"a".repeat(64), true)
        .await
        .unwrap();
    removed
        .registry
        .finish(&removed.provision, true)
        .await
        .unwrap();
    let mut request = removed.request.clone();
    request.command_id = crate::new_id();
    assert_eq!(
        removed.inspect(request).await.result_code,
        Code::QuiescenceUnknown as i32
    );
}

#[tokio::test]
async fn exact_scope_and_latest_writer_are_required_but_reader_does_not_replace_owner() {
    let fixture = Fixture::new().await;
    for mutation in 0..5 {
        let mut request = fixture.request.clone();
        request.command_id = crate::new_id();
        match mutation {
            0 => request.lease_fencing_token += 1,
            1 => request.environment_epoch += 1,
            2 => request.project_id = crate::new_id(),
            3 => request.surface_id = crate::new_id(),
            _ => request.run_id = crate::new_id(),
        }
        assert_eq!(
            fixture.inspect(request).await.result_code,
            Code::StaleScope as i32
        );
    }
    let mut reader = fixture.provision.clone();
    reader.run_id = crate::new_id();
    reader.command_id = crate::new_id();
    let mut spec = fixture.spec.clone();
    spec.binding.access = SurfaceAccess::ReadOnly;
    let forge_domain::runtime::SurfaceSpec::GitWorktree {
        repository,
        base_ref,
    } = spec.binding.surface
    else {
        panic!("fixture source must be Git");
    };
    spec.binding.surface = forge_domain::runtime::SurfaceSpec::GitCandidateSnapshot {
        repository,
        base_ref,
        candidate: forge_domain::git::GitCandidate {
            commit: GitObjectId::new(fixture.request.expected_commit.clone()).unwrap(),
            tree: GitObjectId::new(git(&fixture.worktree, &["rev-parse", "HEAD^{tree}"]).await)
                .unwrap(),
        },
    };
    reader.run_spec_json = serde_json::to_string(&spec).unwrap();
    fixture
        .registry
        .register(&reader, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.inspect(fixture.request.clone()).await.result_code,
        Code::Verified as i32
    );
    fixture.registry.finish(&reader, true).await.unwrap();
    let mut writer = fixture.provision.clone();
    writer.run_id = crate::new_id();
    writer.command_id = crate::new_id();
    fixture
        .registry
        .register(&writer, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    fixture.registry.finish(&writer, true).await.unwrap();
    let mut request = fixture.request.clone();
    request.command_id = crate::new_id();
    assert_eq!(
        fixture.inspect(request).await.result_code,
        Code::StaleScope as i32
    );
}

#[tokio::test]
async fn path_manifest_revision_and_host_boot_mismatches_fail_closed() {
    let fixture = Fixture::new().await;
    for change in 0..4 {
        let mut request = fixture.request.clone();
        request.command_id = crate::new_id();
        match change {
            0 => request.surface_id = "../../outside".into(),
            1 => request.expected_commit = "HEAD".into(),
            2 => request.boot_id = "other-boot".into(),
            _ => request.host_id = "other-host".into(),
        }
        let result = fixture.inspect(request).await;
        assert_eq!(result.result_code, Code::InvalidRequest as i32);
        assert!(result.commit.is_empty() && result.tree.is_empty());
    }
    let mut request = fixture.request.clone();
    request.expected_commit = "a".repeat(40);
    assert_eq!(
        fixture.inspect(request).await.result_code,
        Code::HeadChanged as i32
    );
    let manifest = fixture
        .config
        .state_directory
        .join("surface-manifests")
        .join(format!("{}.json", fixture.spec.surface_id));
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["task_id"] = serde_json::json!(crate::new_id());
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let mut request = fixture.request.clone();
    request.command_id = crate::new_id();
    assert_eq!(
        fixture.inspect(request).await.result_code,
        Code::UnsafeSurface as i32
    );
}

#[tokio::test]
async fn command_reuse_is_conflict_not_a_new_inspection_and_full_journal_cannot_publish() {
    let fixture = Fixture::new().await;
    let result = fixture.inspect(fixture.request.clone()).await;
    fixture.acknowledge(&result.message_id).await;
    let mut conflict = fixture.request.clone();
    conflict.expected_commit = "b".repeat(40);
    let rejected = fixture.inspect(conflict.clone()).await;
    assert_eq!(rejected.result_code, Code::ConflictingRequest as i32);
    fixture.acknowledge(&rejected.message_id).await;
    assert_eq!(fixture.inspect(conflict).await, rejected);
    fixture.acknowledge(&rejected.message_id).await;
    assert_eq!(fixture.inspect(fixture.request.clone()).await, result);
    fixture.acknowledge(&result.message_id).await;
    fixture.registry.journal.lock().await.set_test_capacity(1);
    let mut request = fixture.request.clone();
    request.command_id = crate::new_id();
    assert!(matches!(
        fixture
            .backend
            .inspect_git_candidate(request, fixture.registry.clone())
            .await,
        Err(SupervisorError::JournalFull)
    ));
    assert_eq!(fixture.registry.journal.lock().await.pending().count(), 0);
}

#[tokio::test]
async fn inspection_never_waits_behind_creation_or_reopens_the_run_sequence() {
    let fixture = Fixture::new().await;
    let _creating = fixture.backend.provisioning.lock().await;
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        fixture.inspect(fixture.request.clone()),
    )
    .await
    .unwrap();
    assert_eq!(result.result_code, Code::Busy as i32);
    let record = fixture.registry.journal.lock().await.records().remove(0);
    assert!(record.quiescence_confirmed);
}

#[tokio::test]
async fn slow_physical_inspection_keeps_journal_and_stop_controls_available() {
    let fixture = Fixture::new().await;
    fixture.mark("slow");
    let backend = fixture.backend.clone();
    let registry = fixture.registry.clone();
    let request = fixture.request.clone();
    let inspection =
        tokio::spawn(async move { backend.inspect_git_candidate(request, registry).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let other = crate::tests::provision();
    tokio::time::timeout(Duration::from_millis(100), async {
        fixture
            .registry
            .register(&other, &fixture.config.boot_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            fixture
                .registry
                .request_stop(&forge_protocol::supervisor::v1::StopRun {
                    command_id: crate::new_id(),
                    run_id: other.run_id.clone(),
                    lease_fencing_token: other.lease_fencing_token,
                    environment_epoch: other.environment_epoch,
                    mode: forge_protocol::supervisor::v1::StopMode::Force as i32,
                    grace_period_ms: 0,
                    reason_code: "fixture_stop".into(),
                })
                .await
        );
    })
    .await
    .expect("inspection cannot monopolize registry or stop controls");
    inspection.await.unwrap().unwrap();
}
