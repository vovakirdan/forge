//! Safe Run context coordinates and technical evidence receipts.

use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{EvidenceLocation, EvidenceObject};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::{ListView, timestamp},
};
use crate::{CoreError, CoreService};

pub(super) fn routes() -> Router<CoreService> {
    Router::new()
        .route(
            "/v1/projects/{project_id}/runs/{run_id}/context",
            get(context),
        )
        .route(
            "/v1/projects/{project_id}/runs/{run_id}/evidence",
            get(evidence),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextCoordinates {
    context_snapshot_id: Uuid,
    run_id: Uuid,
    project_id: forge_domain::ProjectId,
    task_id: forge_domain::TaskId,
    employee_id: forge_domain::EmployeeId,
    pipeline_version_id: forge_domain::PipelineVersionId,
    stage_id: forge_domain::StageId,
    stage_visit: Option<u64>,
    task_revision_before_dispatch: u64,
    run_spec_id: Uuid,
}

async fn context(
    State(core): State<CoreService>,
    Path((project, run)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let run = uuid_id("run_id", &run, &request)?;
    core.read_run(project, run)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let coordinates = core
        .store()
        .run_context_coordinates(project, run)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let body = if let Some(value) = coordinates {
        let typed: ContextCoordinates = serde_json::from_value(value).map_err(|_| {
            HttpError::from_core(
                request.clone(),
                CoreError::InvalidTransport {
                    field: "context",
                    reason: "invalid context coordinates".into(),
                },
            )
        })?;
        if typed.run_id != run
            || typed.project_id != project
            || typed.context_snapshot_id.get_version_num() != 7
            || typed.run_spec_id.get_version_num() != 7
            || typed.task_revision_before_dispatch == 0
        {
            return Err(HttpError::from_core(
                request,
                CoreError::InvalidTransport {
                    field: "context",
                    reason: "context scope or revision mismatch".into(),
                },
            ));
        }
        json!({"availability":"available","coordinates":typed})
    } else {
        json!({"availability":"unavailable","run_id":run,"coordinates":null})
    };
    Ok(json_response(StatusCode::OK, body, &request))
}

async fn evidence(
    State(core): State<CoreService>,
    Path((project, run)): Path<(String, String)>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let run = uuid_id("run_id", &run, &request)?;
    let query = query
        .map_err(|error| HttpError::invalid_request(request.clone(), error.to_string()))?
        .0;
    let limit = query.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(HttpError::invalid_request(
            request,
            "limit must be between 1 and 50",
        ));
    }
    let after = query
        .cursor
        .as_deref()
        .map(|value| uuid_id("cursor", value, &request))
        .transpose()?;
    core.read_run(project, run)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let mut rows = core
        .store()
        .run_evidence_page(project, run, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = more
        .then(|| rows.last().map(|(value, _)| value.data().id.to_string()))
        .flatten();
    let items = rows
        .into_iter()
        .map(|(value, stored)| safe_evidence_receipt(&value, stored))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}

pub(super) fn safe_evidence_receipt(
    receipt: &EvidenceObject,
    stored: bool,
) -> Result<Value, CoreError> {
    let data = receipt.data();
    let storage_state = match data.location {
        EvidenceLocation::PendingUpload => "pending_upload",
        EvidenceLocation::Stored { .. } if stored => "stored",
        EvidenceLocation::Stored { .. } => "unconfirmed",
    };
    Ok(json!({
        "id":data.id,"run_id":data.scope.run_id,"task_id":data.scope.task_id,
        "stream":data.stream,"sequence":data.sequence,"sha256":data.sha256,
        "size_bytes":data.size_bytes,"redaction_policy_reference":data.redaction_policy_reference,
        "storage_state":storage_state,"created_at":timestamp(data.created_at)?,
        "content_availability":"unavailable"
    }))
}

pub(super) fn safe_diagnostics(raw: Value) -> Result<Value, CoreError> {
    let evidence = raw["evidence"]
        .as_array()
        .ok_or(CoreError::InvalidTransport {
            field: "diagnostics.evidence",
            reason: "expected array".into(),
        })?;
    let evidence = evidence
        .iter()
        .map(|value| {
            let receipt: EvidenceObject =
                serde_json::from_value(value.clone()).map_err(|_| CoreError::InvalidTransport {
                    field: "diagnostics.evidence",
                    reason: "invalid receipt".into(),
                })?;
            safe_evidence_receipt(
                &receipt,
                matches!(receipt.data().location, EvidenceLocation::Stored { .. }),
            )
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let incidents = raw["incidents"]
        .as_array()
        .ok_or(CoreError::InvalidTransport {
            field: "diagnostics.incidents",
            reason: "expected array".into(),
        })?;
    let incidents=incidents.iter().map(|item| json!({"id":item["id"],"assessment":item["assessment"],"created_at":item["created_at"]})).collect::<Vec<_>>();
    let streams = raw["streams"]
        .as_array()
        .ok_or(CoreError::InvalidTransport {
            field: "diagnostics.streams",
            reason: "expected array".into(),
        })?;
    let streams = streams
        .iter()
        .map(|item| json!({"stream":item["stream"],"incomplete":item["incomplete"]}))
        .collect::<Vec<_>>();
    let marker = |name| (!raw[name].is_null()).then(|| json!({"available":true}));
    let safe = json!({
        "runtime_report":marker("runtime_report"),"handoff":marker("handoff"),
        "incidents":incidents,"evidence":evidence,"streams":streams,
        "proxy_usage":marker("proxy_usage"),"git_source":marker("git_source")
    });
    if serde_json::to_vec(&safe)
        .map_err(|_| CoreError::InvalidTransport {
            field: "diagnostics",
            reason: "cannot encode".into(),
        })?
        .len()
        > 1024 * 1024
    {
        return Err(CoreError::InvalidTransport {
            field: "diagnostics",
            reason: "safe view exceeds one MiB".into(),
        });
    }
    Ok(safe)
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        EvidenceLocation, EvidenceObject, EvidenceObjectInput, EvidenceScope, EvidenceStream,
        ProjectId, TaskId, Timestamp,
    };
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn opaque_diagnostics_are_replaced_with_availability_and_safe_receipts() {
        let receipt = EvidenceObject::new(EvidenceObjectInput {
            id: Uuid::now_v7(),
            scope: EvidenceScope {
                project_id: ProjectId::new(),
                task_id: Some(TaskId::new()),
                run_id: Uuid::now_v7(),
            },
            stream: EvidenceStream::Stdout,
            sequence: 1,
            sha256: "a".repeat(64),
            size_bytes: 32,
            redaction_policy_reference: "redaction/v1".into(),
            location: EvidenceLocation::PendingUpload,
            created_at: Timestamp::now_utc(),
        })
        .unwrap()
        .stored()
        .unwrap();
        let raw = json!({
            "runtime_report":{"prompt":"private-prompt-canary"},
            "handoff":{"auth":"private-auth-canary"},
            "incidents":[{"id":Uuid::now_v7(),"kind":"private-path-canary","assessment":"unknown","created_at":"2026-01-01T00:00:00Z"}],
            "evidence":[receipt],"streams":[{"stream":"stdout","incomplete":true,"raw":"private-stream-canary"}],
            "proxy_usage":{"token":"private-proxy-canary"},
            "git_source":{"path":"private-git-canary"},
        });
        let safe = super::safe_diagnostics(raw).unwrap();
        assert_eq!(safe["runtime_report"], json!({"available":true}));
        assert_eq!(safe["handoff"], json!({"available":true}));
        assert_eq!(safe["proxy_usage"], json!({"available":true}));
        assert_eq!(safe["git_source"], json!({"available":true}));
        assert_eq!(
            safe["streams"][0],
            json!({"stream":"stdout","incomplete":true})
        );
        let serialized = serde_json::to_string(&safe).unwrap();
        for forbidden in ["private-", "object_key", "prompt", "auth"] {
            assert!(!serialized.contains(forbidden));
        }
    }
}
