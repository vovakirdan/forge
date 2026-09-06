use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    time::Duration,
};

use super::{
    OUTBOX_STREAM_NAME, OUTBOX_SUBJECT, bind_owner_socket, m0_actors, outbox_stream_config,
    prepare_socket_parent, spawn_outbox_reconnector,
};
use uuid::Uuid;

#[test]
fn m0_actor_identity_is_stable_across_daemon_restarts() {
    let first = m0_actors();
    let second = m0_actors();

    assert_eq!(first.human.id(), second.human.id());
    assert_eq!(first.core.id(), second.core.id());
    assert_eq!(first.human.id().as_uuid().get_version_num(), 7);
    assert_eq!(first.core.id().as_uuid().get_version_num(), 7);
}

#[test]
fn forge_owned_socket_parent_is_tightened_before_listening() {
    let parent = std::env::temp_dir().join(format!("forge-core-{}", Uuid::now_v7()));
    fs::create_dir_all(&parent).expect("create test directory");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755))
        .expect("make fixture permissive");

    prepare_socket_parent(&parent, true).expect("tighten Forge-owned directory");

    let mode = fs::metadata(&parent)
        .expect("read test directory")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
    fs::remove_dir(&parent).expect("remove empty test directory");
}

#[test]
fn second_core_socket_bind_refuses_a_live_listener() {
    let parent = std::env::temp_dir().join(format!("forge-core-live-{}", Uuid::now_v7()));
    fs::create_dir(&parent).expect("create test directory");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
        .expect("restrict test directory");
    let socket = parent.join("api.sock");
    let first_listener = UnixListener::bind(&socket).expect("bind first Core socket");
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
        .expect("restrict first Core socket");

    let error = bind_owner_socket(&socket, true).expect_err("reject second Core socket");
    assert!(error.to_string().contains("live Forge socket"));

    drop(first_listener);
    fs::remove_file(&socket).expect("remove first Core socket");
    fs::remove_dir(&parent).expect("remove test directory");
}

#[test]
fn outbox_stream_uses_the_canonical_event_subject() {
    let stream = outbox_stream_config();

    assert_eq!(stream.name, OUTBOX_STREAM_NAME);
    assert_eq!(stream.subjects, [OUTBOX_SUBJECT]);
}

#[test]
fn nats_token_url_exposes_a_single_credential_component() {
    let address = "nats://local-token@127.0.0.1:4222"
        .parse::<async_nats::ServerAddr>()
        .expect("parse local token URL");

    assert_eq!(address.username(), Some("local-token"));
    assert_eq!(address.password(), None);
}

#[tokio::test]
async fn malformed_nats_url_stays_in_the_background_reconnect_loop() {
    let task = spawn_outbox_reconnector("://not-a-nats-url".to_owned(), |_| async {});

    tokio::time::sleep(Duration::from_millis(20)).await;

    assert!(
        !task.is_finished(),
        "a malformed optional broker URL must not end the Core-owned reconnect task"
    );
    task.abort();
    let error = task
        .await
        .expect_err("aborted reconnect task must not complete");
    assert!(error.is_cancelled());
}
