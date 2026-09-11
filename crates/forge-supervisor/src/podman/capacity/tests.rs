use super::*;
use forge_protocol::{
    runtime::RuntimeInputConfig,
    supervisor::v1::{DeliverRuntimeInput, RuntimeMessageInput, deliver_runtime_input::Action},
};
use forge_provider_common::native_input::NativeMailbox;
use std::os::unix::{fs::DirBuilderExt, net::UnixListener};
use uuid::Uuid;

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("forge-evidence-admission-{}", crate::new_id()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        let directory = Self(path);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory.epoch())
            .unwrap();
        directory
    }

    fn evidence(&self) -> PathBuf {
        self.0.join("evidence")
    }

    fn epoch(&self) -> PathBuf {
        self.evidence().join("run/1")
    }

    fn mailbox(&self) -> NativeMailbox {
        let private = self.0.join("private");
        fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
        NativeMailbox::open(
            RuntimeInputConfig {
                run_id: Uuid::now_v7().to_string(),
                fencing_token: 1,
                environment_epoch: 1,
            },
            &private,
            self.0.join("inputs"),
            self.epoch().join("input-receipts"),
        )
        .unwrap()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn retained_files_exhaust_capacity_without_being_deleted() {
    let directory = Directory::new();
    let stdout = directory.epoch().join("stdout.jsonl");
    fs::write(&stdout, b"retained evidence").unwrap();
    assert!(matches!(
        retained_bytes(&directory.evidence(), 1),
        Err(SupervisorError::EvidenceCapacity)
    ));
    assert_eq!(fs::read(stdout).unwrap(), b"retained evidence");
}

#[test]
fn native_mailbox_empty_receipts_directory_does_not_block_admission() {
    let directory = Directory::new();
    let _mailbox = directory.mailbox();
    assert_eq!(retained_bytes(&directory.evidence(), 0).unwrap(), 0);
}

#[test]
fn native_receipt_and_pending_bytes_share_the_evidence_limit() {
    let directory = Directory::new();
    let mailbox = directory.mailbox();
    let input = DeliverRuntimeInput {
        command_id: Uuid::now_v7().to_string(),
        run_id: mailbox.scope().run_id.clone(),
        lease_fencing_token: 1,
        environment_epoch: 1,
        sequence: 1,
        action: Some(Action::Message(RuntimeMessageInput {
            source_message_json: "{}".into(),
        })),
    };
    mailbox.accepted(&input).unwrap();
    let receipts = directory.epoch().join("input-receipts");
    let receipt = receipts.join(format!("{}.json", input.command_id));
    fs::write(receipts.join(".pending-fixture"), b"atomic temp").unwrap();
    fs::write(directory.epoch().join("stdout.jsonl"), b"output").unwrap();
    let expected = fs::metadata(&receipt).unwrap().len() + 11 + 6;
    assert_eq!(
        retained_bytes(&directory.evidence(), expected).unwrap(),
        expected
    );
    assert!(matches!(
        retained_bytes(&directory.evidence(), expected - 1),
        Err(SupervisorError::EvidenceCapacity)
    ));
    assert!(receipt.is_file(), "capacity checks must retain receipts");
}

#[test]
fn arbitrary_epoch_directories_and_symlinks_remain_forbidden() {
    for name in ["other-directory", "symlink"] {
        let directory = Directory::new();
        let entry = directory.epoch().join(name);
        if name == "symlink" {
            std::os::unix::fs::symlink("/unmounted-host-data", entry).unwrap();
        } else {
            fs::create_dir(entry).unwrap();
        }
        assert!(matches!(
            retained_bytes(&directory.evidence(), 1024),
            Err(SupervisorError::UnsafeSurface)
        ));
    }
}

#[test]
fn receipt_subtree_rejects_symlink_nested_directory_and_socket() {
    for kind in ["symlink", "directory", "socket"] {
        let directory = Directory::new();
        let _mailbox = directory.mailbox();
        let entry = directory.epoch().join("input-receipts").join("unsafe");
        let _socket = match kind {
            "symlink" => {
                std::os::unix::fs::symlink("/unmounted-host-data", entry).unwrap();
                None
            }
            "directory" => {
                fs::create_dir(entry).unwrap();
                None
            }
            _ => Some(UnixListener::bind(entry).unwrap()),
        };
        assert!(matches!(
            retained_bytes(&directory.evidence(), 1024),
            Err(SupervisorError::UnsafeSurface)
        ));
    }
}

#[test]
fn receipt_directory_symlink_is_not_followed() {
    let directory = Directory::new();
    std::os::unix::fs::symlink(&directory.0, directory.epoch().join("input-receipts")).unwrap();
    assert!(matches!(
        retained_bytes(&directory.evidence(), 1024),
        Err(SupervisorError::UnsafeSurface)
    ));
}

#[test]
fn receipt_cleanup_after_enumeration_is_a_safe_missing_leaf() {
    let directory = Directory::new();
    let _mailbox = directory.mailbox();
    let receipts = directory.epoch().join("input-receipts");
    fs::write(receipts.join("acknowledged.json"), b"receipt").unwrap();
    let pinned = open_directory(&receipts).unwrap();
    let entry = fs::read_dir(directory_path(&pinned))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    fs::remove_file(entry.path()).unwrap();
    let mut usage = EvidenceUsage {
        bytes: 0,
        entries: 0,
        maximum: 0,
    };
    account_file(&pinned, &entry.file_name(), &mut usage).unwrap();
    assert_eq!(usage.bytes, 0);
}

#[test]
fn receipt_replacement_after_enumeration_does_not_follow_a_symlink() {
    let directory = Directory::new();
    let _mailbox = directory.mailbox();
    let receipts = directory.epoch().join("input-receipts");
    fs::write(receipts.join("receipt.json"), b"receipt").unwrap();
    let pinned = open_directory(&receipts).unwrap();
    let entry = fs::read_dir(directory_path(&pinned))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    fs::remove_file(entry.path()).unwrap();
    std::os::unix::fs::symlink("/unmounted-host-data", entry.path()).unwrap();
    let mut usage = EvidenceUsage {
        bytes: 0,
        entries: 0,
        maximum: 1024,
    };
    assert!(matches!(
        account_file(&pinned, &entry.file_name(), &mut usage),
        Err(SupervisorError::UnsafeSurface)
    ));
}

#[test]
fn directory_replacement_after_open_cannot_redirect_receipt_metadata() {
    let directory = Directory::new();
    let _mailbox = directory.mailbox();
    let receipts = directory.epoch().join("input-receipts");
    fs::write(receipts.join("receipt.json"), b"original").unwrap();
    let pinned = open_directory(&receipts).unwrap();
    let outside = directory.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(
        outside.join("receipt.json"),
        b"foreign bytes must not be counted",
    )
    .unwrap();
    fs::rename(&receipts, directory.0.join("moved-receipts")).unwrap();
    std::os::unix::fs::symlink(&outside, &receipts).unwrap();
    let mut usage = EvidenceUsage {
        bytes: 0,
        entries: 0,
        maximum: 8,
    };
    account_receipts(&pinned, &mut usage).unwrap();
    assert_eq!(usage.bytes, 8);
}

#[test]
fn receipt_entries_share_the_bounded_scan_limit() {
    let directory = Directory::new();
    let _mailbox = directory.mailbox();
    let receipts = directory.epoch().join("input-receipts");
    fs::write(receipts.join("empty.json"), []).unwrap();
    let pinned = open_directory(&receipts).unwrap();
    let mut usage = EvidenceUsage {
        bytes: 0,
        entries: 100_000,
        maximum: 1024,
    };
    assert!(matches!(
        account_receipts(&pinned, &mut usage),
        Err(SupervisorError::EvidenceCapacity)
    ));
}
