use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

use axum::{body::to_bytes, response::IntoResponse};
use bytes::Bytes;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
};

use crate::{
    command::{AmendDraft, RECEIPT_LIMIT},
    command_tests::{receipt, request},
    core_client::CoreClient,
    http::ApiError,
};

pub(crate) async fn fake_core(
    response: Vec<u8>,
    delay: Duration,
) -> (
    tempfile::TempDir,
    CoreClient,
    tokio::task::JoinHandle<Vec<u8>>,
) {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("core.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            request.push(socket.read_u8().await.unwrap());
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
            assert!(request.len() < 4096);
        }
        let headers = String::from_utf8(request.clone()).unwrap();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap_or(0);
        let mut body = vec![0; length];
        socket.read_exact(&mut body).await.unwrap();
        request.extend(body);
        tokio::time::sleep(delay).await;
        let _ = socket.write_all(&response).await;
        request
    });
    (dir, CoreClient { socket: path }, server)
}

pub(crate) fn response(status: u16, body: &[u8]) -> Vec<u8> {
    [format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes(),body.to_vec()].concat()
}

async fn invoke(client: &CoreClient) -> Result<Bytes, ApiError> {
    let bytes = serde_json::to_vec(&request()).unwrap();
    let command = AmendDraft::parse(&bytes).unwrap();
    client
        .amend_draft(&command, "synthetic-request-key", bytes.into())
        .await
}

#[tokio::test]
async fn post_sends_only_the_fixed_target_key_and_original_body_and_accepts_replay() {
    for status in ["applied", "replayed"] {
        let body = serde_json::to_vec(&receipt(status)).unwrap();
        let (_dir, client, server) = fake_core(response(200, &body), Duration::ZERO).await;
        assert_eq!(invoke(&client).await.unwrap(), body);
        let request_bytes = server.await.unwrap();
        let headers_end = request_bytes
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = std::str::from_utf8(&request_bytes[..headers_end]).unwrap();
        assert!(headers.starts_with("POST /v1/commands/amend_draft HTTP/1.1\r\n"));
        assert!(headers.contains("idempotency-key: synthetic-request-key\r\n"));
        assert!(
            !headers.contains("authorization")
                && !headers.contains("actor")
                && !headers.contains("origin")
        );
        assert_eq!(
            &request_bytes[headers_end..],
            serde_json::to_vec(&request()).unwrap()
        );
    }
}

#[tokio::test]
async fn only_known_status_code_pairs_are_definite_safe_refusals() {
    for (status, code) in [
        (400, "invalid_request"),
        (409, "stale_revision"),
        (409, "conflict"),
        (409, "idempotency_conflict"),
        (422, "validation_failed"),
        (404, "not_found"),
        (403, "forbidden"),
    ] {
        let body = serde_json::to_vec(&serde_json::json!({"error":{
            "code":code,"message":"PRIVATE diagnostic","details":{"secret":"PRIVATE"},"request_id":"PRIVATE"
        }})).unwrap();
        let (_dir, client, server) = fake_core(response(status, &body), Duration::ZERO).await;
        let safe = invoke(&client).await.unwrap_err().into_response();
        assert_eq!(safe.status().as_u16(), status);
        let bytes = to_bytes(safe.into_body(), 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["code"], code);
        assert!(!std::str::from_utf8(&bytes).unwrap().contains("PRIVATE"));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn unknown_or_malformed_responses_remain_ambiguous() {
    for (status, body) in [
        (
            409,
            serde_json::json!({"error":{"code":"invalid_request","message":"private"}}),
        ),
        (
            400,
            serde_json::json!({"error":{"code":"stale_revision","message":"private"}}),
        ),
        (404, serde_json::json!({"code":"not_found"})),
        (
            422,
            serde_json::json!({"error":{"code":"new_refusal","message":"private"}}),
        ),
        (
            500,
            serde_json::json!({"error":{"code":"internal","message":"private"}}),
        ),
        (201, receipt("applied")),
        (200, serde_json::json!({"status":"applied"})),
    ] {
        let (_dir, client, server) = fake_core(
            response(status, &serde_json::to_vec(&body).unwrap()),
            Duration::ZERO,
        )
        .await;
        assert!(matches!(invoke(&client).await, Err(ApiError::BadGateway)));
        server.await.unwrap();
    }
    for raw in [
        b"HTTP/1.1 302 Found\r\nLocation: http://evil.invalid\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        response(200, b"not json"),
    ] {
        let (_dir, client, server) = fake_core(raw, Duration::ZERO).await;
        assert!(matches!(invoke(&client).await, Err(ApiError::BadGateway)));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn receipts_are_bounded_for_declared_and_chunked_responses_including_errors() {
    for status in [200, 400, 409, 422, 503] {
        let declared = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n\r\n",
            RECEIPT_LIMIT + 1
        )
        .into_bytes();
        let chunked = [
            format!(
                "HTTP/1.1 {status} Test\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
                RECEIPT_LIMIT + 1
            )
            .into_bytes(),
            vec![b'x'; RECEIPT_LIMIT + 1],
            b"\r\n0\r\n\r\n".to_vec(),
        ]
        .concat();
        for raw in [declared, chunked] {
            let (_dir, client, server) = fake_core(raw, Duration::ZERO).await;
            assert!(matches!(
                invoke(&client).await,
                Err(ApiError::ResponseTooLarge)
            ));
            server.await.unwrap();
        }
    }
    let mut body = serde_json::to_vec(&receipt("applied")).unwrap();
    body.resize(RECEIPT_LIMIT, b' ');
    let (_dir, client, server) = fake_core(response(200, &body), Duration::ZERO).await;
    assert_eq!(invoke(&client).await.unwrap().len(), RECEIPT_LIMIT);
    server.await.unwrap();
}

#[tokio::test]
async fn unavailable_socket_and_stalled_command_do_not_claim_rejection() {
    let (_dir, client, server) = fake_core(Vec::new(), Duration::from_secs(60)).await;
    assert!(matches!(invoke(&client).await, Err(ApiError::Timeout)));
    server.abort();
    fs::set_permissions(&client.socket, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(invoke(&client).await, Err(ApiError::Unavailable)));
}
