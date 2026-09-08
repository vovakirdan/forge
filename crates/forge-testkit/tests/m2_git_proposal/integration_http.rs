use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

pub(super) async fn assert_history(
    harness: &M0Harness,
    project: forge_domain::ProjectId,
    task: forge_domain::TaskId,
) -> Result<()> {
    let router = forge_core::router(harness.core.clone());
    let prefix = format!("/v1/projects/{project}/tasks/{task}/integrations");
    let get = |path: String| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(Request::builder().uri(path).body(Body::empty())?)
                .await?;
            let status = response.status();
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
            Ok::<_, anyhow::Error>((status, value))
        }
    };
    let (status, page) = get(format!("{prefix}?limit=1")).await?;
    assert_eq!(status, StatusCode::OK);
    let items = page["items"].as_array().context("items")?;
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item["state"], "completed");
    assert_eq!(item["result_code"], "applied");
    assert!(
        item["created_at"]
            .as_str()
            .context("timestamp")?
            .contains('T')
    );
    assert!(item.get("command").is_none());
    assert!(!item.to_string().contains("mock-source"));
    let id = item["id"].as_str().context("operation id")?;
    assert!(
        get(format!("{prefix}?after={id}")).await?.1["items"]
            .as_array()
            .context("tail")?
            .is_empty()
    );
    for suffix in ["limit=0", "limit=101", "after=invalid", "extra=1"] {
        assert_eq!(
            get(format!("{prefix}?{suffix}")).await?.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        get(format!(
            "/v1/projects/{}/tasks/{task}/integrations",
            Uuid::now_v7()
        ))
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}
