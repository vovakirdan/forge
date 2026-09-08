//! A real replacement Core may stop an old Hook, never replay or adopt its work.
use super::*;
use forge_core::{CoreActors, CoreService, SupervisorHub, SupervisorService};
use forge_domain::{ActorId, runtime::RunScope};
use forge_protocol::supervisor::v1::{
    EnvironmentPresence, RunInventoryEntry, supervisor_control_server::SupervisorControlServer,
};
use forge_testkit::m0::ManualSupervisor;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; two real Core instances, no provider or Hook process"]
async fn replacement_core_stops_retained_hook_without_credentials_or_replay() -> Result<()> {
    let mut f = Box::pin(support::setup("passed")).await?;
    let provision = f.supervisor.next_provision().await?;
    let run =
        f.h.store
            .load_run(provision.run_id.parse()?)
            .await?
            .context("Hook")?;
    f.supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Running)
        .await?;
    drop(f.supervisor);
    f.h.wait_for_supervisor_disconnected().await?;
    let store = f.h.store.clone();
    let pool = f.h.pool.clone();
    // Release the old Core and its exclusive evidence spool lock exactly as a
    // process restart does; retain its database and private execution directory.
    f.h.shutdown().await;
    // Same canonical store and same host boot, but a new management incarnation.
    // No SecretStore is installed: restoring provider resources would fail here.
    let core = CoreService::new(
        store.clone(),
        CoreActors::new(ActorId::new(), ActorId::new()),
        Arc::new(SupervisorHub::default()),
    )
    .with_execution_root(f.root.clone())?;
    let socket = f.root.join("replacement.sock");
    let listener = UnixListener::bind(&socket)?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(
        Server::builder()
            .add_service(SupervisorControlServer::new(SupervisorService::new(
                core.clone(),
            )))
            .serve_with_incoming_shutdown(UnixListenerStream::new(listener), async {
                let _ = shutdown_rx.await;
            }),
    );
    let result = exercise(&store, &pool, &core, &socket, &run).await;
    let _ = shutdown_tx.send(());
    server.await??;
    result
}

async fn exercise(
    store: &forge_storage::PostgresStore,
    pool: &sqlx::PgPool,
    core: &CoreService,
    socket: &std::path::Path,
    run: &forge_storage::RunProjection,
) -> Result<()> {
    let mut supervisor = ManualSupervisor::connect(socket).await?;
    supervisor
        .reconcile(vec![RunInventoryEntry {
            run_id: run.id.to_string(),
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
            last_sequence: 1,
            presence: EnvironmentPresence::Active as i32,
            environment_id: "retained-hook".into(),
            provision_boot_id: "m0-acceptance-boot".into(),
        }])
        .await?;
    let stop = supervisor.next_stop_for_run(&run.id.to_string()).await?;
    assert_eq!(stop.lease_fencing_token, run.lease_fencing_token);
    assert_eq!(stop.environment_epoch, run.environment_epoch);
    let mut tx = store.begin().await?;
    let state = tx
        .run_recovery_state(run.id)
        .await?
        .context("Hook reservation")?;
    assert!(!state.lease_active && state.reserved);
    assert!(
        tx.validate_gateway_scope(&RunScope {
            run_id: run.id,
            fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch
        })
        .await?
        .is_none()
    );
    tx.commit().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE hook_invocation_id=$1")
        .bind(run.assignment.hook().context("owner")?.invocation_id)
        .fetch_one(pool)
        .await?;
    assert_eq!(count, 1, "Core restart never creates another Hook attempt");
    supervisor.send_hook_result(run, 2, None).await?;
    let state: String = sqlx::query_scalar("SELECT state FROM hook_invocations WHERE run_id=$1")
        .bind(run.id)
        .fetch_one(pool)
        .await?;
    assert_eq!(state, "held");
    let core = core.clone();
    tokio::spawn(async move {
        core.watchdog_tick(
            forge_domain::Timestamp::now_utc(),
            forge_core::WatchdogDeadlines::default(),
        )
        .await
    })
    .await??;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE hook_invocation_id=$1")
        .bind(run.assignment.hook().context("owner")?.invocation_id)
        .fetch_one(pool)
        .await?;
    assert_eq!(count, 1);
    Ok(())
}
