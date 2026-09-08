//! PG + real Gateway/gRPC, synthetic Supervisor inspections; no paid provider.
use anyhow::{Context, Result};
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, LifecycleStatus,
    git_delivery::GitProposalState,
    runtime::{RuntimeBinding, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::{
    supervisor::v1::{
        AcknowledgementDisposition, GitCandidateInspectionCode, GitCandidateInspectionResult,
        RunEventKind,
    },
    wire::CommandName,
};
use forge_provider_claude::ClaudeAdapter;
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore};
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};
use std::{collections::BTreeSet, os::unix::fs::DirBuilderExt, time::Duration};
use uuid::Uuid;

#[path = "m2_git_proposal/finding.rs"]
mod finding;
#[path = "m2_git_proposal/integration.rs"]
mod integration;
#[path = "m2_git_proposal/integration_fairness.rs"]
mod integration_fairness;
#[path = "m2_git_proposal/integration_http.rs"]
mod integration_http;
#[path = "m2_git_proposal/qa_writer.rs"]
mod qa_writer;
#[path = "m2_git_proposal/review.rs"]
mod review;
#[path = "m2_git_proposal/setup.rs"]
mod setup;

async fn scenario(mode: &str) -> Result<()> {
    // Keep each phase's alternative async states off the test-thread stack;
    // inline nesting exhausted its default stack during ordinary Core dispatch.
    let setup = Box::pin(setup::create(mode)).await?;
    if let Some(result) = Box::pin(exercise(setup, mode)).await? {
        Box::pin(finish_scenario(result, mode)).await?;
    }
    Ok(())
}

#[test]
fn scenario_future_keeps_phase_state_off_the_test_thread_stack() {
    fn size<F: std::future::Future>(_: impl FnOnce() -> F) -> usize {
        std::mem::size_of::<F>()
    }
    // Type measurement does not poll a future or create services/resources.
    let bytes = size(|| scenario("finding"));
    assert!(bytes <= 16 * 1024, "scenario future is {bytes} bytes");
}

struct ScenarioResult {
    setup: setup::Setup,
    task: forge_domain::TaskId,
    final_task: forge_domain::Task,
    client: reqwest::Client,
}

async fn exercise(setup: setup::Setup, mode: &str) -> Result<Option<ScenarioResult>> {
    let with_review = matches!(mode, "review_pass" | "review_rework" | "qa_report");
    let with_integration = mode.starts_with("integration_");
    let setup::Setup {
        root,
        harness,
        mut supervisor,
        project,
        employee,
        binding,
        version,
    } = setup;
    let task = harness
        .create_task(project, version, "Candidate proposal")
        .await?;
    if mode == "unbound_policy" {
        let mut employee_source = binding.clone();
        employee_source.access = SurfaceAccess::ReadOnly;
        employee_source.surface = SurfaceSpec::GitWorktree {
            repository: root.join("mock-source").to_string_lossy().into_owned(),
            base_ref: "a".repeat(40),
        };
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":employee,"binding":employee_source}),
            )
            .await?;
        assert!(
            harness.approve_task(project, task).await.is_err(),
            "independent=false cannot bypass a configured candidate policy using an Employee-owned source"
        );
        assert_eq!(
            harness
                .store
                .load_task(task)
                .await?
                .context("Task")?
                .task
                .lifecycle(),
            LifecycleStatus::Draft
        );
        assert!(harness.store.list_runs(task).await?.is_empty());
        harness.shutdown().await;
        return Ok(None);
    }
    let repository=harness.execute(project,CommandName::RegisterProjectRepository,json!({"name":"fixture","source":root.join("mock-source"),"target_ref":"refs/heads/accepted"})).await?.resource.context("repository")?.id;
    let revision = harness
        .store
        .load_task(task)
        .await?
        .context("task")?
        .task
        .revision()
        .get();
    harness.execute(project,CommandName::BindTaskGitRepository,json!({"task_id":task,"expected_task_revision":revision,"repository_id":repository,"initial_base":"a".repeat(40)})).await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    let provision = supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let spec: Value = serde_json::from_str(&provision.run_spec_json)?;
    assert_eq!(
        spec["binding"]["surface"]["repository"],
        json!(root.join("mock-source")),
        "Task source overrides Employee source"
    );
    let client = reqwest::Client::builder()
        .unix_socket(
            root.join("gateways")
                .join(run.id.to_string())
                .join("gateway.sock"),
        )
        .timeout(Duration::from_secs(8))
        .build()?;
    let call = |tool: &str, args: Value, id: Uuid| {
        client
            .post("http://localhost/tools")
            .json(&json!({"tool":tool,"arguments":args,"message_id":id}))
    };
    let artifact_id = Uuid::now_v7();
    let artifact=call("artifact.submit",json!({"artifact_kind":"stage_evidence","title":"Writer report","body":{"summary":"Synthetic report, no model execution"}}),artifact_id).send().await?;
    assert!(artifact.status().is_success(), "{}", artifact.text().await?);
    if mode == "finding" {
        finding::report(&harness, &client, project, task, &run).await?;
    }
    let outcome_id = Uuid::now_v7();
    let args = json!({"stage_id":"work","outcome":"completed","artifact_submission_message_ids":[artifact_id],"candidate_commit":"b".repeat(40)});
    let response = call("outcome.submit", args, outcome_id).send().await?;
    assert!(response.status().is_success(), "{}", response.text().await?);
    assert_eq!(response.json::<Value>().await?["status"], "proposal_saved");
    if with_review && mode != "qa_report" {
        // An equally capable former writer is still not an independent reviewer.
        let mut readonly_binding = binding.clone();
        readonly_binding.access = SurfaceAccess::ReadOnly;
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":employee,"binding":readonly_binding}),
            )
            .await?;
    }
    assert_eq!(
        harness
            .store
            .load_task(task)
            .await?
            .context("task")?
            .task
            .lifecycle(),
        LifecycleStatus::InProgress
    );
    supervisor.next_stop_for_run(&run.id.to_string()).await?;
    let mut tx = harness.store.begin().await?;
    let stored = tx.load_git_proposal(run.id).await?.context("proposal")?;
    assert_eq!(stored.state, GitProposalState::AwaitingQuiescence);
    assert!(!tx.git_writer_quiescent(&stored.proposal).await?);
    tx.commit().await?;
    // A task-directed instruction is accepted even after submission. It must be
    // rechecked at the FINAL decision, not merely in the earlier writer request.
    if mode == "late_instruction" {
        let thread = harness
            .execute(
                project,
                CommandName::OpenEmployeeThread,
                json!({"employee_id":employee}),
            )
            .await?
            .resource
            .context("thread")?
            .id;
        harness.execute(project,CommandName::SendEmployeeMessage,json!({"thread_id":thread,"expected_thread_revision":1,
            "target":{"kind":"task_execution","context":{"task_id":task,"pipeline_version_id":version,"stage_id":"work","stage_visit":stored.proposal.stage_visit}},
            "kind":"instruction","requirement":"answered","body":"Explain this result before closing."})).await?;
    }
    let stopped = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    assert_eq!(
        stopped.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "{stopped:?}"
    );
    assert_eq!(
        harness
            .store
            .load_task(task)
            .await?
            .context("task")?
            .task
            .lifecycle(),
        LifecycleStatus::InProgress
    );
    harness
        .core
        .watchdog_tick(
            forge_domain::Timestamp::now_utc(),
            forge_core::WatchdogDeadlines::default(),
        )
        .await?;
    let mut request = supervisor.next_git_inspection().await?;
    if mode == "busy" {
        let busy = GitCandidateInspectionResult {
            message_id: Uuid::now_v7().to_string(),
            request_command_id: request.command_id.clone(),
            run_id: run.id.to_string(),
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
            commit: String::new(),
            tree: String::new(),
            result_code: GitCandidateInspectionCode::Busy as i32,
            host_id: request.host_id.clone(),
            boot_id: request.boot_id.clone(),
        };
        assert_eq!(
            supervisor
                .send_git_inspection(busy.clone())
                .await?
                .disposition,
            AcknowledgementDisposition::Accepted as i32
        );
        let prior_id = request.command_id.clone();
        harness
            .core
            .watchdog_tick(
                forge_domain::Timestamp::now_utc(),
                forge_core::WatchdogDeadlines::default(),
            )
            .await?;
        request = supervisor.next_git_inspection().await?;
        assert_ne!(
            request.command_id, prior_id,
            "Busy retry needs a new durable request identity"
        );
        assert_eq!(
            supervisor.send_git_inspection(busy).await?.disposition,
            AcknowledgementDisposition::Accepted as i32,
            "old reply stays replayable after retry"
        );
    }
    assert_eq!(request.expected_commit, "b".repeat(40));
    if mode == "stale_revision" {
        harness.set_task_priority(project, task, "normal").await?;
    }
    let mut result = GitCandidateInspectionResult {
        message_id: Uuid::now_v7().to_string(),
        request_command_id: request.command_id,
        run_id: run.id.to_string(),
        lease_fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
        commit: "b".repeat(40),
        tree: "c".repeat(40),
        result_code: GitCandidateInspectionCode::Verified as i32,
        host_id: request.host_id,
        boot_id: request.boot_id,
    };
    let mut forged = result.clone();
    forged.lease_fencing_token += 1;
    assert_eq!(
        supervisor.send_git_inspection(forged).await?.disposition,
        AcknowledgementDisposition::Rejected as i32
    );
    if mode == "dirty" {
        result.commit.clear();
        result.tree.clear();
        result.result_code = GitCandidateInspectionCode::DirtyCandidate as i32;
    }
    let ack = supervisor.send_git_inspection(result.clone()).await?;
    assert_eq!(
        ack.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "{ack:?}"
    );
    assert_eq!(
        supervisor.send_git_inspection(result).await?.disposition,
        AcknowledgementDisposition::Accepted as i32
    );
    let final_task = harness.store.load_task(task).await?.context("task")?.task;
    let success = matches!(mode, "success" | "busy" | "finding") || with_review || with_integration;
    assert_eq!(
        final_task.lifecycle(),
        if with_review || with_integration {
            LifecycleStatus::InProgress
        } else if success {
            LifecycleStatus::Done
        } else {
            LifecycleStatus::Waiting
        }
    );
    let mut tx = harness.store.begin().await?;
    let final_proposal = tx.load_git_proposal(run.id).await?.context("proposal")?;
    assert_eq!(
        final_proposal.state,
        if success {
            GitProposalState::Accepted
        } else if mode == "stale_revision" {
            GitProposalState::Superseded
        } else {
            GitProposalState::NeedsAttention
        }
    );
    assert_eq!(
        final_proposal.proposal, stored.proposal,
        "immutable intent is not rewritten by observations"
    );
    let accepted = tx.latest_accepted_git_candidate(project, task).await?;
    assert_eq!(
        accepted.is_some(),
        success,
        "verified but unaccepted candidate cannot grant a review source"
    );
    assert!(
        tx.latest_accepted_git_candidate(forge_domain::ProjectId::new(), task)
            .await?
            .is_none()
    );
    assert!(
        tx.latest_accepted_git_candidate(project, forge_domain::TaskId::new())
            .await?
            .is_none()
    );
    tx.commit().await?;
    Ok(Some(ScenarioResult {
        setup: setup::Setup {
            root,
            harness,
            supervisor,
            project,
            employee,
            binding,
            version,
        },
        task,
        final_task,
        client,
    }))
}

async fn finish_scenario(result: ScenarioResult, mode: &str) -> Result<()> {
    let ScenarioResult {
        setup:
            setup::Setup {
                root,
                harness,
                mut supervisor,
                project,
                employee,
                binding,
                ..
            },
        task,
        final_task,
        client,
    } = result;
    let call = |tool: &str, args: Value, id: Uuid| {
        client
            .post("http://localhost/tools")
            .json(&json!({"tool":tool,"arguments":args,"message_id":id}))
    };
    if mode.starts_with("integration_") {
        integration::finish(&harness, &mut supervisor, project, task, mode).await?;
    }
    if mode == "integration_applied" {
        integration_http::assert_history(&harness, project, task).await?;
    }
    if mode == "finding" {
        // Quiescence can close the socket before this late call. Both the
        // closed Gateway and an explicit rejection deny stale Run authority.
        match call(
            "finding.report",
            json!({"description":"Too late for this execution","severity":"normal"}),
            Uuid::now_v7(),
        )
        .send()
        .await
        {
            Ok(response) => assert!(!response.status().is_success()),
            Err(error) => assert!(error.is_connect(), "unexpected Gateway failure: {error}"),
        }
        assert_eq!(
            harness
                .store
                .finding_page(project, None, None, 100)
                .await?
                .len(),
            1
        );
    }
    if matches!(mode, "review_pass" | "review_rework" | "qa_report") {
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":employee,"binding":binding}),
            )
            .await?;
        review::finish(
            &harness,
            &mut supervisor,
            project,
            task,
            &root,
            mode,
            employee,
        )
        .await?;
    }
    if matches!(mode, "dirty" | "stale_revision") {
        let wait = final_task
            .wait_conditions()
            .next()
            .context("manual hold")?
            .id();
        harness.execute(project,CommandName::ResumeTask,json!({"task_id":task,"expected_task_revision":final_task.revision().get(),"wait_condition_id":wait})).await?;
        supervisor.next_provision_for_task(task).await?;
        assert_eq!(
            harness.wait_for_run_count(task, 2).await?.len(),
            2,
            "retired proposal queue cannot strand a resumed Task"
        );
    }
    harness.stop_project(project).await?;
    harness.shutdown().await;
    Ok(())
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic inspection, no provider"]
async fn git_proposal_accepts_only_after_positive_quiescence_and_exact_inspection() -> Result<()> {
    scenario("success").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic inspection, no provider"]
async fn git_proposal_dirty_workspace_holds_task_and_retains_intent() -> Result<()> {
    scenario("dirty").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic inspection, no provider"]
async fn git_proposal_final_acceptance_rechecks_late_instruction() -> Result<()> {
    scenario("late_instruction").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic inspection, no provider"]
async fn git_proposal_busy_inspection_retries_with_fresh_intent_and_keeps_history() -> Result<()> {
    scenario("busy").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic inspection, no provider"]
async fn git_proposal_stale_revision_holds_without_stranding_next_attempt() -> Result<()> {
    scenario("stale_revision").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic inspection, no provider"]
async fn independent_review_pins_revision_and_records_authorized_acceptance() -> Result<()> {
    scenario("review_pass").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic inspection, no provider"]
async fn rejected_review_returns_same_task_to_work_after_reader_quiescence() -> Result<()> {
    scenario("review_rework").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic inspection, no provider"]
async fn qa_report_is_a_readonly_employee_stage_without_implicit_acceptance_policy() -> Result<()> {
    scenario("qa_report").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; no provider"]
async fn self_review_policy_still_requires_task_owned_candidate_before_approval() -> Result<()> {
    scenario("unbound_policy").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; real Task Gateway, no provider inference"]
async fn finding_gateway_keeps_authority_and_backlog_separate() -> Result<()> {
    scenario("finding").await
}
