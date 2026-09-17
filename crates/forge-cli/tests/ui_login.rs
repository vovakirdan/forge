//! No real credential is issued: the private test listener returns a synthetic code.

use std::{
    fs::{self, DirBuilder},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};

use forge_protocol::ui_control::{
    ControlRequest, ControlResponse, LoginCodeResponse, read_frame, write_frame,
};
use tokio::{net::UnixListener, process::Command, time::timeout};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("forge-cli-ui-{}", uuid::Uuid::now_v7()));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn cli_command(path: &std::path::Path) -> String {
    format!(
        "{} ui login --control-socket {}",
        shell_quote(env!("CARGO_BIN_EXE_forge-cli")),
        shell_quote(path.to_str().unwrap())
    )
}

#[tokio::test]
async fn no_terminal_refuses_before_connecting_or_issuing_code() {
    let directory = Directory::new();
    let path = directory.0.join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_forge-cli"))
        .kill_on_drop(true)
        .args(["ui", "login", "--control-socket"])
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("controlling terminal"));
    assert!(
        timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn redirected_output_with_real_controlling_pty_still_refuses_before_issuance() {
    let directory = Directory::new();
    let path = directory.0.join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let command = format!("{} >/dev/null", cli_command(&path));
    let output = Command::new("script")
        .kill_on_drop(true)
        .args(["--quiet", "--return", "--command", &command, "/dev/null"])
        .stdin(Stdio::null())
        .output()
        .await
        .expect("Linux acceptance requires util-linux script for real PTY proof");
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Code:"));
    assert!(
        timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn real_controlling_pty_receives_only_validated_synthetic_login_fields() {
    let directory = Directory::new();
    let path = directory.0.join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert_eq!(
            read_frame::<ControlRequest>(&mut stream).await.unwrap(),
            ControlRequest::IssueLoginCode {}
        );
        write_frame(
            &mut stream,
            &ControlResponse::Login(LoginCodeResponse {
                origin: "http://127.0.0.1:45000".into(),
                code: "a".repeat(64),
                expires_at: "2026-09-17T12:00:00Z".into(),
            }),
        )
        .await
        .unwrap();
    });
    let output = timeout(
        Duration::from_secs(10),
        Command::new("script")
            .kill_on_drop(true)
            .args([
                "--quiet",
                "--return",
                "--command",
                &cli_command(&path),
                "/dev/null",
            ])
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .unwrap()
    .expect("Linux acceptance requires util-linux script for real PTY proof");
    assert!(
        output.status.success(),
        "PTY login failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("Origin: http://127.0.0.1:45000"));
    assert!(output.contains(&format!("Code: {}", "a".repeat(64))));
    assert!(output.contains("Expires: 2026-09-17T12:00:00Z"));
    timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
}
