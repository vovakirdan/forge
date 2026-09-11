use super::*;
use crate::podman::git_inspection::tests::fixture::{Fixture, git};
use forge_domain::runtime::SurfaceSpec;
use std::fs;
use uuid::Uuid;

fn request(fixture: &Fixture, paths: &[&str]) -> FileCaptureRequest {
    FileCaptureRequest {
        operation_id: Uuid::now_v7(),
        project_id: fixture.spec.project_id,
        task_id: serde_json::from_value(serde_json::json!(fixture.provision.task_id)).unwrap(),
        run_id: Uuid::parse_str(&fixture.provision.run_id).unwrap(),
        fencing_token: fixture.provision.lease_fencing_token,
        environment_epoch: fixture.provision.environment_epoch,
        surface_id: fixture.spec.surface_id,
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
    }
}

async fn capture(fixture: &Fixture, request: &FileCaptureRequest) -> Result<(), SupervisorError> {
    tokio::time::timeout(
        Duration::from_secs(5),
        fixture.backend.capture_file_snapshot(
            CaptureFileSnapshot {
                request_json: serde_json::to_string(request).unwrap(),
            },
            fixture.registry.clone(),
        ),
    )
    .await
    .expect("bounded synthetic inspector/capture")
}

fn assert_failed_without_reading(fixture: &Fixture, request: &FileCaptureRequest) {
    let captures = fixture.config.state_directory.join("file-captures");
    assert!(!captures.join(request.operation_id.to_string()).exists());
    let marker =
        PrivateMaterialization::open(&captures.join(format!("{}.failed", request.operation_id)))
            .unwrap()
            .read(128 * 1024)
            .unwrap();
    assert_eq!(marker.expose(), serde_json::to_vec(request).unwrap());
}

#[tokio::test]
async fn quiescent_capture_reads_tracked_and_untracked_files_and_replays_after_restart() {
    let mut fixture = Fixture::new().await;
    fs::write(fixture.worktree.join("untracked.bin"), [0, 255, 17]).unwrap();
    fs::write(fixture.worktree.join("empty.txt"), []).unwrap();
    let request = request(&fixture, &["result.txt", "untracked.bin", "empty.txt"]);
    let status_before = git(
        &fixture.worktree,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .await;
    assert!(status_before.contains("?? untracked.bin"));
    assert!(
        !status_before.contains("result.txt"),
        "the selected report is tracked and clean"
    );
    capture(&fixture, &request).await.unwrap();
    let target = fixture
        .config
        .state_directory
        .join("file-captures")
        .join(request.operation_id.to_string());
    let identity = serde_json::to_vec(&request).unwrap();
    let captured = read_capture(&target, &identity, MAX_SNAPSHOT_BYTES)
        .unwrap()
        .unwrap();
    assert_eq!(
        captured
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["result.txt", "untracked.bin", "empty.txt"]
    );
    for (file, expected) in captured.iter().zip([
        b"Employee implementation\n".as_slice(),
        &[0, 255, 17],
        b"".as_slice(),
    ]) {
        let bytes = PrivateMaterialization::read_beneath(
            &target,
            &Path::new("blobs").join(&file.sha256),
            file.size_bytes,
        )
        .unwrap();
        assert_eq!(bytes.expose(), expected);
    }
    assert_eq!(
        git(
            &fixture.worktree,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        )
        .await,
        status_before
    );
    assert!(
        !fixture
            .config
            .state_directory
            .join("file-captures")
            .join(format!("{}.failed", request.operation_id))
            .exists()
    );

    fs::write(
        fixture.worktree.join("result.txt"),
        b"Later tracked contents",
    )
    .unwrap();
    fs::write(fixture.worktree.join("untracked.bin"), [9, 8, 7]).unwrap();
    fixture.restart().await;
    fixture.mark("error"); // An immutable completed receipt needs no new physical inspection.
    capture(&fixture, &request).await.unwrap();
    assert_eq!(
        read_capture(&target, &identity, MAX_SNAPSHOT_BYTES)
            .unwrap()
            .unwrap(),
        captured
    );
    assert_eq!(
        fs::read(fixture.worktree.join("result.txt")).unwrap(),
        b"Later tracked contents"
    );
    assert_eq!(
        fs::read(fixture.worktree.join("untracked.bin")).unwrap(),
        [9, 8, 7]
    );
}

#[tokio::test]
async fn logical_stop_is_insufficient_when_physical_state_or_scope_is_wrong() {
    let fixture = Fixture::new().await;
    for case in 0..3 {
        let mut request = request(&fixture, &["result.txt"]);
        match case {
            0 => fixture.physical_state(true, "running", &"a".repeat(64)),
            1 => fixture.physical_state(false, "exited", &"b".repeat(64)),
            _ => {
                fixture.physical_state(false, "exited", &"a".repeat(64));
                request.surface_id = Uuid::now_v7();
            }
        }
        assert!(
            matches!(
                capture(&fixture, &request).await,
                Err(SupervisorError::UnsafeSurface)
            ),
            "case {case}"
        );
        assert_failed_without_reading(&fixture, &request);
    }
}

#[tokio::test]
async fn absent_container_does_not_authorize_another_tasks_retained_surface() {
    let fixture = Fixture::new().await;
    let mut foreign_provision = fixture.provision.clone();
    foreign_provision.task_id = crate::new_id();
    foreign_provision.run_id = crate::new_id();
    let mut foreign_spec: RuntimeLaunchSpec = fixture.spec.clone().into();
    foreign_spec.surface_id = Uuid::now_v7();
    foreign_spec.binding.surface = SurfaceSpec::FilesystemSandbox;
    let prepared = surface::prepare(
        &fixture.config.state_directory,
        &foreign_provision,
        &foreign_spec,
    )
    .await
    .unwrap();
    let foreign_worktree = prepared.mount.unwrap();
    fs::write(foreign_worktree.join("result.txt"), b"Foreign Task content").unwrap();
    fixture.mark("absent");
    let mut request = request(&fixture, &["result.txt"]);
    request.surface_id = foreign_spec.surface_id;
    assert!(matches!(
        capture(&fixture, &request).await,
        Err(SupervisorError::UnsafeSurface)
    ));
    assert_failed_without_reading(&fixture, &request);
    assert_eq!(
        fs::read(foreign_worktree.join("result.txt")).unwrap(),
        b"Foreign Task content"
    );
}
