use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{Arc, Barrier, Mutex},
};

use forge_provider_common::{
    AuthSnapshotLease, AuthWriteback, AuthWritebackConflict, MasterKeySource,
    PrivateMaterialization, SecretBytes, SecretScope, SecretStore, SecretStoreError,
    enroll_codex_auth, evaluate_auth_writeback,
};
use uuid::Uuid;

fn store() -> (tempfile::TempDir, SecretStore) {
    let directory = private_tempdir();
    let store = SecretStore::initialize(directory.path(), MasterKeySource::OwnerOnlyFile).unwrap();
    (directory, store)
}

fn private_tempdir() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

#[test]
fn rooted_runtime_read_rejects_symlinked_ancestors_and_parent_escape() {
    let root = private_tempdir();
    let outside = private_tempdir();
    PrivateMaterialization::create(
        &outside.path().join("auth.json"),
        &SecretBytes::new(b"synthetic-outside".to_vec()),
    )
    .unwrap();
    symlink(outside.path(), root.path().join("codex-home")).unwrap();
    assert!(
        PrivateMaterialization::read_beneath(
            root.path(),
            std::path::Path::new("codex-home/auth.json"),
            1024
        )
        .is_err()
    );
    assert!(
        PrivateMaterialization::read_beneath(
            root.path(),
            std::path::Path::new("../auth.json"),
            1024
        )
        .is_err()
    );
    PrivateMaterialization::create(
        &root.path().join("safe"),
        &SecretBytes::new(b"synthetic-safe".to_vec()),
    )
    .unwrap();
    assert_eq!(
        PrivateMaterialization::read_beneath(root.path(), std::path::Path::new("safe"), 1024)
            .unwrap()
            .expose(),
        b"synthetic-safe"
    );
}

#[test]
fn rooted_cleanup_removes_only_a_private_leaf_and_preserves_outside_files() {
    let root = private_tempdir();
    let outside = private_tempdir();
    let secret = SecretBytes::new(b"synthetic-cleanup".to_vec());
    PrivateMaterialization::create(&outside.path().join("auth.json"), &secret).unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    assert!(
        PrivateMaterialization::cleanup_beneath(
            root.path(),
            std::path::Path::new("escape/auth.json")
        )
        .is_err()
    );
    assert!(outside.path().join("auth.json").exists());
    fs::create_dir(root.path().join("run")).unwrap();
    fs::set_permissions(root.path().join("run"), fs::Permissions::from_mode(0o700)).unwrap();
    PrivateMaterialization::create(&root.path().join("run/auth.json"), &secret).unwrap();
    assert!(
        PrivateMaterialization::cleanup_beneath(root.path(), std::path::Path::new("run/auth.json"))
            .unwrap()
    );
    assert!(
        !PrivateMaterialization::cleanup_beneath(
            root.path(),
            std::path::Path::new("run/auth.json")
        )
        .unwrap()
    );
    assert!(root.path().join("run").is_dir());
}

fn scope() -> SecretScope {
    SecretScope {
        secret_id: Uuid::now_v7(),
        project_id: Uuid::now_v7(),
        version: 1,
        purpose: "test".into(),
    }
}

fn auth(account: &str, refresh: &str, token: &str) -> SecretBytes {
    SecretBytes::new(format!(r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"{account}","access_token":"synthetic-access-{token}","refresh_token":"synthetic-refresh-{token}","id_token":"synthetic-id-{token}"}},"last_refresh":"{refresh}"}}"#).into_bytes())
}

#[test]
fn encryption_roundtrips_after_reload_without_plaintext_in_record_or_debug() {
    let (directory, store) = store();
    let secret = SecretBytes::new(b"synthetic-super-private-token".to_vec());
    let scope = scope();
    let sealed = store.seal(scope.clone(), &secret).unwrap();
    let encoded = serde_json::to_string(&sealed).unwrap();
    assert!(!encoded.contains("synthetic-super-private-token"));
    assert!(!format!("{store:?} {secret:?} {sealed:?}").contains("synthetic-super-private-token"));
    let reloaded = SecretStore::load(directory.path()).unwrap();
    assert_eq!(
        reloaded.open(&sealed, &scope).unwrap().expose(),
        secret.expose()
    );
}

#[test]
fn ciphertext_and_nonce_tampering_are_rejected() {
    let (_directory, store) = store();
    let scope = scope();
    let mut sealed = store
        .seal(scope.clone(), &SecretBytes::new(b"fixture".to_vec()))
        .unwrap();
    sealed.ciphertext[0] ^= 1;
    assert!(matches!(
        store.open(&sealed, &scope),
        Err(SecretStoreError::Authentication)
    ));
    sealed.ciphertext[0] ^= 1;
    sealed.nonce[0] ^= 1;
    assert!(matches!(
        store.open(&sealed, &scope),
        Err(SecretStoreError::Authentication)
    ));
}

#[test]
fn another_scope_or_master_key_cannot_decrypt_a_record() {
    let (_directory, store) = store();
    let (_other_directory, other_store) = self::store();
    let scope = scope();
    let sealed = store
        .seal(scope.clone(), &SecretBytes::new(b"fixture".to_vec()))
        .unwrap();
    assert!(matches!(
        other_store.open(&sealed, &scope),
        Err(SecretStoreError::Authentication)
    ));
    let mut changed_scope = scope.clone();
    changed_scope.version += 1;
    let mut tampered = sealed;
    tampered.scope = changed_scope.clone();
    assert!(matches!(
        store.open(&tampered, &changed_scope),
        Err(SecretStoreError::Authentication)
    ));
}

#[test]
fn every_seal_gets_a_fresh_nonce() {
    let (_directory, store) = store();
    let scope = scope();
    let secret = SecretBytes::new(b"fixture".to_vec());
    assert_ne!(
        store.seal(scope.clone(), &secret).unwrap().nonce,
        store.seal(scope, &secret).unwrap().nonce
    );
}

#[test]
fn missing_master_key_is_not_recreated_or_replaced() {
    let (directory, store) = store();
    let binding = store.binding().clone();
    fs::remove_file(directory.path().join("master.key")).unwrap();
    assert!(matches!(
        SecretStore::load(directory.path()),
        Err(SecretStoreError::KeyUnavailable)
    ));
    assert!(SecretStore::initialize(directory.path(), MasterKeySource::OwnerOnlyFile).is_err());
    assert!(!directory.path().join("master.key").exists());
    let disk: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.path().join("master-key-binding.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(disk["key_id"], binding.key_id.to_string());
}

#[test]
fn existing_binding_is_never_replaced_to_change_provider() {
    let (directory, store) = store();
    assert!(SecretStore::initialize(directory.path(), MasterKeySource::LinuxKeyring).is_err());
    assert_eq!(
        SecretStore::load(directory.path()).unwrap().binding(),
        store.binding()
    );
}

#[test]
fn unsafe_key_permissions_and_symlinks_are_rejected() {
    let (directory, _store) = store();
    let key = directory.path().join("master.key");
    fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        SecretStore::load(directory.path()),
        Err(SecretStoreError::UnsafePermissions)
    ));
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    let relocated = directory.path().join("relocated.key");
    fs::rename(&key, &relocated).unwrap();
    symlink(&relocated, &key).unwrap();
    assert!(SecretStore::load(directory.path()).is_err());
}

#[test]
fn a_world_readable_store_directory_is_not_repaired_silently() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        SecretStore::initialize(directory.path(), MasterKeySource::OwnerOnlyFile),
        Err(SecretStoreError::UnsafePermissions)
    ));
}

#[test]
fn private_materialization_is_bounded_owner_only_and_explicitly_cleaned() {
    let directory = private_tempdir();
    let path = directory.path().join("auth.json");
    let secret = SecretBytes::new(b"synthetic".to_vec());
    let materialized = PrivateMaterialization::create(&path, &secret).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(materialized.read(32).unwrap().expose(), secret.expose());
    assert!(matches!(
        materialized.read(2),
        Err(SecretStoreError::AuthTooLarge)
    ));
    assert!(PrivateMaterialization::create(&path, &secret).is_err());
    materialized.cleanup().unwrap();
    assert!(!path.exists());
}

#[test]
fn runtime_replacing_auth_with_fifo_does_not_block_collection() {
    let directory = private_tempdir();
    let path = directory.path().join("auth.json");
    let materialized =
        PrivateMaterialization::create(&path, &SecretBytes::new(b"fixture".to_vec())).unwrap();
    fs::remove_file(&path).unwrap();
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &path,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    assert!(matches!(
        materialized.read(1024),
        Err(SecretStoreError::UnsafePermissions)
    ));
    materialized.cleanup().unwrap();
}

#[test]
fn hard_linked_key_is_rejected() {
    let (directory, _store) = store();
    fs::hard_link(
        directory.path().join("master.key"),
        directory.path().join("key-copy"),
    )
    .unwrap();
    assert!(matches!(
        SecretStore::load(directory.path()),
        Err(SecretStoreError::UnsafePermissions)
    ));
}

#[test]
fn late_run_cannot_write_back_into_a_new_login_of_the_same_account() {
    let (_directory, store) = store();
    let original = auth("one", "2026-09-05T10:00:00Z", "original");
    let base = enroll_codex_auth(
        &store,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        &original,
    )
    .unwrap();
    let lease = AuthSnapshotLease {
        run_id: Uuid::now_v7(),
        environment_epoch: 1,
        base: base.clone(),
    };
    let reenrolled = enroll_codex_auth(
        &store,
        base.binding_id,
        base.auth.scope.project_id,
        base.auth.scope.secret_id,
        &original,
    )
    .unwrap();
    let candidate = auth("one", "2026-09-05T10:01:00Z", "late");
    let result = evaluate_auth_writeback(&store, &reenrolled, &lease, &candidate).unwrap();
    assert!(matches!(
        result,
        AuthWriteback::Conflict {
            reason: AuthWritebackConflict::SessionChanged,
            ..
        }
    ));
}

#[test]
fn changed_account_is_encrypted_as_conflict_not_applied() {
    let (_directory, store) = store();
    let base = enroll_codex_auth(
        &store,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        &auth("one", "2026-09-05T10:00:00Z", "original"),
    )
    .unwrap();
    let lease = AuthSnapshotLease {
        run_id: Uuid::now_v7(),
        environment_epoch: 1,
        base: base.clone(),
    };
    let candidate = auth("two", "2026-09-05T10:01:00Z", "new");
    let result = evaluate_auth_writeback(&store, &base, &lease, &candidate).unwrap();
    let AuthWriteback::Conflict { reason, recovery } = result else {
        panic!("expected conflict")
    };
    assert_eq!(reason, AuthWritebackConflict::AccountChanged);
    assert_eq!(
        store.open(&recovery, &recovery.scope).unwrap().expose(),
        candidate.expose()
    );
}

#[test]
fn simultaneous_refreshes_produce_one_replacement_and_one_recoverable_conflict() {
    let (_directory, store) = store();
    let base = enroll_codex_auth(
        &store,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        &auth("one", "2026-09-05T10:00:00Z", "original"),
    )
    .unwrap();
    let current = Arc::new(Mutex::new(base.clone()));
    let store = Arc::new(store);
    let start = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["left", "right"]
        .into_iter()
        .map(|token| {
            let current = Arc::clone(&current);
            let store = Arc::clone(&store);
            let start = Arc::clone(&start);
            let lease = AuthSnapshotLease {
                run_id: Uuid::now_v7(),
                environment_epoch: 1,
                base: base.clone(),
            };
            std::thread::spawn(move || {
                let candidate = auth("one", "2026-09-05T10:01:00Z", token);
                start.wait();
                // Equivalent to Core's row lock; execution itself remains concurrent.
                let mut row = current.lock().unwrap();
                match evaluate_auth_writeback(&store, &row, &lease, &candidate).unwrap() {
                    AuthWriteback::Replace {
                        expected_version,
                        snapshot,
                    } => {
                        assert_eq!(row.auth.scope.version, expected_version);
                        *row = snapshot;
                        true
                    }
                    AuthWriteback::Conflict { reason, recovery } => {
                        assert_eq!(reason, AuthWritebackConflict::ConcurrentRefresh);
                        assert_eq!(
                            store.open(&recovery, &recovery.scope).unwrap().expose(),
                            candidate.expose()
                        );
                        false
                    }
                    AuthWriteback::Unchanged => panic!("both candidates are changed"),
                }
            })
        })
        .collect();
    let applied = handles
        .into_iter()
        .map(|handle| usize::from(handle.join().unwrap()))
        .sum::<usize>();
    assert_eq!(applied, 1);
    assert_eq!(current.lock().unwrap().auth.scope.version, 2);
}

#[test]
fn stale_unchanged_auth_does_not_roll_back_a_refreshed_snapshot() {
    let (_directory, store) = store();
    let original = auth("one", "2026-09-05T10:00:00Z", "original");
    let base = enroll_codex_auth(
        &store,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        &original,
    )
    .unwrap();
    let lease = AuthSnapshotLease {
        run_id: Uuid::now_v7(),
        environment_epoch: 1,
        base: base.clone(),
    };
    let AuthWriteback::Replace { snapshot, .. } = evaluate_auth_writeback(
        &store,
        &base,
        &lease,
        &auth("one", "2026-09-05T10:01:00Z", "new"),
    )
    .unwrap() else {
        panic!("expected replacement")
    };
    assert!(matches!(
        evaluate_auth_writeback(&store, &snapshot, &lease, &original).unwrap(),
        AuthWriteback::Unchanged
    ));
}

#[test]
fn unparseable_and_non_newer_auth_are_retained_as_conflicts() {
    let (_directory, store) = store();
    let base = enroll_codex_auth(
        &store,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        &auth("one", "2026-09-05T10:00:00Z", "original"),
    )
    .unwrap();
    let lease = AuthSnapshotLease {
        run_id: Uuid::now_v7(),
        environment_epoch: 1,
        base: base.clone(),
    };
    for (candidate, expected) in [
        (
            SecretBytes::new(b"not auth JSON".to_vec()),
            AuthWritebackConflict::InvalidCandidate,
        ),
        (
            auth("one", "2026-09-05T09:00:00Z", "old"),
            AuthWritebackConflict::RefreshNotNewer,
        ),
    ] {
        let AuthWriteback::Conflict { reason, .. } =
            evaluate_auth_writeback(&store, &base, &lease, &candidate).unwrap()
        else {
            panic!("expected conflict")
        };
        assert_eq!(reason, expected);
    }
}

#[test]
#[ignore = "requires explicitly available Linux Secret Service; creates/deletes a synthetic test key"]
fn linux_keyring_roundtrip_and_deleted_key_fail_closed() {
    let directory = private_tempdir();
    let store = SecretStore::initialize(directory.path(), MasterKeySource::LinuxKeyring).unwrap();
    assert_eq!(
        SecretStore::load(directory.path()).unwrap().binding(),
        store.binding()
    );
    keyring::Entry::new("forge-secret-store", &store.binding().key_id.to_string())
        .unwrap()
        .delete_credential()
        .unwrap();
    assert!(matches!(
        SecretStore::load(directory.path()),
        Err(SecretStoreError::KeyUnavailable)
    ));
    assert!(!directory.path().join("master.key").exists());
}
