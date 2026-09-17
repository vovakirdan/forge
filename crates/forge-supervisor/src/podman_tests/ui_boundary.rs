//! Physical sandbox-ingress acceptance; synthetic executable, no inference.

use super::*;
use tokio::net::TcpListener;

#[tokio::test]
#[ignore = "requires rootless Podman and TASK-12 fixture image; no credentials or inference"]
async fn actual_run_sandbox_cannot_reach_owner_ui_or_control_material() {
    let fixture = Fixture::new(true).await;
    let owner = fixture.config.state_directory.join("owner-ui");
    private_directory(&owner).unwrap();
    let core_path = owner.join("api.sock");
    let control_path = owner.join("ui-control.sock");
    let _core = UnixListener::bind(&core_path).unwrap();
    let _control = UnixListener::bind(&control_path).unwrap();
    private_write(
        &owner.join("bootstrap"),
        b"synthetic-owner-code-not-for-run",
    )
    .unwrap();
    private_write(&owner.join("session"), b"synthetic-owner-token-not-for-run").unwrap();
    // A live host listener, not just inspection of --network=none arguments.
    let ui = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = ui.local_addr().unwrap().port();
    let worker = running_fixture(&fixture).await;
    let name = format!(
        "forge-run-{}-{}-{}",
        fixture.provision.run_id,
        fixture.provision.lease_fencing_token,
        fixture.provision.environment_epoch
    );
    let probe = Command::new("podman")
        .args([
            "exec", &name, "/bin/bash", "--noprofile", "--norc", "-c",
            r#"
set -eu
for host in 127.0.0.1 localhost host.containers.internal host.docker.internal 10.0.2.2; do
    if timeout 2 bash --noprofile --norc -c 'exec 3<>/dev/tcp/"$1"/"$2"' probe "$host" "$1" 2>/dev/null; then
        exit 41
    fi
done
for path in "$2" "$3" "$4/bootstrap" "$4/session"; do
    test ! -e "$path" || exit 42
done
test ! -S /run/forge/api.sock
test ! -S /run/forge/ui-control.sock
test ! -S /run/user/$(id -u)/forge/api.sock
test ! -S /run/user/$(id -u)/forge/ui-control.sock
test -d /workspace
printf 'owner-ingress-denied\n'
"#,
            "probe", &port.to_string(),
            core_path.to_str().unwrap(), control_path.to_str().unwrap(),
            owner.to_str().unwrap(),
        ])
        .kill_on_drop(true)
        .output();
    let result = timeout(Duration::from_secs(20), probe).await;
    let unexpected_connection = timeout(Duration::from_millis(150), ui.accept()).await;
    // Keep cleanup ahead of assertions, including a failed network probe.
    request_fixture_stop(&fixture).await;
    await_container_exit(&fixture).await;
    let worker_result = timeout(Duration::from_secs(10), worker).await;
    fixture.cleanup().await;
    let output = result.unwrap().unwrap();
    assert!(
        output.status.success(),
        "sandbox owner-ingress probe failed"
    );
    assert_eq!(output.stdout, b"owner-ingress-denied\n");
    assert!(
        unexpected_connection.is_err(),
        "host listener was reached from Run"
    );
    worker_result.unwrap().unwrap().unwrap();
}
