//! Bounded owner reads of retained management intent and resolution receipts.

use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{
    ExecutorKind, ProjectId, TaskResumeSchedule, Timestamp,
    resolution::{EscalationSource, ResolutionAssignment, ResolverRoute},
    runtime::BootRecoveryPolicy,
};
use forge_storage::{
    EscalationReadRow, RecoveryAssessmentRead, RecoveryRunRead, RecoverySettingsRead,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::{ListView, timestamp},
};
use crate::{CoreError, CoreService};

const DEFAULT_PAGE_LIMIT: u32 = 20;

fn serialize_timestamp<S: serde::Serializer>(
    value: &forge_domain::Timestamp,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    timestamp(*value)
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}

fn serialize_optional_timestamp<S: serde::Serializer>(
    value: &Option<forge_domain::Timestamp>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value
        .map(timestamp)
        .transpose()
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}

pub(super) fn routes() -> Router<CoreService> {
    Router::new()
        .route(
            "/v1/projects/{project_id}/resume-schedules",
            get(resume_schedules),
        )
        .route("/v1/projects/{project_id}/escalations", get(escalations))
        .route(
            "/v1/projects/{project_id}/escalations/{escalation_id}",
            get(escalation_detail),
        )
        .route(
            "/v1/projects/{project_id}/resolver-routes",
            get(resolver_routes),
        )
        .route(
            "/v1/projects/{project_id}/next-run-constraints",
            get(next_run_constraints),
        )
        .route("/v1/projects/{project_id}/recovery", get(recovery_settings))
        .route(
            "/v1/projects/{project_id}/recovery-runs",
            get(recovery_runs),
        )
        .route(
            "/v1/projects/{project_id}/recovery-runs/{run_id}/assessment-readiness",
            get(recovery_assessment_readiness),
        )
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    cursor: Option<String>,
    limit: Option<u32>,
}

fn page(
    query: Result<Query<Page>, QueryRejection>,
    request_id: &str,
    maximum: u32,
) -> Result<(Option<Uuid>, u32), HttpError> {
    let query = query
        .map_err(|error| HttpError::invalid_request(request_id.to_owned(), error.to_string()))?
        .0;
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=maximum).contains(&limit) {
        return Err(HttpError::invalid_request(
            request_id.to_owned(),
            format!("limit must be between 1 and {maximum}"),
        ));
    }
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| uuid_id("cursor", value, request_id))
        .transpose()?;
    Ok((cursor, limit))
}

async fn resume_schedules(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let (after, limit) = page(query, &request, 50)?;
    let mut schedules = core
        .read_resume_schedule_page(project, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let has_more = schedules.len() > limit as usize;
    schedules.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| schedules.last().map(|item| item.id.to_string()))
        .flatten();
    Ok(json_response(
        StatusCode::OK,
        ListView {
            items: schedules
                .into_iter()
                .map(ResumeScheduleView::from)
                .collect(),
            next_cursor,
        },
        &request,
    ))
}

async fn escalations(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let (after, limit) = page(query, &request, 20)?;
    let mut rows = core
        .read_escalation_page(project, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| rows.last().map(|item| item.escalation.id.to_string()))
        .flatten();
    Ok(json_response(
        StatusCode::OK,
        ListView {
            items: rows.into_iter().map(EscalationView::from).collect(),
            next_cursor,
        },
        &request,
    ))
}

async fn escalation_detail(
    State(core): State<CoreService>,
    Path((project, escalation)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let escalation = uuid_id("escalation_id", &escalation, &request)?;
    core.read_project(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let row = core
        .store()
        .escalation_detail(project, escalation)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?
        .ok_or_else(|| {
            HttpError::from_core(
                request.clone(),
                CoreError::NotFound {
                    aggregate: "escalation",
                },
            )
        })?;
    let route = row.escalation.route.clone().map(RouteView::from);
    let created_by = row.escalation.created_by;
    Ok(json_response(
        StatusCode::OK,
        EscalationDetailView {
            escalation: EscalationView::from(row),
            route,
            created_by,
        },
        &request,
    ))
}

async fn resolver_routes(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let query = query
        .map_err(|error| HttpError::invalid_request(request.clone(), error.to_string()))?
        .0;
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=50).contains(&limit) {
        return Err(HttpError::invalid_request(
            request,
            "limit must be between 1 and 50",
        ));
    }
    if query
        .cursor
        .as_ref()
        .is_some_and(|value| !valid_route_key(value))
    {
        return Err(HttpError::invalid_request(
            request,
            "cursor must be a resolver route key",
        ));
    }
    core.read_project(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let mut routes = core
        .store()
        .resolver_route_page(project, query.cursor.as_deref(), limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let has_more = routes.len() > limit as usize;
    routes.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| routes.last().map(|route| route.key.clone()))
        .flatten();
    Ok(json_response(
        StatusCode::OK,
        ListView {
            items: routes.into_iter().map(RouteView::from).collect(),
            next_cursor,
        },
        &request,
    ))
}

fn valid_route_key(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

async fn next_run_constraints(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let (after, limit) = page(query, &request, 50)?;
    core.read_project(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let mut rows = core
        .store()
        .next_run_constraint_page(project, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| rows.last().map(|row| row.id.to_string()))
        .flatten();
    let items = rows
        .into_iter()
        .map(|row| {
            Ok::<_, CoreError>(serde_json::json!({
                "id":row.id,"task_id":row.task_id,"pipeline_version_id":row.pipeline_version_id,
                "stage_id":row.stage_id,"stage_visit":row.stage_visit,"employee_id":row.employee_id,
                "created_by":row.created_by,"created_at":timestamp(row.created_at)?,"state":row.state,
            }))
        })
        .collect::<Result<Vec<_>,_>>().map_err(|error|HttpError::from_core(request.clone(),error))?;
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}

async fn recovery_settings(
    State(core): State<CoreService>,
    Path(project): Path<String>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    core.read_project(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let settings = core
        .store()
        .recovery_settings_read(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    Ok(json_response(
        StatusCode::OK,
        RecoverySettingsView::from(settings),
        &request,
    ))
}

async fn recovery_runs(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let (after, limit) = page(query, &request, 50)?;
    core.read_project(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let mut rows = core
        .store()
        .recovery_run_page(project, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| rows.last().map(|row| row.run_id.to_string()))
        .flatten();
    Ok(json_response(
        StatusCode::OK,
        ListView {
            items: rows.into_iter().map(RecoveryRunView::from).collect(),
            next_cursor,
        },
        &request,
    ))
}

async fn recovery_assessment_readiness(
    State(core): State<CoreService>,
    Path((project, run)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project_id = project_id(&project, &request)?;
    let run_id = uuid_id("run_id", &run, &request)?;
    let project = core
        .read_project(project_id)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let facts = core
        .store()
        .recovery_assessment_read(project_id, run_id)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?
        .ok_or_else(|| {
            HttpError::from_core(request.clone(), CoreError::NotFound { aggregate: "run" })
        })?;
    let reason_code = not_started_readiness_reason(&facts);
    Ok(json_response(
        StatusCode::OK,
        RecoveryAssessmentReadinessView {
            run_id: facts.run_id,
            assessment: "not_started_confirmed",
            eligible: reason_code.is_none(),
            reason_code,
            project_revision: project.revision(),
            run_revision: facts.run_revision,
            lease_fencing_token: facts.lease_fencing_token,
            environment_epoch: facts.environment_epoch,
            task_id: facts.task_id,
            task_revision: facts.task.as_ref().map(|task| task.revision().get()),
        },
        &request,
    ))
}

#[derive(Serialize)]
struct RecoveryAssessmentReadinessView {
    run_id: Uuid,
    assessment: &'static str,
    eligible: bool,
    reason_code: Option<&'static str>,
    project_revision: u64,
    run_revision: u64,
    lease_fencing_token: u64,
    environment_epoch: u64,
    task_id: Option<Uuid>,
    task_revision: Option<u64>,
}

fn not_started_readiness_reason(facts: &RecoveryAssessmentRead) -> Option<&'static str> {
    if facts.decision_accepted {
        return Some("already_assessed");
    }
    if !facts.reservation_present {
        return Some("recovery_state_unavailable");
    }
    if facts.lease_active {
        return Some("execution_not_retired");
    }
    if facts.reserved {
        return Some("physical_state_unresolved");
    }
    if facts.result_evidence_present {
        return Some("result_evidence_present");
    }
    let (Some(task_id), Some(stage_id), Some(task)) = (
        facts.task_id,
        facts.stage_id.as_deref(),
        facts.task.as_ref(),
    ) else {
        return Some("unsupported_assignment");
    };
    if task.id().as_uuid() != task_id
        || task.lifecycle().is_terminal()
        || task.current_stage_id().map(|stage| stage.as_str()) != Some(stage_id)
    {
        return Some("task_stage_changed");
    }
    if facts.boot_policy == BootRecoveryPolicy::ManualHold {
        return None;
    }
    let detail = format!("recovery_run={}", facts.run_id);
    let Some(wait) = task
        .wait_conditions()
        .find(|wait| wait.detail() == Some(detail.as_str()))
        .map(|wait| wait.id())
    else {
        return Some("recovery_wait_missing");
    };
    let mut resumed = task.clone();
    if resumed
        .resolve_wait_condition(wait, Timestamp::now_utc())
        .is_err()
    {
        return Some("recovery_wait_unresolvable");
    }
    if matches!(
        resumed.lifecycle(),
        forge_domain::LifecycleStatus::Ready | forge_domain::LifecycleStatus::InProgress
    ) {
        let Some(version) = facts.pipeline_version.as_ref() else {
            return Some("pipeline_version_unavailable");
        };
        let Some(stage) = resumed.current_stage_id().and_then(|id| version.stage(id)) else {
            return Some("pipeline_stage_unavailable");
        };
        if stage.executor_kind() != ExecutorKind::Employee {
            return Some("executor_not_employee");
        }
    }
    None
}

#[cfg(test)]
mod recovery_assessment_tests {
    use super::*;

    #[test]
    fn taskless_assignment_never_offers_nonstart_confirmation() {
        let facts = RecoveryAssessmentRead {
            run_id: Uuid::now_v7(),
            run_revision: 1,
            lease_fencing_token: 7,
            environment_epoch: 1,
            task_id: None,
            stage_id: None,
            lease_active: false,
            reservation_present: true,
            reserved: false,
            decision_accepted: false,
            result_evidence_present: false,
            boot_policy: BootRecoveryPolicy::ManualHold,
            task: None,
            pipeline_version: None,
        };
        assert_eq!(
            not_started_readiness_reason(&facts),
            Some("unsupported_assignment")
        );
    }
}

impl CoreService {
    async fn read_resume_schedule_page(
        &self,
        project: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<TaskResumeSchedule>, CoreError> {
        self.read_project(project).await?;
        Ok(self
            .store()
            .task_resume_schedule_page(project, after, limit)
            .await?)
    }

    async fn read_escalation_page(
        &self,
        project: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<EscalationReadRow>, CoreError> {
        self.read_project(project).await?;
        Ok(self.store().escalation_page(project, after, limit).await?)
    }
}

#[derive(Serialize)]
struct RouteView {
    key: String,
    revision: u64,
    employee_ids: Vec<forge_domain::EmployeeId>,
    assignment_timeout_seconds: u32,
    #[serde(serialize_with = "serialize_timestamp")]
    updated_at: forge_domain::Timestamp,
}

impl From<ResolverRoute> for RouteView {
    fn from(value: ResolverRoute) -> Self {
        Self {
            key: value.key,
            revision: value.revision,
            employee_ids: value.employee_ids,
            assignment_timeout_seconds: value.assignment_timeout_seconds,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Serialize)]
struct EscalationDetailView {
    #[serde(flatten)]
    escalation: EscalationView,
    route: Option<RouteView>,
    created_by: forge_domain::Actor,
}

#[derive(Serialize)]
struct RecoverySettingsView {
    boot_policy: forge_domain::runtime::BootRecoveryPolicy,
    hold: bool,
}

impl From<RecoverySettingsRead> for RecoverySettingsView {
    fn from(value: RecoverySettingsRead) -> Self {
        Self {
            boot_policy: value.boot_policy,
            hold: value.hold,
        }
    }
}

#[derive(Serialize)]
struct RecoveryRunView {
    run_id: Uuid,
    task_id: Option<Uuid>,
    desired_state: forge_storage::RunDesiredState,
    observed_state: forge_storage::RunObservedState,
    #[serde(serialize_with = "serialize_timestamp")]
    created_at: forge_domain::Timestamp,
    #[serde(serialize_with = "serialize_optional_timestamp")]
    started_at: Option<forge_domain::Timestamp>,
    #[serde(serialize_with = "serialize_optional_timestamp")]
    liveness_observed_at: Option<forge_domain::Timestamp>,
    #[serde(serialize_with = "serialize_optional_timestamp")]
    stop_requested_at: Option<forge_domain::Timestamp>,
    #[serde(serialize_with = "serialize_timestamp")]
    updated_at: forge_domain::Timestamp,
    lease_active: bool,
    reservation_state: Option<String>,
    reservation_released: Option<bool>,
    accepted_assessment: Option<forge_domain::runtime::RecoveryAssessment>,
    #[serde(serialize_with = "serialize_optional_timestamp")]
    assessment_accepted_at: Option<forge_domain::Timestamp>,
    assessment_command_id: Option<Uuid>,
    recovery_queue_entry_id: Option<Uuid>,
}

impl From<RecoveryRunRead> for RecoveryRunView {
    fn from(value: RecoveryRunRead) -> Self {
        Self {
            run_id: value.run_id,
            task_id: value.task_id,
            desired_state: value.desired_state,
            observed_state: value.observed_state,
            created_at: value.created_at,
            started_at: value.started_at,
            liveness_observed_at: value.liveness_observed_at,
            stop_requested_at: value.stop_requested_at,
            updated_at: value.updated_at,
            lease_active: value.lease_active,
            reservation_state: value.reservation_state,
            reservation_released: value.reservation_released,
            accepted_assessment: value.accepted_assessment,
            assessment_accepted_at: value.assessment_accepted_at,
            assessment_command_id: value.assessment_command_id,
            recovery_queue_entry_id: value.recovery_queue_entry_id,
        }
    }
}

#[derive(Serialize)]
struct ResumeScheduleView {
    id: Uuid,
    task_id: forge_domain::TaskId,
    expected_task_revision: u64,
    pipeline_version_id: forge_domain::PipelineVersionId,
    stage_id: forge_domain::StageId,
    stage_visit: forge_domain::StageVisit,
    wait_condition_id: forge_domain::WaitConditionId,
    #[serde(serialize_with = "serialize_timestamp")]
    not_before: forge_domain::Timestamp,
    reason: String,
    created_by: forge_domain::Actor,
    #[serde(serialize_with = "serialize_timestamp")]
    created_at: forge_domain::Timestamp,
    state: ScheduledResumeStateView,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ScheduledResumeStateView {
    Pending,
    Applied {
        command_id: forge_domain::CommandId,
        #[serde(serialize_with = "serialize_timestamp")]
        at: forge_domain::Timestamp,
    },
    Rejected {
        reason: forge_domain::ResumeRejection,
        #[serde(serialize_with = "serialize_timestamp")]
        at: forge_domain::Timestamp,
    },
    Cancelled {
        #[serde(serialize_with = "serialize_timestamp")]
        at: forge_domain::Timestamp,
    },
}

impl From<forge_domain::ScheduledResumeState> for ScheduledResumeStateView {
    fn from(value: forge_domain::ScheduledResumeState) -> Self {
        match value {
            forge_domain::ScheduledResumeState::Pending => Self::Pending,
            forge_domain::ScheduledResumeState::Applied { command_id, at } => {
                Self::Applied { command_id, at }
            }
            forge_domain::ScheduledResumeState::Rejected { reason, at } => {
                Self::Rejected { reason, at }
            }
            forge_domain::ScheduledResumeState::Cancelled { at } => Self::Cancelled { at },
        }
    }
}

impl From<TaskResumeSchedule> for ResumeScheduleView {
    fn from(value: TaskResumeSchedule) -> Self {
        Self {
            id: value.id,
            task_id: value.task_id,
            expected_task_revision: value.expected_task_revision,
            pipeline_version_id: value.pipeline_version_id,
            stage_id: value.stage_id,
            stage_visit: value.stage_visit,
            wait_condition_id: value.wait_condition_id,
            not_before: value.not_before,
            reason: value.reason,
            created_by: value.created_by,
            created_at: value.created_at,
            state: value.state.into(),
        }
    }
}

#[derive(Serialize)]
struct EscalationView {
    id: Uuid,
    source: EscalationSourceView,
    category: forge_domain::resolution::EscalationCategory,
    question: String,
    allowed_outcomes: Vec<String>,
    revision: u64,
    generation: u64,
    state: forge_domain::resolution::EscalationState,
    latest_assignment: Option<ResolutionAssignmentView>,
    #[serde(serialize_with = "serialize_timestamp")]
    created_at: forge_domain::Timestamp,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum EscalationSourceView {
    Task {
        task_id: forge_domain::TaskId,
        pipeline_version_id: forge_domain::PipelineVersionId,
        stage_id: forge_domain::StageId,
        stage_visit: forge_domain::StageVisit,
        wait_condition_id: forge_domain::WaitConditionId,
    },
    Communication {
        run_id: Uuid,
        assignment_id: Uuid,
        thread_id: Uuid,
        source_message_id: Uuid,
    },
}

#[derive(Serialize)]
struct ResolutionAssignmentView {
    id: Uuid,
    generation: u64,
    resolver: forge_domain::resolution::Resolver,
    #[serde(serialize_with = "serialize_timestamp")]
    issued_at: forge_domain::Timestamp,
    #[serde(serialize_with = "serialize_optional_timestamp")]
    expires_at: Option<forge_domain::Timestamp>,
    state: forge_domain::resolution::ResolutionAssignmentState,
}

impl From<ResolutionAssignment> for ResolutionAssignmentView {
    fn from(value: ResolutionAssignment) -> Self {
        Self {
            id: value.id,
            generation: value.lease.generation,
            resolver: value.resolver,
            issued_at: value.issued_at,
            expires_at: value.lease.expires_at,
            state: value.state,
        }
    }
}

impl From<EscalationReadRow> for EscalationView {
    fn from(value: EscalationReadRow) -> Self {
        let escalation = value.escalation;
        let source = match escalation.source {
            EscalationSource::Task(source) => EscalationSourceView::Task {
                task_id: source.task_id,
                pipeline_version_id: source.pipeline_version_id,
                stage_id: source.stage_id,
                stage_visit: source.stage_visit,
                wait_condition_id: source.wait_condition_id,
            },
            EscalationSource::Communication(source) => EscalationSourceView::Communication {
                run_id: source.run_id,
                assignment_id: source.assignment.assignment_id,
                thread_id: source.assignment.thread_id,
                source_message_id: source.assignment.source_message_id,
            },
        };
        Self {
            id: escalation.id,
            source,
            category: escalation.category,
            question: escalation.question,
            allowed_outcomes: escalation.allowed_outcomes,
            revision: escalation.revision,
            generation: escalation.generation,
            state: escalation.state,
            latest_assignment: value.latest_assignment.map(ResolutionAssignmentView::from),
            created_at: escalation.created_at,
        }
    }
}
