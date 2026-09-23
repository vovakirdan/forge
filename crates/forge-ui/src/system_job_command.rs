//! Named SystemJob commands use the domain's typed, bounded mutation contract.

use forge_domain::{ProjectId, system_job::SystemJobManagement};
use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{
    command::{CommandTarget, uuid_v7},
    http::ApiError,
};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    project_id: String,
    expected_revision: u64,
    payload: serde_json::Value,
}

pub(crate) struct SystemJobCommand {
    expected_revision: u64,
    resource_kind: Option<&'static str>,
    expected_resource_id: Option<String>,
}

impl SystemJobCommand {
    pub(crate) fn parse(bytes: &[u8], target: CommandTarget) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id)
            || !(1..MAX_SAFE_INTEGER).contains(&envelope.expected_revision)
        {
            return Err(ApiError::BadRequest);
        }
        let (operation, resource_kind) = match target {
            CommandTarget::ConfigureSystemJobs => ("configure", None),
            CommandTarget::RequestTaskSummary => ("request_summary", Some("system_job")),
            CommandTarget::RequestEmployeeOnboarding => ("request_onboarding", Some("system_job")),
            CommandTarget::RetrySystemJob => ("retry", Some("system_job")),
            CommandTarget::SkipEmployeeOnboarding => ("skip_onboarding", Some("employee")),
            _ => return Err(ApiError::BadRequest),
        };
        let mut payload = envelope
            .payload
            .as_object()
            .cloned()
            .ok_or(ApiError::BadRequest)?;
        if payload.contains_key("operation") {
            return Err(ApiError::BadRequest);
        }
        payload.insert(
            "operation".into(),
            serde_json::Value::String(operation.into()),
        );
        let command: SystemJobManagement =
            serde_json::from_value(serde_json::Value::Object(payload))
                .map_err(|_| ApiError::BadRequest)?;
        let project = ProjectId::from(
            uuid::Uuid::parse_str(&envelope.project_id).map_err(|_| ApiError::BadRequest)?,
        );
        command
            .validate_project(project)
            .map_err(|_| ApiError::BadRequest)?;
        let expected_resource_id = match command {
            SystemJobManagement::Retry { job_id } => Some(job_id.to_string()),
            SystemJobManagement::SkipOnboarding { employee_id, .. } => {
                Some(employee_id.to_string())
            }
            _ => None,
        };
        Ok(Self {
            expected_revision: envelope.expected_revision,
            resource_kind,
            expected_resource_id,
        })
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        if !uuid_v7(&receipt.command_id)
            || receipt.project_revision != self.expected_revision + 1
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || match (self.resource_kind, receipt.resource.as_ref()) {
                (None, None) => false,
                (Some(kind), Some(resource)) => {
                    resource.kind != kind
                        || !uuid_v7(&resource.id)
                        || self
                            .expected_resource_id
                            .as_ref()
                            .is_some_and(|id| !id.eq_ignore_ascii_case(&resource.id))
                }
                _ => true,
            }
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
