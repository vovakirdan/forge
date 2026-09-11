//! Physical input-boundary acceptance without a provider, login, or network access.

use super::*;
use forge_domain::{
    ArtifactId, TaskId,
    file_snapshot::{
        FileCaptureRequest, FileSnapshotManifest, MAX_SNAPSHOT_BYTES, SnapshotFile, TaskFileInput,
        snapshot_object_key,
    },
    git::{
        GitBranchRef, GitObjectId, GitSourceDescriptor, GitSourceRequest, LocalGitPath,
        TaskGitSourcePolicy,
    },
    runtime::TaskRunSpecV6,
};
use forge_protocol::supervisor::v1::{
    AcknowledgementDisposition, CaptureFileSnapshot, CoreAcknowledgement,
    TaskStageExecutionAssignment, provision_run::Assignment,
};
use forge_provider_common::selected_file::{CapturedFile, read_capture};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use uuid::Uuid;

type Worker = tokio::task::JoinHandle<Result<(), crate::SupervisorError>>;

#[derive(Default)]
struct ContainerCleanup(Vec<String>);

impl Drop for ContainerCleanup {
    fn drop(&mut self) {
        // A failed assertion must not orphan a HOLD fixture. Retain its files
        // for diagnosis, but remove only container identities created by this test.
        for name in &self.0 {
            let _ = std::process::Command::new("podman")
                .args(["rm", "--force", name])
                .output();
        }
    }
}

fn worktree(fixture: &Fixture) -> PathBuf {
    fixture
        .config
        .state_directory
        .join("surfaces")
        .join(fixture.spec.surface_id.to_string())
        .join("worktree")
}

fn container(fixture: &Fixture) -> String {
    format!(
        "forge-run-{}-{}-{}",
        fixture.provision.run_id,
        fixture.provision.lease_fencing_token,
        fixture.provision.environment_epoch
    )
}

fn install_spec(
    fixture: &mut Fixture,
    source: Option<GitSourceRequest>,
    inputs: Vec<TaskFileInput>,
) {
    let spec = TaskRunSpecV6 {
        schema_version: 6,
        project_id: fixture.spec.project_id,
        surface_id: fixture.spec.surface_id,
        binding: fixture.spec.binding.clone(),
        instruction: fixture.spec.instruction.clone(),
        source_request: source,
        file_inputs: inputs,
    };
    spec.validate().unwrap();
    fixture.provision.run_spec_version = 6;
    fixture.provision.run_spec_json = serde_json::to_string(&spec).unwrap();
    fixture.provision.assignment = Some(Assignment::TaskStage(TaskStageExecutionAssignment {
        task_id: fixture.provision.task_id.clone(),
        stage_id: fixture.provision.stage_id.clone(),
        queue_entry_id: new_id(),
    }));
}

async fn start(fixture: &Fixture, cleanup: &mut ContainerCleanup) -> Worker {
    cleanup.0.push(container(fixture));
    let backend = PodmanBackend::new(&fixture.config);
    let control = fixture
        .registry
        .register(&fixture.provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    let provision = fixture.provision.clone();
    let registry = fixture.registry.clone();
    let worker =
        tokio::spawn(async move { backend.run(provision, registry, control, false).await });
    timeout(Duration::from_secs(30), async {
        loop {
            if worker.is_finished() {
                break;
            }
            let inspected = Command::new("podman")
                .args([
                    "inspect",
                    "--format",
                    "{{.State.Running}}",
                    &container(fixture),
                ])
                .output()
                .await
                .unwrap();
            if inspected.status.success() && inspected.stdout == b"true\n" {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("fixture container starts within 30 seconds");
    if worker.is_finished() {
        panic!(
            "fixture exited before physical acceptance: {:?}",
            worker.await
        );
    }
    worker
}

async fn exec(fixture: &Fixture, args: &[&str]) -> Vec<u8> {
    let output = timeout(
        Duration::from_secs(10),
        Command::new("podman")
            .args(["exec", &container(fixture)])
            .args(args)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "container assertion {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

async fn stop(fixture: &Fixture, worker: Worker) {
    request_fixture_stop(fixture).await;
    timeout(Duration::from_secs(15), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    await_container_exit(fixture).await;
    let mut journal = fixture.registry.journal.lock().await;
    let ids: Vec<_> = journal
        .pending()
        .map(|message| crate::journal::message_id(message).unwrap().to_owned())
        .collect();
    for id in ids {
        journal
            .acknowledge(&CoreAcknowledgement {
                message_id: new_id(),
                acknowledged_message_id: id,
                disposition: AcknowledgementDisposition::Accepted as i32,
                reason_code: String::new(),
                message: String::new(),
            })
            .unwrap();
    }
}

async fn next_run(fixture: &mut Fixture, new_task: bool) {
    let removed = Command::new("podman")
        .args(["rm", &container(fixture)])
        .output()
        .await
        .unwrap();
    assert!(
        removed.status.success(),
        "remove only the already stopped test container"
    );
    let previous_grant = scoped_directory(&fixture.config.grants_directory, &fixture.provision);
    fixture.provision.command_id = new_id();
    fixture.provision.run_id = new_id();
    if new_task {
        fixture.provision.task_id = new_id();
        fixture.spec.surface_id = Uuid::now_v7();
    }
    let grant = scoped_directory(&fixture.config.grants_directory, &fixture.provision);
    private_directory(grant.parent().unwrap()).unwrap();
    private_directory(&grant).unwrap();
    for file in ["invocation.json", "stdin"] {
        private_write(
            &grant.join(file),
            &fs::read(previous_grant.join(file)).unwrap(),
        )
        .unwrap();
    }
    let gateway = fixture
        .config
        .gateways_directory
        .join(&fixture.provision.run_id);
    private_directory(&gateway).unwrap();
    fixture._gateway = Some(UnixListener::bind(gateway.join("gateway.sock")).unwrap());
}

async fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .current_dir(path)
        .env_clear()
        .envs([
            ("PATH", "/usr/bin:/bin"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ])
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "synthetic Git operation {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

async fn descriptor(fixture: &Fixture) -> GitSourceDescriptor {
    serde_json::from_slice(&exec(fixture, &["cat", "/run/forge-source/descriptor.json"]).await)
        .unwrap()
}

async fn assert_read_only(fixture: &Fixture, path: &str) {
    exec(fixture, &["/bin/sh", "-c",
        "if (printf forbidden > \"$1\") 2>/tmp/input-write-error; then exit 1; fi; grep -q 'Read-only file system' /tmp/input-write-error",
        "assert-read-only", path]).await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and the TASK-12 fixture image; no credentials"]
async fn pinned_source_is_frozen_in_container_and_future_latest_preserves_retained_work() {
    let mut fixture = Fixture::new(true).await;
    let mut cleanup = ContainerCleanup::default();
    let source = fixture.config.state_directory.join("source");
    private_directory(&source).unwrap();
    git(&source, &["init", "--initial-branch=main", "--template="]).await;
    private_write(&source.join("README.md"), b"commit A\n").unwrap();
    git(&source, &["add", "README.md"]).await;
    git(&source, &["commit", "-m", "A"]).await;
    let a = git(&source, &["rev-parse", "HEAD"]).await;
    fs::write(source.join("README.md"), b"commit B\n").unwrap();
    git(&source, &["commit", "-am", "B"]).await;
    let b = git(&source, &["rev-parse", "HEAD"]).await;
    assert_ne!(a, b);
    fixture.spec.binding.surface = SurfaceSpec::GitWorktree {
        repository: source.to_str().unwrap().into(),
        base_ref: a.clone(),
    };
    let mut request = GitSourceRequest {
        repository_id: Uuid::now_v7(),
        source: LocalGitPath::new(source.to_str().unwrap()).unwrap(),
        target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
        initial_revision: GitObjectId::new(&a).unwrap().into(),
        policy_revision: 1,
        policy: TaskGitSourcePolicy::PinnedCommit {
            commit: GitObjectId::new(&a).unwrap(),
        },
    };
    install_spec(&mut fixture, Some(request.clone()), Vec::new());
    let worker = start(&fixture, &mut cleanup).await;
    let selected = descriptor(&fixture).await;
    assert_eq!(
        selected.selected_revision,
        GitObjectId::new(&a).unwrap().into()
    );
    assert_eq!(selected.policy, request.policy);
    assert_eq!(selected.policy_revision, 1);
    let bundle = exec(&fixture, &["cat", "/run/forge-source/source.bundle"]).await;
    assert_eq!(
        selected.bundle_sha256,
        Some(format!("{:x}", Sha256::digest(&bundle)))
    );
    let exported_bundle = fixture
        .config
        .state_directory
        .join("git-sources")
        .join(&fixture.provision.run_id)
        .join(format!(
            "{}-{}",
            fixture.provision.lease_fencing_token, fixture.provision.environment_epoch
        ))
        .join("source.bundle");
    assert_eq!(fs::read(&exported_bundle).unwrap(), bundle);
    assert_eq!(
        git(
            &source,
            &["bundle", "list-heads", exported_bundle.to_str().unwrap()]
        )
        .await,
        format!("{a} refs/forge/source")
    );
    assert_eq!(
        exec(&fixture, &["cat", "/workspace/worktree/README.md"]).await,
        b"commit A\n"
    );
    assert_eq!(git(&worktree(&fixture), &["rev-parse", "HEAD"]).await, a);
    assert_read_only(&fixture, "/run/forge-source/source.bundle").await;

    git(&source, &["update-ref", "refs/heads/main", &a, &b]).await;
    assert_eq!(descriptor(&fixture).await, selected);
    assert_eq!(
        exec(&fixture, &["cat", "/run/forge-source/source.bundle"]).await,
        bundle
    );
    git(&source, &["update-ref", "refs/heads/main", &b, &a]).await;
    stop(&fixture, worker).await;

    let retained = worktree(&fixture);
    fs::write(retained.join("employee.txt"), b"employee commit\n").unwrap();
    git(&retained, &["add", "employee.txt"]).await;
    git(&retained, &["commit", "-m", "Retained employee work"]).await;
    fs::write(retained.join("README.md"), b"staged work\n").unwrap();
    git(&retained, &["add", "README.md"]).await;
    fs::write(retained.join("README.md"), b"dirty work\n").unwrap();
    fs::write(retained.join("untracked.txt"), b"retained untracked\n").unwrap();
    let head = git(&retained, &["rev-parse", "HEAD"]).await;
    let status = git(
        &retained,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .await;
    let index = git(&retained, &["write-tree"]).await;

    next_run(&mut fixture, false).await;
    request.policy = TaskGitSourcePolicy::LatestTarget;
    request.policy_revision = 2;
    install_spec(&mut fixture, Some(request.clone()), Vec::new());
    let worker = start(&fixture, &mut cleanup).await;
    let latest = descriptor(&fixture).await;
    assert_eq!(
        latest.selected_revision,
        GitObjectId::new(&b).unwrap().into()
    );
    assert_eq!(latest.policy, TaskGitSourcePolicy::LatestTarget);
    assert_eq!(latest.policy_revision, 2);
    assert_eq!(git(&retained, &["rev-parse", "HEAD"]).await, head);
    assert_eq!(git(&retained, &["write-tree"]).await, index);
    assert_eq!(
        git(
            &retained,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        )
        .await,
        status
    );
    assert_eq!(
        exec(&fixture, &["cat", "/workspace/worktree/README.md"]).await,
        b"dirty work\n"
    );
    assert_eq!(
        exec(&fixture, &["cat", "/workspace/worktree/untracked.txt"]).await,
        b"retained untracked\n"
    );
    stop(&fixture, worker).await;

    next_run(&mut fixture, true).await;
    install_spec(&mut fixture, Some(request), Vec::new());
    let worker = start(&fixture, &mut cleanup).await;
    assert_eq!(
        exec(&fixture, &["cat", "/workspace/worktree/README.md"]).await,
        b"commit B\n"
    );
    assert_eq!(git(&worktree(&fixture), &["rev-parse", "HEAD"]).await, b);
    stop(&fixture, worker).await;
    fixture.cleanup().await;
}

fn capture_request(fixture: &Fixture) -> FileCaptureRequest {
    FileCaptureRequest {
        operation_id: Uuid::now_v7(),
        project_id: fixture.spec.project_id,
        task_id: fixture.provision.task_id.parse().unwrap(),
        run_id: fixture.provision.run_id.parse().unwrap(),
        fencing_token: fixture.provision.lease_fencing_token,
        environment_epoch: fixture.provision.environment_epoch,
        surface_id: fixture.spec.surface_id,
        paths: ["text.txt", "empty.txt", "binary.bin", "hello.sh"]
            .map(str::to_owned)
            .to_vec(),
    }
}

async fn capture(
    fixture: &Fixture,
    request: &FileCaptureRequest,
) -> Result<(), crate::SupervisorError> {
    timeout(
        Duration::from_secs(15),
        PodmanBackend::new(&fixture.config).capture_file_snapshot(
            CaptureFileSnapshot {
                request_json: serde_json::to_string(request).unwrap(),
            },
            fixture.registry.clone(),
        ),
    )
    .await
    .unwrap()
}

fn materialize_capture(
    fixture: &Fixture,
    task: TaskId,
    capture: &Path,
    files: &[CapturedFile],
) -> TaskFileInput {
    // Core's object-store/materialization boundary has separate integration tests;
    // this fixture supplies those captured bytes to the real Supervisor mount path.
    let artifact_id = ArtifactId::new();
    let root = fixture.config.state_directory.join("file-inputs");
    private_directory(&root).unwrap();
    let root = root.join(&fixture.provision.run_id);
    private_directory(&root).unwrap();
    let root = root.join(fixture.provision.environment_epoch.to_string());
    private_directory(&root).unwrap();
    let artifact = root.join(artifact_id.to_string());
    private_directory(&artifact).unwrap();
    private_directory(&artifact.join("files")).unwrap();
    let mut manifest = Vec::new();
    for file in files {
        let bytes = fs::read(capture.join("blobs").join(&file.sha256)).unwrap();
        let path = artifact.join("files").join(&file.path);
        private_write(&path, &bytes).unwrap();
        if file.executable {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        manifest.push(SnapshotFile {
            path: file.path.clone(),
            size_bytes: file.size_bytes,
            executable: file.executable,
            sha256: file.sha256.clone(),
            object_key: snapshot_object_key(fixture.spec.project_id, artifact_id, &file.sha256),
        });
    }
    let input = TaskFileInput {
        artifact_id,
        manifest: FileSnapshotManifest {
            schema_version: 1,
            project_id: fixture.spec.project_id,
            source_task_id: task,
            files: manifest,
        },
    };
    private_write(
        &artifact.join("manifest.json"),
        &serde_json::to_vec(&input).unwrap(),
    )
    .unwrap();
    private_write(
        &root.join("inputs.json"),
        &serde_json::to_vec(&vec![input.clone()]).unwrap(),
    )
    .unwrap();
    input
}

#[tokio::test]
#[ignore = "requires rootless Podman and the TASK-12 fixture image; no credentials"]
async fn stopped_surface_snapshot_keeps_bytes_and_mounts_readonly_without_workspace_overlay() {
    let mut fixture = Fixture::new(true).await;
    let mut cleanup = ContainerCleanup::default();
    install_spec(&mut fixture, None, Vec::new());
    let worker = start(&fixture, &mut cleanup).await;
    exec(&fixture, &["/bin/sh", "-c", "printf 'original text\\n' > text.txt; : > empty.txt; printf '\\000\\377\\020binary\\n' > binary.bin; printf '#!/bin/sh\\nprintf executable\\n' > hello.sh; chmod 700 hello.sh"]).await;
    let request = capture_request(&fixture);
    assert!(matches!(
        capture(&fixture, &request).await,
        Err(crate::SupervisorError::UnsafeSurface)
    ));
    assert_eq!(
        exec(&fixture, &["cat", "/workspace/worktree/text.txt"]).await,
        b"original text\n"
    );
    stop(&fixture, worker).await;
    let request = capture_request(&fixture);
    capture(&fixture, &request).await.unwrap();
    let captured = fixture
        .config
        .state_directory
        .join("file-captures")
        .join(request.operation_id.to_string());
    let identity = serde_json::to_vec(&request).unwrap();
    let files = read_capture(&captured, &identity, MAX_SNAPSHOT_BYTES)
        .unwrap()
        .unwrap();
    let source = worktree(&fixture);
    assert!(!source.join(".git").exists());
    for path in &request.paths {
        fs::write(source.join(path), b"changed after capture\n").unwrap();
    }
    capture(&fixture, &request).await.unwrap();
    assert_eq!(
        read_capture(&captured, &identity, MAX_SNAPSHOT_BYTES)
            .unwrap()
            .unwrap(),
        files
    );
    next_run(&mut fixture, true).await;
    let input = materialize_capture(&fixture, request.task_id, &captured, &files);
    let path = format!("/run/forge-inputs/{}/files", input.artifact_id);
    install_spec(&mut fixture, None, vec![input]);
    let worker = start(&fixture, &mut cleanup).await;
    for (relative, expected) in [
        ("text.txt", b"original text\n".as_slice()),
        ("empty.txt", b"".as_slice()),
        ("binary.bin", b"\0\xff\x10binary\n".as_slice()),
        ("hello.sh", b"#!/bin/sh\nprintf executable\n".as_slice()),
    ] {
        assert_eq!(
            exec(&fixture, &["cat", &format!("{path}/{relative}")]).await,
            expected
        );
        assert_eq!(
            fs::read(source.join(relative)).unwrap(),
            b"changed after capture\n"
        );
        exec(
            &fixture,
            &[
                "test",
                "!",
                "-e",
                &format!("/workspace/worktree/{relative}"),
            ],
        )
        .await;
    }
    assert_eq!(
        exec(&fixture, &[&format!("{path}/hello.sh")]).await,
        b"executable"
    );
    exec(
        &fixture,
        &[
            "/bin/sh",
            "-c",
            "printf 'workspace remains writable' > /workspace/worktree/text.txt",
        ],
    )
    .await;
    assert_eq!(
        exec(&fixture, &["cat", "/workspace/worktree/text.txt"]).await,
        b"workspace remains writable"
    );
    assert_read_only(&fixture, &format!("{path}/text.txt")).await;
    assert_read_only(&fixture, &format!("{path}/new-file")).await;
    assert_eq!(
        exec(&fixture, &["cat", &format!("{path}/text.txt")]).await,
        b"original text\n"
    );
    stop(&fixture, worker).await;
    fixture.cleanup().await;
}
