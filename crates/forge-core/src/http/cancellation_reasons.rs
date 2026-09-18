//! Project-owned reason labels, including retired entries for historical display.

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id},
};
use crate::CoreService;
use axum::{
    Router,
    extract::{Path, RawQuery, State},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::Project;
use serde::Serialize;

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route("/v1/projects/{project_id}/cancellation-reasons", get(read))
}

async fn read(
    State(core): State<CoreService>,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, HttpError> {
    let request = request_id();
    let id = project_id(&path, &request)?;
    if path.len() != 36 || id.as_uuid().get_variant() != uuid::Variant::RFC4122 || query.is_some() {
        return Err(HttpError::invalid_request(
            request,
            "cancellation reasons require a UUIDv7 and no query parameters",
        ));
    }
    let project = core
        .read_project(id)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, view(&project), &request))
}

#[derive(Serialize)]
struct CatalogView<'a> {
    project_id: forge_domain::ProjectId,
    project_revision: u64,
    reasons: Vec<ReasonView<'a>>,
}

#[derive(Serialize)]
struct ReasonView<'a> {
    id: &'a str,
    display_name: &'a str,
    retired: bool,
}

fn view(project: &Project) -> CatalogView<'_> {
    CatalogView {
        project_id: project.id(),
        project_revision: project.revision(),
        reasons: project
            .cancellation_reasons()
            .reasons()
            .map(|reason| ReasonView {
                id: reason.id().as_str(),
                display_name: reason.display_name(),
                retired: reason.is_retired(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Project, ProjectId, Timestamp};
    use serde_json::json;
    #[test]
    fn catalog_is_scoped_to_one_project_snapshot_and_excludes_private_fields() {
        let project = Project::new(ProjectId::new(), "Private name", Timestamp::now_utc()).unwrap();
        assert_eq!(
            serde_json::to_value(super::view(&project)).unwrap(),
            json!({
                "project_id":project.id(), "project_revision":project.revision(),
                "reasons":[{"id":"unspecified","display_name":"Unspecified","retired":false}]
            })
        );
    }
}
