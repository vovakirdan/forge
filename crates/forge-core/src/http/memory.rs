//! Owner-only local operator reads. Employee tools must derive scope from Gateway auth.
use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{EmployeeId, ProjectId};
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
        .route("/v1/projects/{project_id}/memory", get(list))
        .route("/v1/projects/{project_id}/memory/search", get(search))
        .route("/v1/projects/{project_id}/memory/status", get(status))
        .route("/v1/projects/{project_id}/memory/{id}", get(entry))
        .route(
            "/v1/projects/{project_id}/memory/{id}/history",
            get(history),
        )
        .route(
            "/v1/projects/{project_id}/employees/{employee_id}/onboarding",
            get(onboarding),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchQuery {
    query: String,
    employee_id: Option<String>,
    limit: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    after: Option<String>,
    employee_id: Option<String>,
    limit: Option<u32>,
}

fn limit(value: Option<u32>, request: &str) -> Result<u32, HttpError> {
    let value = value.unwrap_or(50);
    if !(1..=100).contains(&value) {
        return Err(HttpError::invalid_request(
            request.to_owned(),
            "limit must be 1..100",
        ));
    }
    Ok(value)
}
fn employee(value: Option<String>, request: &str) -> Result<Option<EmployeeId>, HttpError> {
    value
        .map(|value| uuid_id("employee_id", &value, request).map(EmployeeId::from))
        .transpose()
}
async fn search(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<SearchQuery>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let query = query
        .map_err(|_| HttpError::invalid_request(request.clone(), "invalid memory search query"))?
        .0;
    let result = core
        .query_search(
            project,
            employee(query.employee_id, &request)?,
            &query.query,
            limit(query.limit, &request)?,
        )
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, result, &request))
}
async fn list(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let query = query
        .map_err(|_| HttpError::invalid_request(request.clone(), "invalid memory page query"))?
        .0;
    let employee = employee(query.employee_id, &request)?;
    let limit = limit(query.limit, &request)?;
    let after = query
        .after
        .map(|value| uuid_id("after", &value, &request))
        .transpose()?;
    let result = core
        .read_memory_entries(project, employee, after, limit)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, result, &request))
}
async fn entry(
    State(core): State<CoreService>,
    Path((project, id)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let id = uuid_id("memory_id", &id, &request)?;
    let result = core
        .read_memory_entry(project, id)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, result, &request))
}
async fn history(
    State(core): State<CoreService>,
    Path((project, id)): Path<(String, String)>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let id = uuid_id("memory_id", &id, &request)?;
    let query = query
        .map_err(|_| HttpError::invalid_request(request.clone(), "invalid memory history query"))?
        .0;
    if query.employee_id.is_some() {
        return Err(HttpError::invalid_request(
            request,
            "history is project-scoped operator audit",
        ));
    }
    let limit = limit(query.limit, &request)?;
    let after = query
        .after
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| {
            HttpError::invalid_request(request.clone(), "after must be a revision number")
        })?
        .unwrap_or(0);
    if after > i64::MAX as u64 {
        return Err(HttpError::invalid_request(
            request,
            "revision outside range",
        ));
    }
    let result = core
        .read_memory_history(project, id, after, limit)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, result, &request))
}
async fn status(
    State(core): State<CoreService>,
    Path(project): Path<String>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let result = core
        .read_memory_status(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, result, &request))
}
async fn onboarding(
    State(core): State<CoreService>,
    Path((project, id)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let employee = EmployeeId::from(uuid_id("employee_id", &id, &request)?);
    let result = core
        .read_employee_onboarding(project, employee)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, result, &request))
}

impl CoreService {
    pub async fn read_memory_status(&self, project: ProjectId) -> Result<Value, CoreError> {
        self.read_project(project).await?;
        let mut status = self.store.knowledge_projection_status(project).await?;
        status["configured"] = json!(self.agentmemory.is_some());
        Ok(status)
    }
    pub async fn read_employee_onboarding(
        &self,
        project: ProjectId,
        employee: EmployeeId,
    ) -> Result<Value, CoreError> {
        self.read_project(project).await?;
        self.store
            .knowledge_onboarding_status(project, employee)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "employee_onboarding",
            })
    }
    pub async fn read_memory_entry(
        &self,
        project: ProjectId,
        id: Uuid,
    ) -> Result<forge_domain::knowledge::DerivedMemoryEntry, CoreError> {
        self.read_project(project).await?;
        self.store
            .load_derived_memory_entry(project, id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "memory_entry",
            })
    }
    pub(crate) async fn read_memory_history(
        &self,
        project: ProjectId,
        id: Uuid,
        after: u64,
        limit: u32,
    ) -> Result<ListView<forge_domain::knowledge::DerivedMemoryEntry>, CoreError> {
        self.read_memory_entry(project, id).await?;
        let items = self
            .store
            .derived_memory_revisions_page(project, id, after, limit)
            .await?;
        let next_cursor = (items.len() == limit as usize)
            .then(|| items.last().map(|entry| entry.revision.to_string()))
            .flatten();
        Ok(ListView { items, next_cursor })
    }
    pub(crate) async fn read_memory_entries(
        &self,
        project: ProjectId,
        employee: Option<EmployeeId>,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<ListView<forge_domain::knowledge::DerivedMemoryEntry>, CoreError> {
        self.read_project(project).await?;
        if let Some(employee) = employee
            && self
                .store
                .load_employee(employee)
                .await?
                .is_none_or(|value| value.employee.project_id() != project)
        {
            return Err(CoreError::Forbidden);
        }
        let mut tx = self.store.begin().await?;
        tx.lock_project(project).await?.ok_or(CoreError::NotFound {
            aggregate: "project",
        })?;
        let candidates = self
            .store
            .visible_derived_memory(project, employee, after, limit)
            .await?;
        // Cursor advances over scanned candidates, even when every source was withdrawn.
        let next_cursor = (candidates.len() == limit as usize)
            .then(|| candidates.last().map(|entry| entry.id.to_string()))
            .flatten();
        let mut items = Vec::new();
        for entry in candidates {
            match tx
                .validate_knowledge_sources(project, entry.subject.scope(), &entry.source_refs)
                .await
            {
                Ok(()) => items.push(entry),
                Err(forge_storage::StorageError::NotFound {
                    aggregate: "knowledge_source",
                }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        tx.commit().await?;
        Ok(ListView { items, next_cursor })
    }
}
