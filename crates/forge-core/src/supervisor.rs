//! Core-owned control-channel registry and gRPC service for the local M0 Supervisor.

#[path = "supervisor/bridge.rs"]
pub(crate) mod bridge;
#[path = "supervisor/candidate_review.rs"]
mod candidate_review;
#[path = "supervisor/executor.rs"]
mod executor;
#[path = "supervisor/gateway.rs"]
mod gateway;
#[path = "supervisor/git_inspection.rs"]
mod git_inspection;
#[path = "supervisor/git_proposal.rs"]
mod git_proposal;
#[path = "supervisor/inventory.rs"]
mod inventory;
#[path = "supervisor/outcome.rs"]
pub(crate) mod outcome;
#[path = "supervisor/runtime_inputs.rs"]
mod runtime_inputs;
#[path = "supervisor/source.rs"]
mod source;
#[path = "supervisor/validation.rs"]
mod validation;

use std::pin::Pin;

use forge_domain::{Actor, ActorId};
use forge_protocol::supervisor::v1::{
    AcknowledgementDisposition, CoreAcknowledgement, CoreToSupervisor, SupervisorHello,
    SupervisorToCore, supervisor_control_server::SupervisorControl, supervisor_to_core,
};
use futures_util::Stream;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};
use tonic::{Request, Response, Status};
use tracing::warn;
use uuid::Uuid;

use crate::{CoreError, CoreService};

const OUTBOUND_CHANNEL_CAPACITY: usize = 64;

/// Authenticated identity of one local Supervisor process.
///
/// The Unix-socket listener owns peer credential verification. This identity
/// supplements it so Core can reject accidental stream replacement and retain
/// a stable audit actor for observations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SupervisorIdentity {
    instance_id: Uuid,
    pub(crate) host_id: String,
    pub(crate) boot_id: String,
}

impl SupervisorIdentity {
    fn from_hello(hello: &SupervisorHello) -> Result<Self, TransportRejection> {
        validation::validate_hello(hello)?;
        Ok(Self {
            instance_id: validation::parse_uuid_v7(
                "supervisor_instance_id",
                &hello.supervisor_instance_id,
            )?,
            host_id: hello.host_id.clone(),
            boot_id: hello.boot_id.clone(),
        })
    }

    pub(crate) fn actor(&self) -> Actor {
        Actor::supervisor(ActorId::from(self.instance_id))
    }

    pub(crate) fn instance_id(&self) -> Uuid {
        self.instance_id
    }
}

/// Safe, machine-readable refusal issued to the Supervisor transport.
#[derive(Clone, Debug)]
pub(crate) struct TransportRejection {
    reason_code: &'static str,
    message: &'static str,
}

impl TransportRejection {
    pub(crate) const fn new(reason_code: &'static str, message: &'static str) -> Self {
        Self {
            reason_code,
            message,
        }
    }

    fn as_status(&self) -> Status {
        Status::invalid_argument(self.message)
    }
}

#[derive(Debug)]
struct ActiveSupervisor {
    identity: SupervisorIdentity,
    sender: mpsc::Sender<CoreToSupervisor>,
    inventory_request_id: Option<String>,
    reconciled: bool,
}

/// A registered outbound stream. Its receiver belongs to tonic and its sender
/// is retained by both the hub and the inbound acknowledgement loop.
#[derive(Debug)]
pub(crate) struct SupervisorAttachment {
    sender: mpsc::Sender<CoreToSupervisor>,
    receiver: mpsc::Receiver<CoreToSupervisor>,
}

/// Refusal returned when another local Supervisor still owns the one M0 slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupervisorAttachError {
    /// The same process attempted a second active stream.
    DuplicateInstance,
    /// A different process attempted to silently replace the active stream.
    AnotherInstance,
}

impl SupervisorAttachError {
    fn status(self) -> Status {
        match self {
            Self::DuplicateInstance => {
                Status::already_exists("this Supervisor instance already has an active stream")
            }
            Self::AnotherInstance => {
                Status::already_exists("another local Supervisor already has an active stream")
            }
        }
    }
}

/// Process-local connection registry; it never owns canonical state.
#[derive(Debug)]
pub struct SupervisorHub {
    active: Mutex<Option<ActiveSupervisor>>,
    created_at: std::time::Instant,
}

impl Default for SupervisorHub {
    fn default() -> Self {
        Self {
            active: Mutex::new(None),
            created_at: std::time::Instant::now(),
        }
    }
}

impl SupervisorHub {
    pub(crate) async fn reconciled_identity(&self) -> Option<SupervisorIdentity> {
        self.active
            .lock()
            .await
            .as_ref()
            .filter(|active| active.reconciled)
            .map(|active| active.identity.clone())
    }
    pub(crate) fn startup_reconciliation_grace(&self, seconds: u32) -> bool {
        self.created_at.elapsed() < std::time::Duration::from_secs(u64::from(seconds))
    }
    /// Registers the sole connected local Supervisor without replacing another stream.
    pub(crate) async fn attach(
        &self,
        identity: SupervisorIdentity,
    ) -> Result<SupervisorAttachment, SupervisorAttachError> {
        let mut active = self.active.lock().await;
        if let Some(current) = active.as_ref() {
            return Err(if current.identity == identity {
                SupervisorAttachError::DuplicateInstance
            } else {
                SupervisorAttachError::AnotherInstance
            });
        }
        let (sender, receiver) = mpsc::channel(OUTBOUND_CHANNEL_CAPACITY);
        *active = Some(ActiveSupervisor {
            identity,
            sender: sender.clone(),
            inventory_request_id: None,
            reconciled: false,
        });
        Ok(SupervisorAttachment { sender, receiver })
    }

    /// Clears the sender only when the stream that ended still owns the slot.
    pub(crate) async fn detach_if_current(
        &self,
        identity: &SupervisorIdentity,
        sender: &mpsc::Sender<CoreToSupervisor>,
    ) {
        let mut active = self.active.lock().await;
        if active.as_ref().is_some_and(|current| {
            current.identity == *identity && current.sender.same_channel(sender)
        }) {
            *active = None;
        }
    }

    /// Returns whether a local Supervisor can receive a committed provision request.
    pub async fn is_connected(&self) -> bool {
        self.active.lock().await.is_some()
    }

    /// Real dispatch is gated until this exact Supervisor has answered inventory.
    pub async fn is_reconciled(&self) -> bool {
        self.active
            .lock()
            .await
            .as_ref()
            .is_some_and(|active| active.reconciled)
    }

    async fn request_inventory(&self) -> Result<(), CoreError> {
        let command_id = Uuid::now_v7().to_string();
        if let Some(active) = self.active.lock().await.as_mut() {
            active.inventory_request_id = Some(command_id.clone());
        }
        self.send(CoreToSupervisor {
            message: Some(
                forge_protocol::supervisor::v1::core_to_supervisor::Message::RequestInventory(
                    forge_protocol::supervisor::v1::RequestInventory { command_id },
                ),
            ),
        })
        .await
    }

    async fn inventory_is_requested(
        &self,
        identity: &SupervisorIdentity,
        command_id: &str,
    ) -> bool {
        self.active.lock().await.as_ref().is_some_and(|active| {
            active.identity == *identity
                && !active.reconciled
                && active.inventory_request_id.as_deref() == Some(command_id)
        })
    }

    async fn accept_inventory(&self, identity: &SupervisorIdentity, command_id: &str) -> bool {
        if let Some(active) = self.active.lock().await.as_mut()
            && active.identity == *identity
            && active.inventory_request_id.as_deref() == Some(command_id)
            && !active.reconciled
        {
            active.reconciled = true;
            active.inventory_request_id = None;
            return true;
        }
        false
    }

    /// Delivers a desired-state request after its canonical transaction commits.
    pub async fn send(&self, message: CoreToSupervisor) -> Result<(), CoreError> {
        let sender = self
            .active
            .lock()
            .await
            .as_ref()
            .map(|current| current.sender.clone());
        let Some(sender) = sender else {
            return Err(CoreError::SupervisorUnavailable);
        };
        if sender.send(message).await.is_ok() {
            return Ok(());
        }
        self.detach_sender(&sender).await;
        Err(CoreError::SupervisorUnavailable)
    }

    async fn detach_sender(&self, sender: &mpsc::Sender<CoreToSupervisor>) {
        let mut active = self.active.lock().await;
        if active
            .as_ref()
            .is_some_and(|current| current.sender.same_channel(sender))
        {
            *active = None;
        }
    }
}

/// gRPC façade over a [`CoreService`].
///
/// The service implementation is intentionally kept separate from the hub so
/// transport connections cannot write canonical state without Core validation.
#[derive(Clone)]
pub struct SupervisorService {
    pub(crate) core: CoreService,
}

impl SupervisorService {
    /// Creates the authenticated local supervisor transport façade.
    #[must_use]
    pub fn new(core: CoreService) -> Self {
        Self { core }
    }
}

#[tonic::async_trait]
impl SupervisorControl for SupervisorService {
    type SupervisorSessionStream =
        Pin<Box<dyn Stream<Item = Result<CoreToSupervisor, Status>> + Send>>;

    async fn supervisor_session(
        &self,
        request: Request<tonic::Streaming<SupervisorToCore>>,
    ) -> Result<Response<Self::SupervisorSessionStream>, Status> {
        let mut inbound = request.into_inner();
        let first = inbound
            .message()
            .await?
            .ok_or_else(|| Status::failed_precondition("Supervisor stream ended before Hello"))?;
        let Some(supervisor_to_core::Message::Hello(hello)) = first.message else {
            return Err(Status::failed_precondition(
                "SupervisorHello must be the first stream message",
            ));
        };
        let identity = SupervisorIdentity::from_hello(&hello).map_err(|error| error.as_status())?;
        let attachment = self
            .core
            .supervisor
            .attach(identity.clone())
            .await
            .map_err(SupervisorAttachError::status)?;
        let sender = attachment.sender.clone();
        let receiver = attachment.receiver;
        sender
            .send(acknowledgement(
                &hello.message_id,
                AcknowledgementDisposition::Accepted,
                "",
                "Supervisor session accepted",
            ))
            .await
            .map_err(|_| Status::unavailable("Supervisor response stream closed"))?;

        let core = self.core.clone();
        tokio::spawn(async move {
            if core.supervisor.request_inventory().await.is_err() {
                warn!("could not request Supervisor inventory");
            }
            // Return the response before recovery may need outbound channel
            // capacity. The fake Supervisor can then receive replayed desired
            // messages while this task preserves inbound message ordering.
            if core.recover_after_supervisor_attach().await.is_err() {
                warn!("Supervisor reconnect recovery did not complete");
            }
            while let Ok(Some(message)) = inbound.message().await {
                let acknowledgement = core.handle_supervisor_message(&identity, message).await;
                if sender.send(acknowledgement).await.is_err() {
                    break;
                }
            }
            core.supervisor.detach_if_current(&identity, &sender).await;
        });

        Ok(Response::new(Box::pin(
            ReceiverStream::new(receiver).map(Ok::<CoreToSupervisor, Status>),
        )))
    }
}

pub(crate) fn acknowledgement(
    acknowledged_message_id: &str,
    disposition: AcknowledgementDisposition,
    reason_code: &str,
    message: &str,
) -> CoreToSupervisor {
    CoreToSupervisor {
        message: Some(
            forge_protocol::supervisor::v1::core_to_supervisor::Message::Acknowledgement(
                CoreAcknowledgement {
                    message_id: Uuid::now_v7().to_string(),
                    acknowledged_message_id: validation::acknowledged_message_id(
                        acknowledged_message_id,
                    ),
                    disposition: disposition as i32,
                    reason_code: reason_code.to_owned(),
                    message: message.to_owned(),
                },
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use forge_protocol::supervisor::v1::CoreToSupervisor;
    use uuid::Uuid;

    use super::{SupervisorAttachError, SupervisorHub, SupervisorIdentity};

    fn identity() -> SupervisorIdentity {
        SupervisorIdentity {
            instance_id: Uuid::now_v7(),
            host_id: "local-host".to_owned(),
            boot_id: "local-boot".to_owned(),
        }
    }

    #[tokio::test]
    async fn requested_inventory_completes_reconciliation_only_once() {
        let hub = SupervisorHub::default();
        let identity = identity();
        let mut attachment = hub.attach(identity.clone()).await.expect("attach");
        hub.request_inventory().await.expect("request");
        let message = attachment.receiver.recv().await.expect("request message");
        let Some(forge_protocol::supervisor::v1::core_to_supervisor::Message::RequestInventory(
            request,
        )) = message.message
        else {
            panic!("expected inventory request")
        };
        assert!(!hub.accept_inventory(&identity, "unsolicited").await);
        assert!(hub.accept_inventory(&identity, &request.command_id).await);
        assert!(!hub.accept_inventory(&identity, &request.command_id).await);
    }

    #[tokio::test]
    async fn hub_rejects_silent_replacement_and_retains_first_stream() {
        let hub = SupervisorHub::default();
        let mut first = hub.attach(identity()).await.expect("first attachment");
        let refusal = hub.attach(identity()).await.expect_err("second attachment");

        hub.send(CoreToSupervisor::default())
            .await
            .expect("first stream remains connected");

        assert_eq!(refusal, SupervisorAttachError::AnotherInstance);
        assert!(first.receiver.try_recv().is_ok());
    }
}
