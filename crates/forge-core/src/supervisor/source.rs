//! Source delivery evidence is validated against frozen intent, never accepted from Employees.
use forge_domain::{git::GitSourceDescriptor, runtime::RuntimeLaunchSpec};
use forge_protocol::supervisor::v1::RunEventKind;
use serde_json::Value;

use super::validation::ParsedObservation;
use crate::CoreError;

pub(super) fn source_descriptor(
    run_spec: &Value,
    observation: &ParsedObservation,
) -> Result<Option<GitSourceDescriptor>, CoreError> {
    let Some(raw) = observation.details.get("git_source") else {
        return Ok(None);
    };
    let invalid = || CoreError::InvalidTransport {
        field: "git_source",
        reason: "invalid Git source delivery evidence".into(),
    };
    if observation.kind != RunEventKind::Provisioning {
        return Err(invalid());
    }
    let spec: RuntimeLaunchSpec =
        serde_json::from_value(run_spec.clone()).map_err(|_| invalid())?;
    let request = spec.source_request.as_ref().ok_or_else(invalid)?;
    let descriptor: GitSourceDescriptor =
        serde_json::from_value(raw.clone()).map_err(|_| invalid())?;
    descriptor
        .validate_request(request)
        .map_err(|_| invalid())?;
    Ok(Some(descriptor))
}
