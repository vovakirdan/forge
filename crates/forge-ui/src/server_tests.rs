use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::watch,
    task::JoinHandle,
};

use crate::{Gateway, GatewayConfig, GatewayError};

struct Server {
    _directory: tempfile::TempDir,
    control: std::path::PathBuf,
    address: String,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<Result<(), GatewayError>>,
}

impl Server {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(directory.path().join("index.html"), "<html></html>").unwrap();
        fs::write(directory.path().join("forge-live-manifest.json"),serde_json::to_vec(&serde_json::json!({
            "format":"forge-live-v1","files":[{"path":"index.html","sha256":format!("{:x}",Sha256::digest(b"<html></html>"))}]
        })).unwrap()).unwrap();
        let control = directory.path().join("ui.sock");
        let gateway = Gateway::bind(GatewayConfig {
            assets_dir: directory.path().to_path_buf(),
            control_socket: control.clone(),
            core_socket: directory.path().join("absent-core.sock"),
        })
        .await
        .unwrap();
        let address = gateway.origin().strip_prefix("http://").unwrap().to_owned();
        let (shutdown, receiver) = watch::channel(false);
        let task = tokio::spawn(gateway.serve(receiver));
        Self {
            _directory: directory,
            control,
            address,
            shutdown,
            task,
        }
    }

    async fn request(&self, content: &[u8]) -> Vec<u8> {
        let mut stream = TcpStream::connect(&self.address).await.unwrap();
        stream.write_all(content).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(7), stream.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        response
    }

    async fn stop(self) {
        self.shutdown.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!self.control.exists());
        assert!(TcpStream::connect(self.address).await.is_err());
    }
}

#[tokio::test]
async fn real_http_parser_closes_header_overflow_and_body_deadlines() {
    let server = Server::start().await;
    let oversized = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nX-Large: {}\r\n\r\n",
        server.address,
        "x".repeat(20 * 1024)
    );
    let response = server.request(oversized.as_bytes()).await;
    assert!(response.starts_with(b"HTTP/1.1 431"));
    let slow_body = format!(
        "POST /api/auth/exchange HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{",
        server.address, server.address
    );
    let response = server.request(slow_body.as_bytes()).await;
    assert!(response.starts_with(b"HTTP/1.1 504"));
    server.stop().await;
}

#[tokio::test]
async fn real_listener_caps_idle_connections_and_shutdown_joins_them() {
    let server = Server::start().await;
    let mut pending_control = forge_protocol::ui_control::connect_owner_socket(&server.control)
        .await
        .unwrap();
    pending_control.write_all(&[0, 0]).await.unwrap();
    let mut connections = Vec::new();
    for _ in 0..32 {
        connections.push(TcpStream::connect(&server.address).await.unwrap());
    }
    let mut excess = TcpStream::connect(&server.address).await.unwrap();
    let read = tokio::time::timeout(Duration::from_secs(1), excess.read_u8())
        .await
        .unwrap();
    assert!(read.is_err());
    server.stop().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), pending_control.read_u8())
            .await
            .unwrap()
            .is_err()
    );
    for mut stream in connections {
        assert!(
            tokio::time::timeout(Duration::from_secs(1), stream.read_u8())
                .await
                .unwrap()
                .is_err()
        );
    }
}

#[tokio::test]
async fn partial_headers_cannot_hold_a_connection_past_the_deadline() {
    let server = Server::start().await;
    let started = tokio::time::Instant::now();
    let response = server.request(b"GET / HTTP/1.1\r\nHost:").await;
    assert!(response.is_empty() || response.starts_with(b"HTTP/1.1 408"));
    assert!(started.elapsed() < Duration::from_secs(7));
    server.stop().await;
}

#[tokio::test]
async fn control_code_crosses_real_http_boundary_once_without_http_mint() {
    let server = Server::start().await;
    let code = forge_protocol::ui_control::request_login_code(&server.control)
        .await
        .unwrap();
    let body = serde_json::to_string(&serde_json::json!({"code":code.code})).unwrap();
    let request = format!(
        "POST /api/auth/exchange HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        server.address,
        server.address,
        body.len(),
        body
    );
    let response = server.request(request.as_bytes()).await;
    assert!(response.starts_with(b"HTTP/1.1 200"));
    assert!(
        !response
            .windows(code.code.len())
            .any(|window| window == code.code.as_bytes())
    );
    assert!(
        server
            .request(request.as_bytes())
            .await
            .starts_with(b"HTTP/1.1 401")
    );
    let mint = format!(
        "POST /api/auth/issue HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nContent-Length: 0\r\n\r\n",
        server.address, server.address
    );
    assert!(
        server
            .request(mint.as_bytes())
            .await
            .starts_with(b"HTTP/1.1 404")
    );
    server.stop().await;
}
