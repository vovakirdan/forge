//! A real authenticated UDS stream, without PostgreSQL or provider credentials.

use forge_protocol::supervisor::v1::{
    AcknowledgementDisposition, CoreAcknowledgement, CoreToSupervisor, RunEventKind,
    SupervisorToCore, core_to_supervisor,
    supervisor_control_server::{SupervisorControl, SupervisorControlServer},
    supervisor_to_core,
};
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use tokio::{
    net::UnixListener,
    sync::{mpsc, watch},
};
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
use tonic::{Request, Response, Status, Streaming};

use crate::{SupervisorConfig, new_id};

struct Session {
    inbound: Streaming<SupervisorToCore>,
    outbound: mpsc::Sender<Result<CoreToSupervisor, Status>>,
}

#[derive(Clone)]
struct CoreDouble {
    sessions: mpsc::Sender<Session>,
}

#[tonic::async_trait]
impl SupervisorControl for CoreDouble {
    type SupervisorSessionStream = ReceiverStream<Result<CoreToSupervisor, Status>>;

    async fn supervisor_session(
        &self,
        request: Request<Streaming<SupervisorToCore>>,
    ) -> Result<Response<Self::SupervisorSessionStream>, Status> {
        let (outbound, receiver) = mpsc::channel(16);
        self.sessions
            .send(Session {
                inbound: request.into_inner(),
                outbound,
            })
            .await
            .map_err(|_| Status::unavailable("test closed"))?;
        Ok(Response::new(ReceiverStream::new(receiver)))
    }
}

struct TestDirectory(PathBuf);
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn receive(session: &mut Session) -> SupervisorToCore {
    tokio::time::timeout(Duration::from_secs(5), session.inbound.message())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

async fn acknowledge(session: &Session, message: &SupervisorToCore, reason: &str) {
    session
        .outbound
        .send(Ok(CoreToSupervisor {
            message: Some(core_to_supervisor::Message::Acknowledgement(
                CoreAcknowledgement {
                    message_id: new_id(),
                    acknowledged_message_id: crate::journal::message_id(message).unwrap().clone(),
                    disposition: if reason.is_empty() {
                        AcknowledgementDisposition::Accepted
                    } else {
                        AcknowledgementDisposition::Rejected
                    } as i32,
                    reason_code: reason.into(),
                    message: String::new(),
                },
            )),
        }))
        .await
        .unwrap();
}

#[tokio::test]
async fn disconnect_replays_original_events_without_stopping_or_restarting_worker() {
    disconnect_scenario(false).await;
}

#[tokio::test]
async fn temporary_storage_rejection_replays_before_later_sequence() {
    disconnect_scenario(true).await;
}

async fn disconnect_scenario(reject_first: bool) {
    let directory = TestDirectory(std::env::temp_dir().join(format!("forge-uds-{}", new_id())));
    std::fs::create_dir(&directory.0).unwrap();
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = directory.0.join("core.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (sessions_tx, mut sessions) = mpsc::channel(4);
    let core_task = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(SupervisorControlServer::new(CoreDouble {
                sessions: sessions_tx,
            }))
            .serve_with_incoming(UnixListenerStream::new(listener)),
    );
    let mut config = SupervisorConfig::new(socket, "test-host".into(), "boot-1".into());
    config.reconnect_delay = Duration::from_millis(10);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let supervisor_task = tokio::spawn(crate::run(config, shutdown_rx));
    let mut first = tokio::time::timeout(Duration::from_secs(5), sessions.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        receive(&mut first).await.message,
        Some(supervisor_to_core::Message::Hello(_))
    ));
    let mut provision = super::tests::provision();
    provision.run_spec_json = r#"{"stage_outcome":{"outcome":"passed"},"delay_ms":30}"#.into();
    first
        .outbound
        .send(Ok(CoreToSupervisor {
            message: Some(core_to_supervisor::Message::ProvisionRun(provision.clone())),
        }))
        .await
        .unwrap();
    let first_event = receive(&mut first).await;
    assert!(
        matches!(&first_event.message, Some(supervisor_to_core::Message::ObservedRunEvent(event)) if event.kind == RunEventKind::Provisioning as i32)
    );
    if reject_first {
        assert!(
            tokio::time::timeout(Duration::from_millis(100), first.inbound.message())
                .await
                .is_err(),
            "N+1 must not overtake an unacknowledged N"
        );
        acknowledge(&first, &first_event, "canonical_storage_unavailable").await;
        assert!(
            tokio::time::timeout(Duration::from_secs(5), first.inbound.message())
                .await
                .unwrap()
                .unwrap()
                .is_none(),
            "retryable rejection must reconnect with backoff"
        );
    }
    drop(first);

    let mut second = tokio::time::timeout(Duration::from_secs(5), sessions.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        receive(&mut second).await.message,
        Some(supervisor_to_core::Message::Hello(_))
    ));
    provision.command_id = new_id();
    second
        .outbound
        .send(Ok(CoreToSupervisor {
            message: Some(core_to_supervisor::Message::ProvisionRun(provision)),
        }))
        .await
        .unwrap();
    let mut observed = Vec::new();
    loop {
        let message = receive(&mut second).await;
        let stopped = matches!(&message.message, Some(supervisor_to_core::Message::ObservedRunEvent(event)) if event.kind == RunEventKind::Stopped as i32);
        if crate::journal::message_id(&message).is_some() {
            acknowledge(&second, &message, "").await;
        }
        if !matches!(
            message.message,
            Some(supervisor_to_core::Message::Inventory(_))
        ) {
            observed.push(message);
        }
        if stopped {
            break;
        }
    }
    assert_eq!(observed[0], first_event);
    let kinds: Vec<_> = observed
        .iter()
        .filter_map(|message| match &message.message {
            Some(supervisor_to_core::Message::ObservedRunEvent(event)) => Some(event.kind),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            RunEventKind::Provisioning as i32,
            RunEventKind::Running as i32,
            RunEventKind::Stopped as i32
        ]
    );
    assert!(observed.iter().any(|message| matches!(
        message.message,
        Some(supervisor_to_core::Message::ExecutorSubmission(_))
    )));
    shutdown_tx.send(true).unwrap();
    supervisor_task.await.unwrap().unwrap();
    drop(second);
    core_task.abort();
    let _ = core_task.await;
}
