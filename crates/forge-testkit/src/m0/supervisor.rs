use std::{collections::VecDeque, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use forge_domain::TaskId;
use forge_protocol::supervisor::v1::{
    ArtifactSubmission, CoreAcknowledgement, CoreToSupervisor, ExecutorSubmission,
    ObservedRunEvent, ProvisionRun, RunEventKind, StageOutcomeSubmission, StopRun, SupervisorHello,
    SupervisorToCore, core_to_supervisor, executor_submission,
    supervisor_control_client::SupervisorControlClient, supervisor_to_core,
};
use forge_storage::RunProjection;
use hyper_util::rt::TokioIo;
use serde_json::json;
use tokio::{net::UnixStream, sync::mpsc, time::timeout};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;
use uuid::Uuid;

const WAIT_TIMEOUT: Duration = Duration::from_secs(8);

/// A test-controlled real gRPC Supervisor stream.
pub struct ManualSupervisor {
    outbound: mpsc::Sender<SupervisorToCore>,
    inbound: tonic::Streaming<CoreToSupervisor>,
    deferred: VecDeque<CoreToSupervisor>,
}

impl ManualSupervisor {
    /// Connects with a valid Hello and waits for Core's accepted acknowledgement.
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let channel = uds_channel(socket_path).await?;
        let (outbound, receiver) = mpsc::channel(16);
        let hello_id = Uuid::now_v7().to_string();
        outbound
            .send(SupervisorToCore {
                message: Some(supervisor_to_core::Message::Hello(SupervisorHello {
                    message_id: hello_id.clone(),
                    supervisor_instance_id: Uuid::now_v7().to_string(),
                    host_id: "m0-acceptance-host".to_owned(),
                    boot_id: "m0-acceptance-boot".to_owned(),
                    protocol_major: 1,
                    started_at_unix_ms: 0,
                })),
            })
            .await
            .context("send manual Supervisor Hello")?;
        let response = SupervisorControlClient::new(channel)
            .supervisor_session(ReceiverStream::new(receiver))
            .await
            .context("open manual Supervisor session")?;
        let mut supervisor = Self {
            outbound,
            inbound: response.into_inner(),
            deferred: VecDeque::new(),
        };
        let acknowledgement = supervisor.wait_for_acknowledgement(&hello_id).await?;
        if acknowledgement.disposition
            != forge_protocol::supervisor::v1::AcknowledgementDisposition::Accepted as i32
        {
            bail!("Core rejected manual Supervisor Hello")
        }
        Ok(supervisor)
    }

    /// Receives the next durable provision request while intentionally doing no work.
    pub async fn next_provision(&mut self) -> Result<ProvisionRun> {
        self.next_matching("ProvisionRun", |message| match message.message.as_ref() {
            Some(core_to_supervisor::Message::ProvisionRun(provision)) => Some(provision.clone()),
            _ => None,
        })
        .await
    }

    /// Receives the next provision for one fixture Task, skipping recovery work.
    pub async fn next_provision_for_task(&mut self, task_id: TaskId) -> Result<ProvisionRun> {
        let task_id = task_id.as_uuid().to_string();
        self.next_matching("fixture ProvisionRun", move |message| {
            match message.message.as_ref() {
                Some(core_to_supervisor::Message::ProvisionRun(provision))
                    if provision.task_id == task_id =>
                {
                    Some(provision.clone())
                }
                _ => None,
            }
        })
        .await
    }

    /// Receives the next requested stop for a held Run.
    pub async fn next_stop(&mut self) -> Result<StopRun> {
        self.next_matching("StopRun", |message| match message.message.as_ref() {
            Some(core_to_supervisor::Message::StopRun(stop)) => Some(stop.clone()),
            _ => None,
        })
        .await
    }

    /// Receives the requested stop for one fixture Run, skipping recovery work.
    pub async fn next_stop_for_run(&mut self, run_id: &str) -> Result<StopRun> {
        self.next_matching("fixture StopRun", |message| {
            match message.message.as_ref() {
                Some(core_to_supervisor::Message::StopRun(stop)) if stop.run_id == run_id => {
                    Some(stop.clone())
                }
                _ => None,
            }
        })
        .await
    }

    /// Sends a fenced observation and returns Core's corresponding acknowledgement.
    pub async fn send_observation(
        &mut self,
        run: &RunProjection,
        lease_fencing_token: u64,
        sequence: u64,
    ) -> Result<CoreAcknowledgement> {
        self.send_observation_kind(run, lease_fencing_token, sequence, RunEventKind::Running)
            .await
    }

    /// Sends one selected runtime observation and returns Core's acknowledgement.
    pub async fn send_observation_kind(
        &mut self,
        run: &RunProjection,
        lease_fencing_token: u64,
        sequence: u64,
        kind: RunEventKind,
    ) -> Result<CoreAcknowledgement> {
        let message_id = Uuid::now_v7().to_string();
        self.outbound
            .send(SupervisorToCore {
                message: Some(supervisor_to_core::Message::ObservedRunEvent(
                    ObservedRunEvent {
                        message_id: message_id.clone(),
                        run_id: run.id.to_string(),
                        lease_fencing_token,
                        environment_epoch: run.environment_epoch,
                        sequence,
                        occurred_at_unix_ms: 0,
                        kind: kind as i32,
                        details_json: "{}".to_owned(),
                    },
                )),
            })
            .await
            .context("send manual Supervisor observation")?;
        self.wait_for_acknowledgement(&message_id).await
    }

    /// Submits one valid `stage_evidence` Artifact for the current stage.
    pub async fn submit_stage_evidence(
        &mut self,
        run: &RunProjection,
        sequence: u64,
    ) -> Result<(String, CoreAcknowledgement)> {
        self.send_executor_submission(
            run,
            sequence,
            executor_submission::Payload::Artifact(ArtifactSubmission {
                artifact_kind: "stage_evidence".to_owned(),
                metadata_json: json!({"source": "m0_acceptance"}).to_string(),
                body_json: json!({"stage_id": run.stage_id}).to_string(),
                title: format!("M0 {} evidence", run.stage_id),
            }),
        )
        .await
    }

    /// Submits one explicit Pipeline outcome citing the current Run's evidence.
    pub async fn submit_stage_outcome(
        &mut self,
        run: &RunProjection,
        sequence: u64,
        outcome: &str,
        artifact_submission_message_ids: Vec<String>,
    ) -> Result<CoreAcknowledgement> {
        let (_, acknowledgement) = self
            .send_executor_submission(
                run,
                sequence,
                executor_submission::Payload::StageOutcome(StageOutcomeSubmission {
                    stage_id: run.stage_id.clone(),
                    outcome: outcome.to_owned(),
                    artifact_ids: Vec::new(),
                    note: "m0 acceptance".to_owned(),
                    cancellation_reason_key: String::new(),
                    artifact_submission_message_ids,
                }),
            )
            .await?;
        Ok(acknowledgement)
    }

    async fn wait_for_acknowledgement(&mut self, message_id: &str) -> Result<CoreAcknowledgement> {
        self.next_matching("Core acknowledgement", |message| {
            match message.message.as_ref() {
                Some(core_to_supervisor::Message::Acknowledgement(acknowledgement))
                    if acknowledgement.acknowledged_message_id == message_id =>
                {
                    Some(acknowledgement.clone())
                }
                _ => None,
            }
        })
        .await
    }

    async fn send_executor_submission(
        &mut self,
        run: &RunProjection,
        sequence: u64,
        payload: executor_submission::Payload,
    ) -> Result<(String, CoreAcknowledgement)> {
        let message_id = Uuid::now_v7().to_string();
        self.outbound
            .send(SupervisorToCore {
                message: Some(supervisor_to_core::Message::ExecutorSubmission(
                    ExecutorSubmission {
                        message_id: message_id.clone(),
                        run_id: run.id.to_string(),
                        lease_fencing_token: run.lease_fencing_token,
                        environment_epoch: run.environment_epoch,
                        sequence,
                        submitted_at_unix_ms: 0,
                        payload: Some(payload),
                    },
                )),
            })
            .await
            .context("send manual executor submission")?;
        let acknowledgement = self.wait_for_acknowledgement(&message_id).await?;
        Ok((message_id, acknowledgement))
    }

    async fn next_matching<T>(
        &mut self,
        description: &str,
        mut select: impl FnMut(&CoreToSupervisor) -> Option<T>,
    ) -> Result<T> {
        timeout(WAIT_TIMEOUT, async {
            loop {
                if let Some(position) = self
                    .deferred
                    .iter()
                    .position(|message| select(message).is_some())
                {
                    let message = self
                        .deferred
                        .remove(position)
                        .expect("deferred Supervisor message position is valid");
                    return Ok(select(&message).expect("selected deferred message still matches"));
                }
                let message = self
                    .inbound
                    .message()
                    .await
                    .context("read Core Supervisor message")?
                    .context("Core closed manual Supervisor stream")?;
                if let Some(value) = select(&message) {
                    return Ok(value);
                }
                // Core may dispatch a replacement Run before acknowledging the
                // terminal observation that released capacity. Preserve every
                // nonmatching frame so an acknowledgement wait cannot erase
                // real Supervisor work from the acceptance scenario.
                self.deferred.push_back(message);
            }
        })
        .await
        .with_context(|| format!("wait for {description}"))?
    }
}

async fn uds_channel(socket_path: &Path) -> Result<Channel> {
    let socket_path = socket_path.to_owned();
    Endpoint::from_static("http://[::]:50051")
        .connect_with_connector(service_fn(move |_| {
            let socket_path = socket_path.clone();
            async move {
                let stream = UnixStream::connect(socket_path).await?;
                Ok::<_, std::io::Error>(TokioIo::new(stream))
            }
        }))
        .await
        .context("connect manual Supervisor UDS channel")
}
