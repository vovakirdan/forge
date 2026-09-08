//! Canonical recovery failure injection through a real UDS Supervisor; no inference.

use anyhow::{Context, Result};
use forge_core::WatchdogDeadlines;
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, LifecycleStatus, ProjectId,
    TaskId, Timestamp,
    runtime::{ResourceLimits, RunScope, RuntimeBinding, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::{
    supervisor::v1::{
        AcknowledgementDisposition, EnvironmentPresence, RunEventKind, RunInventoryEntry, StopMode,
    },
    wire::CommandName,
};
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore};
use forge_testkit::m0::{M0Harness, ManualSupervisor, single_stage_pipeline};
use serde_json::json;
use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::PathBuf};
use uuid::Uuid;

#[path = "recovery_acceptance/watchdog_clock_tests.rs"]
mod watchdog_clock_tests;

struct Fixture {
    harness: M0Harness,
    root: PathBuf,
    project: ProjectId,
    host: String,
    boot: String,
}

impl Fixture {
    async fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!("frec-{}", Uuid::now_v7()));
        fs::create_dir(&root)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let store = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
        let shared = root.clone();
        let harness = M0Harness::start_configured(move |core| {
            Ok(core.with_secret_store(store).with_execution_root(shared)?)
        })
        .await?;
        let project = harness.create_project("Recovery acceptance").await?;
        let secret_id = Uuid::now_v7();
        let binding_id = Uuid::now_v7();
        let auth=SecretBytes::new(br#"{"auth_mode":"chatgpt","tokens":{"access_token":"synthetic-access","refresh_token":"synthetic-refresh","id_token":"synthetic-id","account_id":"synthetic"},"last_refresh":"2026-09-01T10:00:00Z"}"#.to_vec());
        let file = PrivateMaterialization::create(&root.join("auth-source.json"), &auth)?;
        harness.execute(project,CommandName::EnrollCredential,json!({"secret_id":secret_id,"binding_id":binding_id,"kind":"codex_chatgpt","source_file":file.path()})).await?;
        harness.create_employee(project, "Recovery worker").await?;
        let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
        let profile = ExecutionProfileInput {
            id: Uuid::now_v7(),
            revision: 1,
            project_id: project,
            adapter_id: "codex_cli".into(),
            adapter_version: "0.153.2".into(),
            provider_id: "openai".into(),
            model: "synthetic".into(),
            credential_binding: CredentialBinding {
                id: binding_id,
                project_id: project,
                secret_id,
                account_id: Some("synthetic".into()),
                allowed_delivery_modes: BTreeSet::from([mode]),
            },
            credential_delivery: mode,
            capability_profile: forge_provider_codex::CodexAdapter::capabilities(),
        }
        .try_into()?;
        let binding = RuntimeBinding {
            execution_profile: profile,
            image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
            surface: SurfaceSpec::FilesystemSandbox,
            access: SurfaceAccess::ReadWrite,
            limits: ResourceLimits::default(),
            budget: Default::default(),
            system_prompt: "Synthetic fixture.".into(),
            employee_prompt: "No provider execution.".into(),
        };
        let employee = harness
            .store
            .list_employees(project)
            .await?
            .remove(0)
            .employee;
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":employee.id(),"binding":binding}),
            )
            .await?;
        Ok(Self {
            harness,
            root,
            project,
            host: format!("recovery-host-{}", Uuid::now_v7()),
            boot: format!("recovery-boot-{}", Uuid::now_v7()),
        })
    }
    async fn supervisor(&self, boot: &str) -> Result<ManualSupervisor> {
        self.harness
            .attach_manual_supervisor_with_identity(&self.host, boot)
            .await
    }
    async fn tasks(&self) -> Result<(TaskId, TaskId)> {
        let pipeline = self
            .harness
            .create_pipeline(self.project, single_stage_pipeline())
            .await?;
        let first = self
            .harness
            .create_task(self.project, pipeline, "interrupted")
            .await?;
        let second = self
            .harness
            .create_task(self.project, pipeline, "ordinary queued")
            .await?;
        self.harness.approve_task(self.project, first).await?;
        self.harness.approve_task(self.project, second).await?;
        self.harness.start_project(self.project).await?;
        Ok((first, second))
    }

    async fn finish(&self, supervisor: &mut ManualSupervisor) -> Result<()> {
        self.harness.stop_project(self.project).await?;
        for run in self
            .harness
            .store
            .list_runs_for_project(self.project)
            .await?
        {
            if run.observed_state != forge_storage::RunObservedState::Stopped {
                supervisor
                    .send_observation_kind(
                        &run,
                        run.lease_fencing_token,
                        run.last_sequence + 1,
                        RunEventKind::Stopped,
                    )
                    .await?;
            }
        }
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; uses synthetic credentials and manual Supervisor"]
async fn heartbeat_loss_revokes_scope_but_retains_physical_writer_until_stop() -> Result<()> {
    let fixture = Fixture::new().await.context("create recovery fixture")?;
    let mut supervisor = fixture.supervisor(&fixture.boot).await?;
    supervisor.reconcile_empty().await?;
    let (task, queued) = fixture.tasks().await?;
    supervisor.next_provision_for_task(task).await?;
    let run = fixture.harness.wait_for_run_count(task, 1).await?.remove(0);
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Running)
        .await?;
    // Explicit fault injection alters only the clock under test, not task meaning.
    sqlx::query(
        "UPDATE runs SET liveness_observed_at=clock_timestamp()-INTERVAL '91 seconds' WHERE id=$1",
    )
    .bind(run.id)
    .execute(&fixture.harness.pool)
    .await?;
    let before: String =
        sqlx::query_scalar("SELECT liveness_observed_at::text FROM runs WHERE id=$1")
            .bind(run.id)
            .fetch_one(&fixture.harness.pool)
            .await?;
    let stale = supervisor
        .send_observation_kind(
            &run,
            run.lease_fencing_token + 1,
            2,
            RunEventKind::Heartbeat,
        )
        .await?;
    assert_ne!(
        stale.disposition,
        AcknowledgementDisposition::Accepted as i32
    );
    let after: String =
        sqlx::query_scalar("SELECT liveness_observed_at::text FROM runs WHERE id=$1")
            .bind(run.id)
            .fetch_one(&fixture.harness.pool)
            .await?;
    assert_eq!(
        before, after,
        "stale heartbeat cannot extend a current Run deadline"
    );
    let report = fixture
        .harness
        .core
        .watchdog_tick(Timestamp::now_utc(), WatchdogDeadlines::default())
        .await?;
    assert!(report.quarantined >= 1);
    let stop = supervisor.next_stop_for_run(&run.id.to_string()).await?;
    assert_eq!(stop.mode, StopMode::Graceful as i32);
    let scope = RunScope {
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
    };
    assert!(
        !fixture
            .harness
            .store
            .begin()
            .await?
            .gateway_scope_is_active(scope)
            .await?
    );
    let reserved: bool = sqlx::query_scalar(
        "SELECT released_at IS NULL FROM run_environment_reservations WHERE run_id=$1",
    )
    .bind(run.id)
    .fetch_one(&fixture.harness.pool)
    .await?;
    assert!(reserved);
    assert_eq!(
        fixture.harness.required_task(task).await?.task.lifecycle(),
        LifecycleStatus::Waiting
    );
    assert!(
        fixture
            .harness
            .store
            .list_runs_for_project(fixture.project)
            .await?
            .iter()
            .all(|run| run.task_id() != Some(queued))
    );
    sqlx::query(
        "UPDATE runs SET stop_requested_at=clock_timestamp()-INTERVAL '31 seconds' WHERE id=$1",
    )
    .bind(run.id)
    .execute(&fixture.harness.pool)
    .await?;
    fixture
        .harness
        .core
        .watchdog_tick(Timestamp::now_utc(), WatchdogDeadlines::default())
        .await?;
    let forced = supervisor.next_stop_for_run(&run.id.to_string()).await?;
    assert_eq!(forced.mode, StopMode::Force as i32);
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 2, RunEventKind::Stopped)
        .await?;
    let handoffs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM task_handoffs WHERE run_id=$1 AND kind='interrupted'",
    )
    .bind(run.id)
    .fetch_one(&fixture.harness.pool)
    .await?;
    assert_eq!(handoffs, 1);
    fixture.finish(&mut supervisor).await
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; uses synthetic credentials and manual Supervisor"]
async fn same_boot_reconnect_restores_gateway_without_revoking_or_restarting_run() -> Result<()> {
    let fixture = Fixture::new().await?;
    let mut supervisor = fixture.supervisor(&fixture.boot).await?;
    supervisor.reconcile_empty().await?;
    let (task, _) = fixture.tasks().await?;
    supervisor.next_provision_for_task(task).await?;
    let run = fixture.harness.wait_for_run_count(task, 1).await?.remove(0);
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Running)
        .await?;
    drop(supervisor);
    fixture.harness.wait_for_supervisor_disconnected().await?;
    let mut supervisor = fixture.supervisor(&fixture.boot).await?;
    supervisor
        .reconcile(vec![RunInventoryEntry {
            run_id: run.id.to_string(),
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
            last_sequence: 1,
            presence: EnvironmentPresence::Active as i32,
            environment_id: "synthetic-container".into(),
            provision_boot_id: fixture.boot.clone(),
        }])
        .await?;
    assert!(
        fixture
            .harness
            .store
            .begin()
            .await?
            .gateway_scope_is_active(RunScope {
                run_id: run.id,
                fencing_token: run.lease_fencing_token,
                environment_epoch: run.environment_epoch
            })
            .await?
    );
    assert_eq!(
        fixture
            .harness
            .store
            .list_runs_for_project(fixture.project)
            .await?
            .iter()
            .filter(|run| run.task_id() == Some(task))
            .count(),
        1
    );
    assert!(
        fixture
            .root
            .join("gateways")
            .join(run.id.to_string())
            .join("gateway.sock")
            .exists()
    );
    fixture.finish(&mut supervisor).await
}

async fn reboot_case(policy: &str) -> Result<()> {
    let fixture = Fixture::new().await.context("create reboot fixture")?;
    let mut supervisor = fixture.supervisor(&fixture.boot).await?;
    supervisor
        .reconcile_empty()
        .await
        .context("initial inventory")?;
    fixture
        .harness
        .execute(
            fixture.project,
            CommandName::ConfigureBootRecoveryPolicy,
            json!({"policy":policy}),
        )
        .await
        .context("configure boot policy")?;
    let (task, ordinary) = fixture
        .tasks()
        .await
        .context("create approved queued tasks")?;
    supervisor.next_provision_for_task(task).await?;
    let run = fixture.harness.wait_for_run_count(task, 1).await?.remove(0);
    drop(supervisor);
    fixture.harness.wait_for_supervisor_disconnected().await?;
    let mut supervisor = fixture.supervisor("after-reboot").await?;
    supervisor
        .reconcile_empty()
        .await
        .context("post-reboot inventory")?;
    let (lease,reserved):(String,bool)=sqlx::query_as("SELECT l.lease_state,e.released_at IS NULL FROM runs r JOIN leases l ON l.id=r.lease_id JOIN run_environment_reservations e ON e.run_id=r.id WHERE r.id=$1").bind(run.id).fetch_one(&fixture.harness.pool).await?;
    assert_eq!(lease, "revoked");
    assert!(!reserved);
    assert_eq!(
        fixture.harness.required_task(task).await?.task.lifecycle(),
        LifecycleStatus::Waiting
    );
    let runs = fixture
        .harness
        .store
        .list_runs_for_project(fixture.project)
        .await?;
    assert_eq!(
        runs.iter()
            .filter(|run| run.task_id() == Some(ordinary))
            .count(),
        usize::from(policy == "reconcile_then_resume_queue")
    );
    fixture
        .harness
        .execute(
            fixture.project,
            CommandName::AcceptRunRecoveryAssessment,
            json!({"run_id":run.id,"assessment":"not_started_confirmed"}),
        )
        .await
        .context("accept safe recovery assessment")?;
    let waiting =
        fixture.harness.required_task(task).await?.task.lifecycle() == LifecycleStatus::Waiting;
    assert_eq!(waiting, policy == "manual_hold");
    if policy == "recover_safe_then_hold" {
        let next = supervisor.next_provision_for_task(task).await?;
        assert_ne!(next.run_id, run.id.to_string());
        assert!(next.lease_fencing_token > run.lease_fencing_token);
        assert!(
            fixture
                .harness
                .store
                .list_runs_for_project(fixture.project)
                .await?
                .iter()
                .all(|run| run.task_id() != Some(ordinary))
        );
    }
    fixture.finish(&mut supervisor).await
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; serial boot failure injection"]
async fn default_boot_policy_only_reexecutes_accepted_not_started_and_holds_ordinary_queue()
-> Result<()> {
    reboot_case("recover_safe_then_hold").await
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; serial boot failure injection"]
async fn manual_hold_does_not_auto_resume_even_an_accepted_safe_assessment() -> Result<()> {
    reboot_case("manual_hold").await
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; serial boot failure injection"]
async fn resume_queue_policy_keeps_interrupted_work_waiting_but_dispatches_ordinary_tasks()
-> Result<()> {
    reboot_case("reconcile_then_resume_queue").await
}

async fn provider_failure(kind: RunEventKind) -> Result<()> {
    let fixture = Fixture::new().await?;
    let mut supervisor = fixture.supervisor(&fixture.boot).await?;
    supervisor.reconcile_empty().await?;
    let (task, _) = fixture.tasks().await?;
    supervisor.next_provision_for_task(task).await?;
    let run = fixture.harness.wait_for_run_count(task, 1).await?.remove(0);
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Running)
        .await?;
    let ack = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 2, kind)
        .await?;
    assert_eq!(ack.disposition, AcknowledgementDisposition::Accepted as i32);
    supervisor.next_stop_for_run(&run.id.to_string()).await?;
    assert_eq!(
        fixture.harness.required_task(task).await?.task.lifecycle(),
        LifecycleStatus::Waiting
    );
    let held: bool = sqlx::query_scalar(
        "SELECT released_at IS NULL FROM run_environment_reservations WHERE run_id=$1",
    )
    .bind(run.id)
    .fetch_one(&fixture.harness.pool)
    .await?;
    assert!(held, "provider failure is not physical exit evidence");
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 3, RunEventKind::Stopped)
        .await?;
    fixture.finish(&mut supervisor).await
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn provider_failure_pauses_and_stops_without_releasing_a_live_writer() -> Result<()> {
    provider_failure(RunEventKind::ProviderFailed).await
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn exhausted_budget_pauses_and_stops_without_releasing_a_live_writer() -> Result<()> {
    provider_failure(RunEventKind::BudgetExhausted).await
}
