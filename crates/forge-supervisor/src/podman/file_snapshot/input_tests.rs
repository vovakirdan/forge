use super::*;
use crate::podman::git_inspection::tests::fixture::Fixture;
use forge_domain::{
    ArtifactId, TaskId,
    file_snapshot::{FileSnapshotManifest, SnapshotFile, TaskFileInput, snapshot_object_key},
    runtime::SurfaceSpec,
};
use std::{fs, os::unix::fs::PermissionsExt};

const BINARY: &[u8] = b"\0\xff\x10binary\n";

async fn fixture() -> (Fixture, RuntimeLaunchSpec, PathBuf) {
    let host = Fixture::new().await;
    let mut spec: RuntimeLaunchSpec = host.spec.clone().into();
    spec.schema_version = 6;
    spec.binding.surface = SurfaceSpec::FilesystemSandbox;
    let mut root = host.config.state_directory.clone();
    for component in [
        "file-inputs".into(),
        host.provision.run_id.clone(),
        host.provision.environment_epoch.to_string(),
    ] {
        root.push(component);
        surface::private_directory(&root).unwrap();
    }
    let artifact_id = ArtifactId::new();
    let artifact = root.join(artifact_id.to_string());
    surface::private_directory(&artifact).unwrap();
    let files = artifact.join("files");
    surface::private_directory(&files).unwrap();
    surface::private_directory(&files.join("data")).unwrap();
    surface::private_directory(&files.join("scripts")).unwrap();
    let mut manifest_files = Vec::new();
    for (path, bytes, executable) in [
        ("data/payload.bin", BINARY, false),
        ("empty.txt", b"".as_slice(), false),
        (
            "scripts/hello.sh",
            b"#!/bin/sh\nprintf 'synthetic input\\n'\n".as_slice(),
            true,
        ),
    ] {
        let path_on_disk = files.join(path);
        PrivateMaterialization::create(&path_on_disk, &SecretBytes::new(bytes.to_vec())).unwrap();
        if executable {
            fs::set_permissions(&path_on_disk, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let digest = format!("{:x}", Sha256::digest(bytes));
        manifest_files.push(SnapshotFile {
            path: path.into(),
            size_bytes: bytes.len() as u64,
            executable,
            object_key: snapshot_object_key(spec.project_id, artifact_id, &digest),
            sha256: digest,
        });
    }
    spec.file_inputs = vec![TaskFileInput {
        artifact_id,
        manifest: FileSnapshotManifest {
            schema_version: 1,
            project_id: spec.project_id,
            source_task_id: TaskId::new(),
            files: manifest_files,
        },
    }];
    PrivateMaterialization::create(
        &artifact.join("manifest.json"),
        &SecretBytes::new(serde_json::to_vec(&spec.file_inputs[0]).unwrap()),
    )
    .unwrap();
    PrivateMaterialization::create(
        &root.join("inputs.json"),
        &SecretBytes::new(serde_json::to_vec(&spec.file_inputs).unwrap()),
    )
    .unwrap();
    (host, spec, root)
}

fn input_path(root: &Path, spec: &RuntimeLaunchSpec, relative: &str) -> PathBuf {
    root.join(spec.file_inputs[0].artifact_id.to_string())
        .join("files")
        .join(relative)
}

#[tokio::test]
async fn binary_empty_and_executable_inputs_mount_readonly_without_overlaying_workspace() {
    let (host, spec, root) = fixture().await;
    let workspace_before = fs::read(host.worktree.join("result.txt")).unwrap();
    let mut args = Vec::new();
    host.backend
        .mount_file_inputs(&host.provision, &spec, &mut args)
        .unwrap();
    assert_eq!(
        args,
        vec![
            "--mount".to_owned(),
            format!(
                "type=bind,src={},target=/run/forge-inputs,ro",
                root.display()
            )
        ]
    );
    assert!(!args.iter().any(|arg| arg.contains("target=/workspace")));
    assert_eq!(
        fs::read(input_path(&root, &spec, "data/payload.bin")).unwrap(),
        BINARY
    );
    assert_eq!(
        fs::metadata(input_path(&root, &spec, "empty.txt"))
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        fs::metadata(input_path(&root, &spec, "scripts/hello.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::read(host.worktree.join("result.txt")).unwrap(),
        workspace_before
    );
    assert!(!host.worktree.join("data/payload.bin").exists());
}

#[tokio::test]
async fn mutated_input_bytes_are_rejected_before_mount_arguments_are_added() {
    let (host, spec, root) = fixture().await;
    let mut changed = BINARY.to_vec();
    changed[0] ^= 1;
    fs::write(input_path(&root, &spec, "data/payload.bin"), changed).unwrap();
    let mut args = Vec::new();
    assert!(matches!(
        host.backend
            .mount_file_inputs(&host.provision, &spec, &mut args),
        Err(SupervisorError::UnsafeSurface)
    ));
    assert!(args.is_empty());
}

#[tokio::test]
async fn stale_manifest_is_rejected_against_the_frozen_run_inputs() {
    let (host, mut spec, _root) = fixture().await;
    spec.file_inputs[0].manifest.source_task_id = TaskId::new();
    let mut args = Vec::new();
    assert!(matches!(
        host.backend
            .mount_file_inputs(&host.provision, &spec, &mut args),
        Err(SupervisorError::InvalidRunSpec)
    ));
    assert!(args.is_empty());
}

#[tokio::test]
async fn changed_per_artifact_manifest_is_not_exposed_to_the_employee() {
    let (host, spec, root) = fixture().await;
    fs::write(
        root.join(spec.file_inputs[0].artifact_id.to_string())
            .join("manifest.json"),
        b"{}",
    )
    .unwrap();
    let mut args = Vec::new();
    assert!(matches!(
        host.backend
            .mount_file_inputs(&host.provision, &spec, &mut args),
        Err(SupervisorError::InvalidRunSpec)
    ));
    assert!(args.is_empty());
}

#[tokio::test]
async fn changed_executable_bit_is_rejected() {
    let (host, spec, root) = fixture().await;
    fs::set_permissions(
        input_path(&root, &spec, "scripts/hello.sh"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let mut args = Vec::new();
    assert!(matches!(
        host.backend
            .mount_file_inputs(&host.provision, &spec, &mut args),
        Err(SupervisorError::UnsafeSurface)
    ));
    assert!(args.is_empty());
}
