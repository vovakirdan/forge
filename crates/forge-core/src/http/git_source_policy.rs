//! Operator read of future-only Task execution settings.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{ProjectId, TaskId, TaskWorkSurface, git::TaskGitSourceSetting};

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
};
use crate::{CoreError, CoreService};

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route(
        "/v1/projects/{project_id}/tasks/{task_id}/git-source-policy",
        get(read),
    )
}

async fn read(
    State(core): State<CoreService>,
    Path((project, task)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let task = TaskId::from(uuid_id("task_id", &task, &request)?);
    let setting = core
        .read_task_git_source_policy(project, task)
        .await
        .map_err(|e| HttpError::from_core(request.clone(), e))?;
    Ok(json_response(StatusCode::OK, setting, &request))
}

impl CoreService {
    pub async fn read_task_git_source_policy(
        &self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<TaskGitSourceSetting, CoreError> {
        self.store
            .load_task(task)
            .await?
            .filter(|stored| {
                stored.task.project_id() == project
                    && matches!(stored.task.work_surface(), TaskWorkSurface::Git(_))
            })
            .ok_or(CoreError::NotFound {
                aggregate: "Git-bound Task",
            })?;
        Ok(self.store.task_git_source_setting(project, task).await?)
    }
}
