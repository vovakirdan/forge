//! Stop revocation while a request body or upstream response is still pending.

use super::*;
use anyhow::ensure;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

pub(super) async fn inference(
    State(broker): State<Arc<Broker>>,
    Json(_request): Json<Value>,
) -> Json<Value> {
    broker.inference_calls.fetch_add(1, Ordering::SeqCst);
    broker.inference_started.notify_one();
    broker.release_inference_headers.notified().await;
    Json(json!({"choices":[{"message":{"role":"assistant","content":"synthetic reply"}}]}))
}

async fn started_fixture() -> Result<(Fixture, reqwest::Client, Value)> {
    let fixture = Fixture::new().await?;
    fixture.broker.release_create.notify_one();
    fixture.harness.start_project(fixture.project).await?;
    let run = fixture
        .harness
        .wait_for_run_count(fixture.task, 1)
        .await?
        .remove(0);
    let socket = fixture
        .root
        .join("gateways")
        .join(run.id.to_string())
        .join("gateway.sock");
    let client = reqwest::Client::builder()
        .unix_socket(socket)
        .timeout(Duration::from_secs(10))
        .build()?;
    let model = fixture
        .broker
        .key
        .lock()
        .await
        .as_ref()
        .and_then(|key| key["models"][0].as_str())
        .context("issued model alias")?
        .to_owned();
    Ok((
        fixture,
        client,
        json!({"model":model,"messages":[{"role":"user","content":"synthetic request"}]}),
    ))
}

async fn response_headers(stream: &mut UnixStream) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            ensure!(
                bytes.len() < 16 * 1024,
                "response headers exceeded fixture bound"
            );
            bytes.push(stream.read_u8().await?);
        }
        Ok(String::from_utf8(bytes)?)
    })
    .await
    .context("Gateway did not return response headers")?
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic inference only, run serially"]
async fn slow_inference_body_completed_after_stop_never_reaches_upstream() -> Result<()> {
    let (fixture, client, body) = started_fixture().await?;
    // Prove that this exact model and body reach the broker while authorized.
    fixture.broker.release_inference_headers.notify_one();
    let allowed = client
        .post("http://localhost/v1/chat/completions")
        .json(&body)
        .send()
        .await?;
    assert_eq!(allowed.status(), StatusCode::OK);
    let _: Value = allowed.json().await?;
    assert_eq!(fixture.broker.inference_calls.load(Ordering::SeqCst), 1);

    let run = fixture
        .harness
        .wait_for_run_count(fixture.task, 1)
        .await?
        .remove(0);
    let socket = fixture
        .root
        .join("gateways")
        .join(run.id.to_string())
        .join("gateway.sock");
    let mut stream = UnixStream::connect(socket).await?;
    let body = serde_json::to_vec(&body)?;
    let headers = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    // Hyper sends this only when the handler polls the body. The relay has
    // already committed its initial scope/key check at that point.
    let interim = response_headers(&mut stream).await?;
    ensure!(
        interim.starts_with("HTTP/1.1 100 Continue\r\n"),
        "expected body-consumption acknowledgement"
    );
    let split = body.len() / 2;
    stream.write_all(&body[..split]).await?;
    fixture.harness.stop_project(fixture.project).await?;
    stream.write_all(&body[split..]).await?;
    let rejected = response_headers(&mut stream).await?;
    ensure!(
        rejected.starts_with("HTTP/1.1 403 Forbidden\r\n"),
        "revoked request was not denied"
    );
    assert_eq!(
        fixture.broker.inference_calls.load(Ordering::SeqCst),
        1,
        "body completed after Stop must not cause another upstream dispatch"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic inference only, run serially"]
async fn stop_cancels_inference_waiting_for_upstream_headers() -> Result<()> {
    let (fixture, client, body) = started_fixture().await?;
    let request = client
        .post("http://localhost/v1/chat/completions")
        .json(&body)
        .send();
    tokio::pin!(request);
    tokio::select! {
        result = &mut request => { result?; anyhow::bail!("upstream headers were not stalled"); },
        () = fixture.broker.inference_started.notified() => {},
        () = tokio::time::sleep(Duration::from_secs(5)) => anyhow::bail!("inference did not reach broker"),
    }
    assert_eq!(fixture.broker.inference_calls.load(Ordering::SeqCst), 1);
    fixture.harness.stop_project(fixture.project).await?;
    // The broker has not released any headers. A bounded denial therefore proves
    // the relay canceled its pending upstream request after scope revocation.
    let response = tokio::time::timeout(Duration::from_secs(3), &mut request)
        .await
        .context("Stop did not cancel the pending upstream response")??;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    ensure!(
        response.bytes().await?.is_empty(),
        "unexpected upstream response body"
    );
    fixture.broker.release_inference_headers.notify_one();
    Ok(())
}
