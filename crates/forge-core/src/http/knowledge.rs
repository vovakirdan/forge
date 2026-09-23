//! Owner-local canonical knowledge reads. Employee retrieval uses the scoped Gateway.

use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{ProjectId, knowledge::KnowledgePage};
use serde::Deserialize;
use uuid::Uuid;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::ListView,
};
use crate::{CoreError, CoreService};

pub(super) fn routes() -> Router<CoreService> {
    Router::new()
        .route("/v1/projects/{project_id}/knowledge", get(pages))
        .route("/v1/projects/{project_id}/knowledge/{page_id}", get(page))
        .route(
            "/v1/projects/{project_id}/knowledge/{page_id}/history",
            get(history),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    after: Option<String>,
    limit: Option<u32>,
}

fn parse_query(
    query: Result<Query<PageQuery>, QueryRejection>,
    request: &str,
) -> Result<(Option<String>, u32), HttpError> {
    let query = query
        .map_err(|_| {
            HttpError::invalid_request(request.to_owned(), "invalid knowledge page query")
        })?
        .0;
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(HttpError::invalid_request(
            request.to_owned(),
            "limit must be 1..100",
        ));
    }
    Ok((query.after, limit))
}

pub(super) fn page_view(page: KnowledgePage) -> Result<serde_json::Value, CoreError> {
    let created_at = super::views::timestamp(page.created_at)?;
    let revised_at = super::views::timestamp(page.revised_at)?;
    let mut value =
        serde_json::to_value(page).map_err(|source| forge_storage::StorageError::Snapshot {
            aggregate: "knowledge_page",
            source,
        })?;
    value["created_at"] = created_at.into();
    value["revised_at"] = revised_at.into();
    Ok(value)
}

async fn pages(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let (after, limit) = parse_query(query, &request)?;
    let after = after
        .as_deref()
        .map(|value| uuid_id("after", value, &request))
        .transpose()?;
    let mut pages = core
        .read_knowledge_pages(project, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let more = pages.len() > limit as usize;
    pages.truncate(limit as usize);
    let next_cursor = more
        .then(|| pages.last().map(|page| page.id.to_string()))
        .flatten();
    let items = pages
        .into_iter()
        .map(page_view)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}

async fn page(
    State(core): State<CoreService>,
    Path((project, id)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let id = uuid_id("page_id", &id, &request)?;
    let page = core
        .read_knowledge_page(project, id)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        page_view(page).map_err(|error| HttpError::from_core(request.clone(), error))?,
        &request,
    ))
}

async fn history(
    State(core): State<CoreService>,
    Path((project, id)): Path<(String, String)>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let id = uuid_id("page_id", &id, &request)?;
    let (after, limit) = parse_query(query, &request)?;
    let after = after
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| {
            HttpError::invalid_request(request.clone(), "after must be a revision number")
        })?
        .unwrap_or(0);
    if after > i64::MAX as u64 {
        return Err(HttpError::invalid_request(
            request,
            "after is outside the revision range",
        ));
    }
    let mut pages = core
        .read_knowledge_page_history(project, id, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let more = pages.len() > limit as usize;
    pages.truncate(limit as usize);
    let next_cursor = more
        .then(|| pages.last().map(|page| page.revision.to_string()))
        .flatten();
    let items = pages
        .into_iter()
        .map(page_view)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}

impl CoreService {
    pub async fn read_knowledge_pages(
        &self,
        project: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<KnowledgePage>, CoreError> {
        self.read_project(project).await?;
        Ok(self
            .store
            .knowledge_pages_page(project, after, limit)
            .await?)
    }
    pub async fn read_knowledge_page(
        &self,
        project: ProjectId,
        id: Uuid,
    ) -> Result<KnowledgePage, CoreError> {
        self.read_project(project).await?;
        self.store
            .load_knowledge_page(project, id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "knowledge_page",
            })
    }
    pub async fn read_knowledge_page_history(
        &self,
        project: ProjectId,
        id: Uuid,
        after_revision: u64,
        limit: u32,
    ) -> Result<Vec<KnowledgePage>, CoreError> {
        self.read_knowledge_page(project, id).await?;
        Ok(self
            .store
            .knowledge_page_revisions_page(project, id, after_revision, limit)
            .await?)
    }
}
