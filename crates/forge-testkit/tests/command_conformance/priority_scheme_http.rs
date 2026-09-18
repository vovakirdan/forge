//! Priority reads use named-command-created Projects and leave canonical state untouched.

use anyhow::Result;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_application::CommandContext;
use forge_domain::ProjectId;
use forge_protocol::wire::CommandName;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::fixture::{Backend, BackendKind, Fixture};

async fn get(router: &Router, path: &str) -> Result<(StatusCode, Value)> {
    let response = router
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty())?)
        .await?;
    let status = response.status();
    Ok((
        status,
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?,
    ))
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; local HTTP read contract"]
async fn priority_scheme_http_is_scoped_complete_and_read_only() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let mut second = fixture.clone();
    second.project_id = ProjectId::new();
    second.context = CommandContext::local_human(
        second.project_id,
        fixture.context.actor,
        fixture.context.core_actor,
    );
    second
        .execute(CommandName::CreateProject, json!({"name":"Other project"}))
        .await?;
    second
        .execute(
            CommandName::StopProjectExecution,
            json!({"reason":"Read-only fixture"}),
        )
        .await?;
    let before = fixture.snapshot().await?;
    assert_ne!(
        before.projects[&fixture.project_id].revision(),
        before.projects[&second.project_id].revision()
    );
    let router = forge_core::router(fixture.core(pool));
    for id in [fixture.project_id, second.project_id] {
        let path = format!("/v1/projects/{id}/priority-scheme");
        for _ in 0..2 {
            let (status, body) = get(&router, &path).await?;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                body,
                json!({
                    "project_id":id, "project_revision": before.projects[&id].revision(),
                    "default_level_id":"normal", "levels":[
                        {"id":"high", "display_name":"High", "rank":100, "retired":false},
                        {"id":"low", "display_name":"Low", "rank":10, "retired":false},
                        {"id":"normal", "display_name":"Normal", "rank":50, "retired":false}
                    ]
                })
            );
        }
        for query in ["", "limit=1", "cursor=x", "actor=owner", "retired=false"] {
            let (status, body) = get(&router, &format!("{path}?{query}")).await?;
            assert_eq!(status, StatusCode::BAD_REQUEST, "query {query}");
            assert_eq!(body["error"]["code"], "invalid_request");
        }
    }
    let (status, body) = get(
        &router,
        &format!("/v1/projects/{}/priority-scheme", ProjectId::new()),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
    for id in [
        "not-a-project",
        "01988000-0000-4000-8000-000000000001",
        "01988000-0000-7000-0000-000000000001",
        "01988000000070008000000000000001",
    ] {
        let (status, body) = get(&router, &format!("/v1/projects/{id}/priority-scheme")).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid_request");
    }
    assert_eq!(before.raw["runs"], json!([]));
    assert_eq!(before.raw["queue_entries"], json!([]));
    assert_eq!(before.raw, fixture.snapshot().await?.raw);
    Ok(())
}
