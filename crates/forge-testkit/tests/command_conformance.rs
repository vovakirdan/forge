//! One command contract executed through the production engine on both stores.

#[path = "command_conformance/active_runs.rs"]
mod active_runs;
#[path = "command_conformance/compatibility.rs"]
mod compatibility;
#[path = "command_conformance/create_draft.rs"]
mod create_draft;

#[path = "command_conformance/approval.rs"]
mod approval;
#[path = "command_conformance/atomicity.rs"]
mod atomicity;
#[path = "command_conformance/boundary.rs"]
mod boundary;
#[path = "command_conformance/clocks.rs"]
mod clocks;
#[path = "command_conformance/command_handoffs.rs"]
mod command_handoffs;
#[path = "command_conformance/commands.rs"]
mod commands;
#[path = "command_conformance/communication.rs"]
mod communication;
#[path = "command_conformance/communication_http.rs"]
mod communication_http;
#[path = "command_conformance/employee_capacity.rs"]
mod employee_capacity;
#[path = "command_conformance/employees.rs"]
mod employees;
#[path = "command_conformance/findings.rs"]
mod findings;
#[path = "command_conformance/fixture.rs"]
mod fixture;
#[path = "command_conformance/manager.rs"]
mod manager;
#[path = "command_conformance/manager_constraints.rs"]
mod manager_constraints;
#[path = "command_conformance/manager_resume.rs"]
mod manager_resume;
#[path = "command_conformance/pipelines.rs"]
mod pipelines;
#[path = "command_conformance/priorities.rs"]
mod priorities;
#[path = "command_conformance/priority_scheme_http.rs"]
mod priority_scheme_http;
#[path = "command_conformance/production.rs"]
mod production;
#[path = "command_conformance/repositories.rs"]
mod repositories;
#[path = "command_conformance/resolution.rs"]
mod resolution;
#[path = "command_conformance/resolution_guards.rs"]
mod resolution_guards;
#[path = "command_conformance/scopes.rs"]
mod scopes;
#[path = "command_conformance/snapshot.rs"]
mod snapshot;
#[path = "command_conformance/source_policy.rs"]
mod source_policy;

macro_rules! conformance_case {
    ($name:ident, $scenario:path) => {
        mod $name {
            #[tokio::test]
            async fn memory() -> anyhow::Result<()> {
                $scenario(crate::fixture::BackendKind::Memory).await
            }

            #[tokio::test]
            #[ignore = "requires explicit local PostgreSQL; run just test-integration"]
            async fn postgres() -> anyhow::Result<()> {
                $scenario(crate::fixture::BackendKind::Postgres).await
            }
        }
    };
}

conformance_case!(all_m0_commands, crate::commands::all_m0_commands);
conformance_case!(
    draft_approval,
    crate::approval::dod_edits_and_pipeline_approval
);
conformance_case!(
    create_draft_receipt,
    crate::create_draft::pinned_draft_replays_without_work
);
conformance_case!(
    priority_lifecycle,
    crate::priorities::lifecycle_queue_and_replay
);
conformance_case!(
    priority_active_run,
    crate::priorities::active_run_is_unchanged
);
conformance_case!(
    git_source_policy,
    crate::source_policy::future_policy_atomicity
);
conformance_case!(
    git_source_authority,
    crate::source_policy::source_authority_and_staleness
);
conformance_case!(
    manual_handoff,
    crate::command_handoffs::handoff_is_atomic_and_replayable
);
conformance_case!(
    resolver_human,
    crate::resolution::human_answer_is_atomic_and_only_clears_its_own_wait
);
conformance_case!(
    resolver_routes,
    crate::resolution::routes_are_pinned_and_human_generations_fence_late_answers
);
conformance_case!(
    resolver_authority,
    crate::resolution::dangerous_questions_and_refusals_preserve_authority
);
conformance_case!(
    resolver_physical,
    crate::resolution::active_source_cannot_be_resumed_by_a_human_answer
);
conformance_case!(
    resolver_resume,
    crate::resolution::final_question_wait_resumes_same_stage_without_completing_it
);
conformance_case!(
    resolver_resume_guards,
    crate::resolution_guards::managed_wait_requires_resolution_but_orphan_wait_is_compatible
);
conformance_case!(findings_promotion, crate::findings::report_promote);
conformance_case!(findings_triage, crate::findings::triage_refusals);
conformance_case!(
    manager_pause,
    crate::manager::individual_pause_preserves_physical_ownership
);
conformance_case!(
    manager_employee_stop,
    crate::manager::employee_stop_is_visit_scoped
);
conformance_case!(
    manager_ready_resume,
    crate::manager::ready_pause_can_resume_to_queue
);
conformance_case!(
    manager_refusals,
    crate::manager::management_refusals_are_atomic
);
conformance_case!(
    manager_future_assignment,
    crate::manager_constraints::future_assignment_is_audited_without_moving_current_work
);
conformance_case!(
    manager_assignment_refusals,
    crate::manager_constraints::future_assignment_refusals_preserve_state
);
conformance_case!(
    manager_alarm_commands,
    crate::manager_resume::alarm_intent_and_cancellation_are_atomic
);
conformance_case!(repository_binding, crate::repositories::binding_round_trip);
conformance_case!(
    repository_refusals,
    crate::repositories::refusals_are_atomic
);
conformance_case!(
    repository_storage_guards,
    crate::repositories::storage_guards_preserve_immutable_source
);
conformance_case!(
    employee_management,
    crate::employees::employee_management_round_trip
);
conformance_case!(
    employee_refusals,
    crate::employees::employee_management_refusals_are_atomic
);
conformance_case!(
    employee_repository_cas,
    crate::employees::employee_repository_rejects_stale_overwrite
);
conformance_case!(
    pipeline_versioning,
    crate::pipelines::versioning_preserves_existing_tasks
);
conformance_case!(
    pipeline_refusals,
    crate::pipelines::refusals_and_repository_cas
);
conformance_case!(
    inbox_durability,
    crate::communication::ordered_durable_messages
);
conformance_case!(inbox_scopes, crate::communication::exact_execution_scope);
conformance_case!(inbox_refusals, crate::communication::refusals_are_atomic);
conformance_case!(
    inbox_waiver,
    crate::communication::operator_waiver_is_explicit_and_audited
);
conformance_case!(
    inbox_waiver_recovery,
    crate::communication::waiver_releases_stopped_exact_run_requirement
);
conformance_case!(
    inbox_executor_scope,
    crate::communication::external_stages_reject_execution_directed_messages
);
conformance_case!(
    negative_domain_cases,
    crate::commands::negative_domain_cases
);
conformance_case!(idempotency, crate::boundary::idempotency);
conformance_case!(revisions, crate::boundary::revisions);
conformance_case!(authorization, crate::boundary::authorization);
conformance_case!(concurrency, crate::boundary::concurrency);
conformance_case!(
    atomic_write_failures,
    crate::atomicity::every_write_failure_rolls_back
);
conformance_case!(
    uncommitted_drop,
    crate::atomicity::dropping_successful_uncommitted_command_rolls_back
);
conformance_case!(
    causal_clock,
    crate::clocks::canonical_and_operational_time_are_distinct
);
conformance_case!(
    composite_rollback,
    crate::atomicity::composite_queue_wait_and_artifact_writes_roll_back
);
conformance_case!(
    catalog_scope,
    crate::scopes::catalog_uniqueness_and_project_scope
);
conformance_case!(
    intent_integrity,
    crate::scopes::mutated_typed_intent_cannot_diverge_from_fingerprinted_json
);
conformance_case!(
    active_run_ownership,
    crate::active_runs::stop_cancel_and_dependency_preserve_physical_ownership
);
