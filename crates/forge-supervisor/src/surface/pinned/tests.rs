use super::*;
use forge_domain::{StageWorkspaceKind, StageWorkspaceRequirements, git::GitObjectId};
use std::fs;
pub(crate) mod fixture;
use fixture::{Fixture, git};

#[tokio::test]
async fn pinned_copy_uses_accepted_commit_not_current_files_or_later_head() {
    let fixture = Fixture::new().await;
    let manifest = fs::read(&fixture.manifest).unwrap();
    fs::write(
        fixture.writer.join("tracked.txt"),
        "later committed revision\n",
    )
    .unwrap();
    git(&fixture.writer, &["add", "tracked.txt"]).await;
    git(&fixture.writer, &["commit", "-m", "Later work"]).await;
    let later = git(&fixture.writer, &["rev-parse", "HEAD"]).await;
    fs::write(fixture.writer.join("tracked.txt"), "retained dirty work\n").unwrap();
    fs::write(
        fixture.writer.join("untracked.txt"),
        "retained untracked work\n",
    )
    .unwrap();
    fs::write(fixture.writer.join("ignored.txt"), "not accepted\n").unwrap();
    let surface = fixture.prepare().await.unwrap();
    assert!(surface.read_only);
    assert_eq!(surface.workdir, "/workspace/worktree");
    let mounted = surface.mount.unwrap();
    let worktree = mounted.join("worktree");
    assert!(mounted.starts_with(fixture.root.join("snapshots")));
    assert_eq!(
        git(&worktree, &["rev-parse", "HEAD"]).await,
        fixture.candidate.commit.as_str()
    );
    assert_eq!(
        fs::read_to_string(worktree.join("tracked.txt")).unwrap(),
        "accepted revision\n"
    );
    assert!(!worktree.join("untracked.txt").exists());
    assert!(!worktree.join("ignored.txt").exists());
    assert!(git(&worktree, &["status", "--porcelain"]).await.is_empty());
    assert_eq!(fs::read(&fixture.manifest).unwrap(), manifest);
    assert_eq!(git(&fixture.writer, &["rev-parse", "HEAD"]).await, later);
    assert_eq!(
        fs::read_to_string(fixture.writer.join("tracked.txt")).unwrap(),
        "retained dirty work\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.source.join("tracked.txt")).unwrap(),
        "original\n"
    );
    // Relocation models mounting the complete private root at /workspace.
    let relocated = fixture.root.join("relocated-snapshot");
    fs::rename(mounted, &relocated).unwrap();
    assert_eq!(
        git(&relocated.join("worktree"), &["rev-parse", "HEAD"]).await,
        fixture.candidate.commit.as_str()
    );
}

#[tokio::test]
async fn snapshots_cannot_claim_writer_access_or_different_task_source() {
    let mut fixture = Fixture::new().await;
    fixture.spec.binding.access = SurfaceAccess::ReadWrite;
    assert!(fixture.spec.validate().is_err());
    assert!(fixture.prepare().await.is_err());
    fixture.spec.binding.access = SurfaceAccess::ReadOnly;
    let requirement = StageWorkspaceRequirements {
        kind: StageWorkspaceKind::Git,
        access: SurfaceAccess::ReadOnly,
    };
    assert!(requirement.is_compatible(&fixture.spec.binding.surface, fixture.spec.binding.access));
    let task_id = fixture.provision.task_id.clone();
    fixture.provision.task_id = crate::new_id();
    assert!(fixture.prepare().await.is_err());
    fixture.provision.task_id = task_id;
    let original = fixture.spec.binding.surface.clone();
    if let SurfaceSpec::GitCandidateSnapshot { repository, .. } = &mut fixture.spec.binding.surface
    {
        *repository = "/tmp/different-project-source".into();
    }
    assert!(fixture.prepare().await.is_err());
    fixture.spec.binding.surface = original;
    if let SurfaceSpec::GitCandidateSnapshot { base_ref, .. } = &mut fixture.spec.binding.surface {
        *base_ref = "a".repeat(40);
    }
    assert!(fixture.prepare().await.is_err());
    assert!(!fixture.root.join("snapshots").exists());
}

#[tokio::test]
async fn mismatched_tree_and_existing_snapshot_fail_without_repair_or_overwrite() {
    let mut fixture = Fixture::new().await;
    let manifest = fs::read(&fixture.manifest).unwrap();
    if let SurfaceSpec::GitCandidateSnapshot { candidate, .. } = &mut fixture.spec.binding.surface {
        candidate.tree = GitObjectId::new("a".repeat(40)).unwrap();
    }
    assert!(fixture.prepare().await.is_err());
    assert_eq!(fs::read(&fixture.manifest).unwrap(), manifest);
    // A failed attempt retains its private directory and cannot be silently reused.
    if let SurfaceSpec::GitCandidateSnapshot { candidate, .. } = &mut fixture.spec.binding.surface {
        *candidate = fixture.candidate.clone();
    }
    assert!(fixture.prepare().await.is_err());
    fixture.provision.run_id = crate::new_id();
    let mount = fixture.prepare().await.unwrap().mount.unwrap();
    fs::write(mount.join("retained-marker"), "not overwritten").unwrap();
    assert!(fixture.prepare().await.is_err());
    assert_eq!(
        fs::read_to_string(mount.join("retained-marker")).unwrap(),
        "not overwritten"
    );
}

#[tokio::test]
async fn missing_or_symlinked_manifest_never_reinitializes_from_repository() {
    let fixture = Fixture::new().await;
    let retained = fixture.root.join("retained-manifest");
    fs::rename(&fixture.manifest, &retained).unwrap();
    assert!(fixture.prepare().await.is_err());
    std::os::unix::fs::symlink(&retained, &fixture.manifest).unwrap();
    assert!(fixture.prepare().await.is_err());
    assert!(!fixture.root.join("snapshots").exists());
    assert!(retained.exists());
}
