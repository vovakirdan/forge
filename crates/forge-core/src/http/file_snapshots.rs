//! Operator read views omit import roots and internal Supervisor control requests.

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{ProjectId, TaskId, file_snapshot::FileSnapshotManifest};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::ListView,
};
use crate::{CoreError, CoreService};

pub(super) fn routes() -> Router<CoreService> {
    Router::new()
        .route(
            "/v1/projects/{project_id}/tasks/{task_id}/file-snapshots",
            get(snapshots),
        )
        .route(
            "/v1/projects/{project_id}/tasks/{task_id}/file-inputs",
            get(inputs),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    after: Option<String>,
    limit: Option<u32>,
}

async fn snapshots(
    State(core): State<CoreService>,
    Path((project, task)): Path<(String, String)>,
    Query(page): Query<Page>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let task = TaskId::from(uuid_id("task_id", &task, &request)?);
    let limit = page.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(HttpError::invalid_request(request, "limit must be 1..100"));
    }
    let after = page
        .after
        .as_deref()
        .map(|value| uuid_id("after", value, &request))
        .transpose()?;
    core.read_task(project, task)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let mut items = core
        .store
        .list_file_snapshots(project, task, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let has_more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| items.last().map(|value| value.id.to_string()))
        .flatten();
    let items = items
        .into_iter()
        .map(|op| {
            json!({
                "id":op.id,"artifact_id":op.artifact_id,"task_id":op.task_id,"title":op.title,
                "state":op.state,"error_code":op.error_code,
                "manifest":op.manifest.as_ref().map(safe_manifest),"created_at":op.created_at
            })
        })
        .collect();
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}

async fn inputs(
    State(core): State<CoreService>,
    Path((project, task)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let task = TaskId::from(uuid_id("task_id", &task, &request)?);
    core.read_task(project, task)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let mut tx = core
        .store
        .begin()
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let items = tx
        .task_file_inputs(project, task)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    tx.commit()
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    Ok(json_response(
        StatusCode::OK,
        json!({"items":items.into_iter().map(|input| json!({
            "artifact_id":input.artifact_id,"manifest":safe_manifest(&input.manifest)
        })).collect::<Vec<_>>()}),
        &request,
    ))
}

fn safe_manifest(manifest: &FileSnapshotManifest) -> Value {
    json!({
        "schema_version":manifest.schema_version,
        "project_id":manifest.project_id,
        "source_task_id":manifest.source_task_id,
        "files":manifest.files.iter().map(|file| json!({
            "path":file.path,"size_bytes":file.size_bytes,
            "executable":file.executable,"sha256":file.sha256
        })).collect::<Vec<_>>()
    })
}

impl CoreService {
    pub async fn read_file_snapshots(
        &self,
        project: ProjectId,
        task: TaskId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<Value>, CoreError> {
        self.read_task(project, task).await?;
        Ok(self.store.list_file_snapshots(project,task,after,limit).await?.into_iter().map(|op| json!({
            "id":op.id,"artifact_id":op.artifact_id,"task_id":op.task_id,"title":op.title,
            "state":op.state,"error_code":op.error_code,"manifest":op.manifest,"created_at":op.created_at
        })).collect())
    }
}
