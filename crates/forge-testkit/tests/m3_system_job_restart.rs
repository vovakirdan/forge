//! Replacement-Core recovery over a retained private schema and real Supervisor UDS.
//! Initial admission uses a storage fixture; result acceptance crosses the real Gateway.
//! Only synthetic sealed credentials and a non-executing ManualSupervisor are used.

#[path = "m3_system_job_restart/support.rs"]
mod support;

use anyhow::{Context, Result};
use forge_application::CommandEnvelope;
use forge_core::{CoreActors, CoreService, SupervisorHub, SupervisorService};
use forge_domain::{ActorId, runtime::RunScope};
use forge_protocol::{
    supervisor::v1::{
        AcknowledgementDisposition, EnvironmentPresence, RunEventKind, RunInventoryEntry, StopMode,
        supervisor_control_server::SupervisorControlServer,
    },
    wire::{CommandName, CommandRequest},
};
use forge_provider_common::SecretStore;
use forge_storage::{PostgresStore, RunDesiredState, RunProjection};
use forge_testkit::m0::ManualSupervisor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{net::UnixListener, time::timeout};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL/NATS; replacement Core and ManualSupervisor, no inference"]
async fn accepted_result_survives_core_restart_and_materializes_once_after_physical_stop()
-> Result<()> {
    restart_case(true).await
}

#[tokio::test]
#[ignore = "requires PostgreSQL/NATS; replacement Core and ManualSupervisor, no inference"]
async fn unfinished_v7_is_not_restored_or_retried_before_physical_stop() -> Result<()> {
    restart_case(false).await
}

fn digest(value: &Value) -> Result<String> {
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn scope(run: &RunProjection) -> RunScope {
    RunScope {
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
    }
}

struct AcceptedResult {
    message_id: Uuid,
    hash: String,
    result: Value,
    receipt: Value,
}

async fn accept_fixture_result(
    client: &reqwest::Client,
    run: &RunProjection,
) -> Result<AcceptedResult> {
    let result = json!({"source_digest":run.run_spec["input"]["source_digest"],"entries":[{
        "subject":{"kind":"employee_memory_entry","employee_id":run.run_spec["input"]["target_employee_id"],"task_id":null},
        "markdown":"Synthetic orientation note retained across the Core restart.",
        "source_refs":run.run_spec["input"]["context"]["source_refs"]
    }]});
    let hash = digest(&json!({"tool":"system_job.submit_result","arguments":result}))?;
    let message_id = Uuid::now_v7();
    let response = client
        .post("http://localhost/tools")
        .header(reqwest::header::CONNECTION, "close")
        .json(
            &json!({"tool":"system_job.submit_result","arguments":result,"message_id":message_id}),
        )
        .send()
        .await?;
    assert!(response.status().is_success());
    let receipt: Value = response.json().await?;
    assert_eq!(
        receipt["state"],
        "result_received_awaiting_physical_quiescence"
    );
    Ok(AcceptedResult {
        message_id,
        hash,
        result,
        receipt,
    })
}

async fn restart_case(accept_result: bool) -> Result<()> {
    let f = support::setup().await?;
    let run = &f.run;
    // Own this listener explicitly so simulating a process exit can abort it
    // without first releasing the Run's canonical physical reservation.
    let gateway =
        f.h.core
            .serve_run_gateway(run.id, &f.root.join("gateways"))
            .await?;
    let client = reqwest::Client::builder()
        .unix_socket(gateway.socket_path())
        .timeout(Duration::from_secs(2))
        .build()?;
    let read = client
        .post("http://localhost/tools")
        .header(reqwest::header::CONNECTION, "close")
        .json(&json!({"tool":"system_job.read","arguments":{},"message_id":Uuid::now_v7()}))
        .send()
        .await?;
    assert!(
        read.status().is_success(),
        "synthetic credential must authorize the old Gateway"
    );
    let input: Value = read.json().await?;
    assert_eq!(input["input"], run.run_spec["input"]);
    let accepted = if accept_result {
        Some(accept_fixture_result(&client, run).await?)
    } else {
        None
    };
    let store = f.h.store.clone();
    let pool = f.h.pool.clone();
    gateway.shutdown().await;
    drop(client);
    // End the old management incarnation, without stop, revoke or a physical report.
    f.h.shutdown().await;

    let root = f.root;
    let secrets = SecretStore::load(&root.join("secrets"))?;
    support::assert_reopened_credential(&secrets, &store, run).await?;
    let core = CoreService::new(
        store.clone(),
        CoreActors::new(ActorId::new(), ActorId::new()),
        Arc::new(SupervisorHub::default()),
    )
    .with_secret_store(secrets)
    .with_execution_root(root.clone())?;
    let socket = root.join("core.sock");
    let listener = UnixListener::bind(&socket)?;
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let mut server = tokio::spawn(
        Server::builder()
            .add_service(SupervisorControlServer::new(SupervisorService::new(
                core.clone(),
            )))
            .serve_with_incoming_shutdown(UnixListenerStream::new(listener), async {
                let _ = stop_rx.await;
            }),
    );
    let outcome = exercise_restart(&store, &pool, &core, &socket, run, accepted.as_ref()).await;
    let _ = stop_tx.send(());
    match timeout(Duration::from_secs(2), &mut server).await {
        Ok(joined) => joined.context("join replacement Core server")??,
        Err(_) => {
            server.abort();
            let _ = server.await;
            anyhow::bail!("replacement Core server did not stop within its test deadline");
        }
    }
    outcome
}

async fn exercise_restart(
    store: &PostgresStore,
    pool: &sqlx::PgPool,
    core: &CoreService,
    socket: &Path,
    run: &RunProjection,
    accepted: Option<&AcceptedResult>,
) -> Result<()> {
    let owner = run
        .assignment
        .system_job()
        .context("SystemJob assignment")?;
    let mut supervisor = ManualSupervisor::connect(socket).await?;
    supervisor
        .reconcile(vec![RunInventoryEntry {
            run_id: run.id.to_string(),
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
            last_sequence: 0,
            presence: EnvironmentPresence::Active as i32,
            environment_id: "retained-v7".into(),
            provision_boot_id: "m0-acceptance-boot".into(),
        }])
        .await?;
    // Inventory alone must not prepare old V7 credentials, sockets or replay a provision.
    let gateway = socket
        .parent()
        .context("execution root")?
        .join("gateways")
        .join(run.id.to_string())
        .join("gateway.sock");
    assert!(
        tokio::net::UnixStream::connect(&gateway).await.is_err(),
        "restart recreated the old semantic Gateway despite valid pinned credentials"
    );
    assert!(
        timeout(Duration::from_millis(150), supervisor.next_provision())
            .await
            .is_err()
    );
    core.system_job_tick().await?;
    assert_eq!(
        store
            .load_run(run.id)
            .await?
            .context("fenced Run")?
            .desired_state,
        RunDesiredState::ForceStopRequested,
        "restart must durably fence the old Core before waiting for transport delivery"
    );
    wait_for_fenced_force(&mut supervisor, run).await?;
    core.system_job_tick().await?;
    let mut tx = store.begin().await?;
    let attempt = tx
        .system_job_attempt(run.project_id, owner.attempt_id)
        .await?
        .context("retained attempt")?;
    assert_eq!(attempt.state, "running");
    assert!(!attempt.physical_quiescent);
    assert_eq!(attempt.result.as_ref(), accepted.map(|value| &value.result));
    assert!(
        tx.run_recovery_state(run.id)
            .await?
            .context("reservation")?
            .reserved
    );
    assert!(tx.validate_gateway_scope(&scope(run)).await?.is_none());
    if let Some(accepted) = accepted {
        assert_eq!(
            tx.gateway_receipt(scope(run), accepted.message_id, &accepted.hash)
                .await?,
            Some(accepted.receipt.clone()),
            "restart retains the original accepted receipt"
        );
    }
    tx.commit().await?;
    assert_eq!(attempt_count(pool, owner.job_id).await?, 1);
    assert!(
        store
            .visible_derived_memory(
                run.project_id,
                attempt.spec.input.target_employee_id,
                None,
                100
            )
            .await?
            .is_empty()
    );
    assert!(
        timeout(Duration::from_millis(150), supervisor.next_provision())
            .await
            .is_err()
    );

    // Hold the gate before acknowledging stop so the no-result case exposes a pending
    // retry, without trying to execute a provider in this keyless recovery fixture.
    let revision = store
        .load_project(run.project_id)
        .await?
        .context("Project")?
        .revision();
    core.execute_command(CommandEnvelope::parse(
        CommandName::StopProjectExecution,
        CommandRequest {
            project_id: run.project_id.to_string(),
            expected_revision: revision,
            payload: serde_json::Map::from_iter([(
                "reason".into(),
                json!("Keep the keyless restart fixture stopped after physical quiescence"),
            )]),
        },
        Uuid::now_v7().to_string(),
    )?)
    .await?;
    let ack = supervisor
        .send_observation_kind(run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    assert_eq!(
        ack.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "{}",
        ack.reason_code
    );
    core.system_job_tick().await?;
    let entries = store
        .visible_derived_memory(
            run.project_id,
            attempt.spec.input.target_employee_id,
            None,
            100,
        )
        .await?;
    assert_eq!(entries.len(), usize::from(accepted.is_some()));
    let mut tx = store.begin().await?;
    let finished = tx
        .system_job_attempt(run.project_id, owner.attempt_id)
        .await?
        .context("finished attempt")?;
    assert!(finished.physical_quiescent);
    assert_eq!(
        finished.state,
        if accepted.is_some() {
            "completed"
        } else {
            "failed"
        }
    );
    assert_eq!(finished.result, attempt.result);
    let job = tx
        .lock_system_job(run.project_id, owner.job_id)
        .await?
        .context("Job")?;
    assert_eq!(
        job.state,
        if accepted.is_some() {
            "completed"
        } else {
            "pending"
        }
    );
    assert_eq!(job.generation, owner.generation);
    tx.commit().await?;
    core.system_job_tick().await?;
    assert_eq!(
        store
            .visible_derived_memory(
                run.project_id,
                attempt.spec.input.target_employee_id,
                None,
                100
            )
            .await?,
        entries
    );
    assert_eq!(attempt_count(pool, owner.job_id).await?, 1);
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM event_log WHERE project_id=$1 AND event_type='derived_memory_changed'")
        .bind(run.project_id.as_uuid()).fetch_one(pool).await?;
    assert_eq!(events, i64::from(accepted.is_some()));
    Ok(())
}

async fn attempt_count(pool: &sqlx::PgPool, job_id: Uuid) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM system_job_attempts WHERE job_id=$1")
            .bind(job_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn wait_for_fenced_force(
    supervisor: &mut ManualSupervisor,
    run: &RunProjection,
) -> Result<()> {
    // Stop delivery is at-least-once: inventory/recovery may replay several
    // previously committed graceful messages before the new force request.
    timeout(Duration::from_secs(4), async {
        for _ in 0..16 {
            let stop = supervisor.next_stop_for_run(&run.id.to_string()).await?;
            assert_eq!(stop.run_id, run.id.to_string());
            assert_eq!(stop.lease_fencing_token, run.lease_fencing_token);
            assert_eq!(stop.environment_epoch, run.environment_epoch);
            if stop.mode == StopMode::Force as i32 {
                return Ok(());
            }
            assert_eq!(stop.mode, StopMode::Graceful as i32, "unknown stop mode");
        }
        anyhow::bail!("force-stop was not delivered within 16 fenced stop frames")
    })
    .await
    .context("wait for force-stop after durable Core restart fencing")?
}
