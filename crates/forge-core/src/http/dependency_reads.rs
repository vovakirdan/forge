//! Read-only Task dependency pages; condition evaluation stays in the domain.

use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{LifecycleStatus, TaskId};
use forge_storage::DependencyDirection;
use serde::{Deserialize, Serialize};

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::ListView,
};
use crate::CoreService;

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route(
        "/v1/projects/{project_id}/tasks/{task_id}/dependencies/{direction}",
        get(dependencies),
    )
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Serialize)]
struct DependencyView {
    blocker_task_id: TaskId,
    blocked_task_id: TaskId,
    required_condition: &'static str,
    related_task: RelatedTaskView,
    condition_state: ConditionState,
}

#[derive(Serialize)]
struct RelatedTaskView {
    id: TaskId,
    key: String,
    title: String,
    lifecycle: LifecycleStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ConditionState {
    Pending,
    Satisfied,
    BlockerCancelled,
}

fn dependency_view(row: forge_storage::DependencyReadRow) -> DependencyView {
    let condition_state = if row.dependency.is_satisfied_by(row.blocker_lifecycle) {
        ConditionState::Satisfied
    } else if row.blocker_lifecycle == LifecycleStatus::Cancelled {
        ConditionState::BlockerCancelled
    } else {
        ConditionState::Pending
    };
    DependencyView {
        blocker_task_id: row.dependency.blocker_task_id(),
        blocked_task_id: row.dependency.blocked_task_id(),
        required_condition: "task_done",
        related_task: RelatedTaskView {
            id: row.related_task.id(),
            key: row.related_task.key().to_string(),
            title: row.related_task.spec().title().to_owned(),
            lifecycle: row.related_task.lifecycle(),
        },
        condition_state,
    }
}

async fn dependencies(
    State(core): State<CoreService>,
    Path((project, task, direction)): Path<(String, String, String)>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let task = TaskId::from(uuid_id("task_id", &task, &request)?);
    let parsed_direction = match direction.as_str() {
        "blocked_by" => DependencyDirection::BlockedBy,
        "blocks" => DependencyDirection::Blocks,
        _ => {
            return Err(HttpError::invalid_request(
                request,
                "unknown dependency direction",
            ));
        }
    };
    let Query(page) = query.map_err(|_| {
        HttpError::invalid_request(request.clone(), "invalid dependency pagination")
    })?;
    let limit = page.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(HttpError::invalid_request(request, "limit must be 1..100"));
    }
    let scope = format!("d1:{project}:{task}:{direction}:");
    let after = page
        .cursor
        .as_deref()
        .map(|cursor| {
            let value = cursor.strip_prefix(&scope).ok_or_else(|| {
                HttpError::cursor_invalid(request.clone(), "dependency cursor scope mismatch")
            })?;
            uuid_id("cursor", value, &request)
                .map(TaskId::from)
                .map_err(|_| {
                    HttpError::cursor_invalid(request.clone(), "invalid dependency cursor")
                })
        })
        .transpose()?;
    let mut page = core
        .read_task_dependencies(project, task, parsed_direction, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    if !page.cursor_exists {
        return Err(HttpError::cursor_invalid(
            request,
            "dependency cursor is no longer present",
        ));
    }
    let more = page.rows.len() > limit as usize;
    page.rows.truncate(limit as usize);
    let next_cursor = more
        .then(|| {
            page.rows
                .last()
                .map(|row| format!("{scope}{}", row.related_task.id()))
        })
        .flatten();
    let items = page.rows.into_iter().map(dependency_view).collect();
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}

impl CoreService {
    async fn read_task_dependencies(
        &self,
        project: forge_domain::ProjectId,
        task: TaskId,
        direction: DependencyDirection,
        after: Option<TaskId>,
        limit: u32,
    ) -> Result<forge_storage::DependencyReadPage, crate::CoreError> {
        Ok(self
            .store()
            .read_task_dependencies(project, task, direction, after, limit)
            .await?)
    }
}
