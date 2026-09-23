//! Safe Employee runtime metadata without prompts, credentials, or host paths.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{EmployeeId, Timestamp, runtime::SurfaceSpec};
use serde_json::json;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::timestamp,
};
use crate::CoreService;

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route(
        "/v1/projects/{project_id}/employees/{employee_id}/runtime-metadata",
        get(runtime_metadata),
    )
}

async fn runtime_metadata(
    State(core): State<CoreService>,
    Path((project, employee)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let employee = EmployeeId::from(uuid_id("employee_id", &employee, &request)?);
    core.read_employee(project, employee)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let record = core
        .store()
        .employee_runtime_binding_record(project, employee)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let body = if let Some(record) = record {
        let binding = record.binding;
        let profile = &binding.execution_profile;
        let image_digest = binding
            .image
            .rsplit_once("@sha256:")
            .map(|(_, digest)| digest);
        let surface_kind = match binding.surface {
            SurfaceSpec::None => "none",
            SurfaceSpec::FilesystemSandbox => "filesystem_sandbox",
            SurfaceSpec::GitWorktree { .. } => "git_worktree",
            SurfaceSpec::GitUnborn { .. } => "git_unborn",
            SurfaceSpec::GitUnbornCandidateSnapshot { .. } => "git_unborn_candidate_snapshot",
            SurfaceSpec::GitCandidateSnapshot { .. } => "git_candidate_snapshot",
        };
        json!({
            "availability":"available","employee_id":employee,
            "binding_revision":record.revision,
            "updated_at":timestamp(Timestamp::from_offset_date_time(record.updated_at))
                .map_err(|error|HttpError::from_core(request.clone(),error))?,
            "profile_id":profile.id(),"profile_revision":profile.revision(),
            "adapter_id":profile.adapter_id(),"adapter_version":profile.adapter_version(),
            "provider_id":profile.provider_id(),"model":profile.model(),
            "credential_binding_id":profile.credential_binding().id,
            "credential_delivery":profile.credential_delivery(),
            "image_digest":image_digest,"surface_kind":surface_kind,
            "access":binding.access,"limits":binding.limits,"budget":binding.budget,
            "prompts_available":true
        })
    } else {
        json!({"availability":"unavailable","employee_id":employee})
    };
    Ok(json_response(StatusCode::OK, body, &request))
}
