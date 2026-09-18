//! Project priority labels are a separate read, not part of the control projection.

use axum::{
    Router,
    extract::{Path, RawQuery, State},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::Project;
use serde::Serialize;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id},
};
use crate::CoreService;

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route("/v1/projects/{project_id}/priority-scheme", get(read))
}

async fn read(
    State(core): State<CoreService>,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, HttpError> {
    let request = request_id();
    let id = project_id(&path, &request)?;
    if path.len() != 36 || id.as_uuid().get_variant() != uuid::Variant::RFC4122 {
        return Err(HttpError::invalid_request(
            request,
            "project_id must be a UUIDv7",
        ));
    }
    if query.is_some() {
        return Err(HttpError::invalid_request(
            request,
            "priority scheme does not accept query parameters",
        ));
    }
    let project = core
        .read_project(id)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, view(&project), &request))
}

#[derive(Serialize)]
struct PrioritySchemeView<'a> {
    project_id: forge_domain::ProjectId,
    project_revision: u64,
    default_level_id: &'a str,
    levels: Vec<PriorityLevelView<'a>>,
}

#[derive(Serialize)]
struct PriorityLevelView<'a> {
    id: &'a str,
    display_name: &'a str,
    rank: i32,
    retired: bool,
}

fn view(project: &Project) -> PrioritySchemeView<'_> {
    let scheme = project.priority_scheme();
    PrioritySchemeView {
        project_id: project.id(),
        project_revision: project.revision(),
        default_level_id: scheme.default_level_id().as_str(),
        levels: scheme
            .levels()
            .map(|level| PriorityLevelView {
                id: level.id().as_str(),
                display_name: level.display_name(),
                rank: level.rank(),
                retired: level.is_retired(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Project, ProjectId, Timestamp};
    use serde_json::json;

    #[test]
    fn view_contains_only_priority_fields_from_one_project_snapshot() {
        let mut project = Project::new(
            ProjectId::new(),
            "private-project-name",
            Timestamp::now_utc(),
        )
        .unwrap();
        project.record_child_mutation(project.updated_at()).unwrap();
        assert_eq!(
            serde_json::to_value(super::view(&project)).unwrap(),
            json!({
                "project_id": project.id(), "project_revision": project.revision(),
                "default_level_id":"normal", "levels":[
                    {"id":"high", "display_name":"High", "rank":100, "retired":false},
                    {"id":"low", "display_name":"Low", "rank":10, "retired":false},
                    {"id":"normal", "display_name":"Normal", "rank":50, "retired":false}
                ]
            })
        );
    }
}
