//! Owner-local SystemJob status contains policy and receipts, never credential bindings.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::ProjectId;
use serde_json::Value;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id},
};
use crate::{CoreError, CoreService};

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route("/v1/projects/{project_id}/system-jobs", get(status))
}

async fn status(
    State(core): State<CoreService>,
    Path(project): Path<String>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let status = core
        .read_system_job_status(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, status, &request))
}

impl CoreService {
    pub async fn read_system_job_status(&self, project: ProjectId) -> Result<Value, CoreError> {
        self.read_project(project).await?;
        Ok(self.store.system_job_status(project).await?)
    }
}
