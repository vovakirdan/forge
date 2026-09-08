//! Local control reads preserve Project scope and bounded cursors.
use super::{
    employees,
    fixture::{Backend, BackendKind, Fixture},
};
use anyhow::{Context, Result};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_protocol::wire::CommandName;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn get(router: &Router, path: &str) -> Result<(StatusCode, Value)> {
    let response = router
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty())?)
        .await?;
    let status = response.status();
    Ok((
        status,
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?,
    ))
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; local HTTP read contract"]
async fn inbox_http_reads_are_scoped_bounded_and_do_not_acknowledge() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let employee = employees::create(&fixture, "Inbox HTTP Bob").await?;
    let mut threads = vec![];
    for _ in 0..2 {
        threads.push(
            fixture
                .execute(
                    CommandName::OpenEmployeeThread,
                    json!({"employee_id":employee}),
                )
                .await?
                .resource
                .context("thread")?
                .id,
        );
    }
    for revision in 1..=2 {
        fixture
            .execute(
                CommandName::SendEmployeeMessage,
                json!({"thread_id":threads[0],
            "expected_thread_revision":revision,"target":{"kind":"inbox"},"kind":"question",
            "requirement":"answered","body":"An ordinary Employee question"}),
            )
            .await?;
    }
    let router = forge_core::router(fixture.core(pool));
    let prefix = format!("/v1/projects/{}", fixture.project_id);
    let thread_path = format!("{prefix}/employees/{employee}/threads");
    let (status, first) = get(&router, &format!("{thread_path}?limit=1")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["items"].as_array().context("items")?.len(), 1);
    assert_eq!(first["next_cursor"], threads[0]);
    let (_, second) = get(
        &router,
        &format!("{thread_path}?after={}&limit=1", threads[0]),
    )
    .await?;
    assert_eq!(second["items"][0]["id"], threads[1]);
    assert!(second["next_cursor"].is_null());
    let message_path = format!("{prefix}/threads/{}/messages", threads[0]);
    let (_, messages) = get(&router, &format!("{message_path}?limit=1")).await?;
    assert_eq!(messages["next_cursor"], "1");
    let (_, tail) = get(&router, &format!("{message_path}?after=1&limit=1")).await?;
    assert_eq!(tail["items"][0]["sequence"], 2);
    assert!(tail["next_cursor"].is_null());
    for suffix in [
        "limit=0",
        "limit=101",
        "after=-1",
        "after=18446744073709551615",
        "extra=1",
    ] {
        assert_eq!(
            get(&router, &format!("{message_path}?{suffix}")).await?.0,
            StatusCode::BAD_REQUEST
        );
    }
    let foreign = format!(
        "/v1/projects/{}/threads/{}/messages",
        Uuid::now_v7(),
        threads[0]
    );
    assert_eq!(get(&router, &foreign).await?.0, StatusCode::NOT_FOUND);
    let foreign = format!(
        "/v1/projects/{}/employees/{employee}/threads",
        Uuid::now_v7()
    );
    assert_eq!(get(&router, &foreign).await?.0, StatusCode::NOT_FOUND);
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM employee_message_receipts")
        .fetch_one(pool)
        .await?;
    assert_eq!(receipts, 0);
    Ok(())
}
