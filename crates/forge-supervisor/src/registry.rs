//! Process-lifetime execution controls, independent of the current Core stream.

use crate::{
    SupervisorError,
    journal::{Journal, scope_key},
};
use forge_protocol::supervisor::v1::{
    ObservedRunEvent, ProvisionRun, RunEventKind, StopMode, StopRun, SupervisorToCore,
    supervisor_to_core,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex, Notify};

#[derive(Clone)]
pub(crate) struct RunRegistry {
    active: Arc<Mutex<HashMap<String, Arc<RunControl>>>>,
    pub journal: Arc<Mutex<Journal>>,
    pub changed: Arc<Notify>,
    pub connected: Arc<AtomicBool>,
}

impl RunRegistry {
    pub fn new(journal: Journal) -> Self {
        Self {
            active: Arc::default(),
            journal: Arc::new(Mutex::new(journal)),
            changed: Arc::default(),
            connected: Arc::default(),
        }
    }

    pub async fn register(
        &self,
        provision: &ProvisionRun,
        boot_id: &str,
    ) -> Result<Option<Arc<RunControl>>, SupervisorError> {
        if !self.journal.lock().await.register(provision, boot_id)? {
            return Ok(None);
        }
        let mut control = RunControl::new(
            provision.run_id.clone(),
            provision.lease_fencing_token,
            provision.environment_epoch,
        );
        control.detachable = provision.run_spec_version == 2;
        let control = Arc::new(control);
        self.active
            .lock()
            .await
            .insert(scope_key(provision), Arc::clone(&control));
        Ok(Some(control))
    }

    pub async fn request_stop(&self, request: &StopRun) -> bool {
        let key = format!(
            "{}:{}:{}",
            request.run_id, request.lease_fencing_token, request.environment_epoch
        );
        if let Some(control) = self.active.lock().await.get(&key) {
            let Ok(mode @ (StopMode::Graceful | StopMode::Force)) =
                StopMode::try_from(request.mode)
            else {
                return false;
            };
            control.request_stop_mode(mode, request.grace_period_ms);
            true
        } else {
            false
        }
    }

    pub async fn request_stop_all(&self) {
        for run in self.active.lock().await.values() {
            if !run.detachable {
                run.request_stop();
            }
        }
    }

    pub async fn finish(
        &self,
        provision: &ProvisionRun,
        quiescent: bool,
    ) -> Result<(), SupervisorError> {
        self.journal.lock().await.finish(provision, quiescent)?;
        self.active.lock().await.remove(&scope_key(provision));
        self.changed.notify_one();
        Ok(())
    }

    /// Uncertainty never retires the live control or its observation worker.
    pub async fn mark_unknown(&self, provision: &ProvisionRun) -> Result<(), SupervisorError> {
        self.journal.lock().await.finish(provision, false)?;
        self.changed.notify_one();
        Ok(())
    }

    pub fn sink(&self) -> EventSink {
        EventSink::Durable {
            journal: Arc::clone(&self.journal),
            changed: Arc::clone(&self.changed),
        }
    }

    pub async fn attach_environment(
        &self,
        provision: &ProvisionRun,
        id: &str,
        active: bool,
    ) -> Result<(), SupervisorError> {
        self.journal
            .lock()
            .await
            .attach_environment(provision, id, active)
    }

    pub async fn adopt_control(&self, provision: &ProvisionRun) -> Arc<RunControl> {
        let mut control = RunControl::new(
            provision.run_id.clone(),
            provision.lease_fencing_token,
            provision.environment_epoch,
        );
        control.detachable = true;
        let control = Arc::new(control);
        self.active
            .lock()
            .await
            .insert(scope_key(provision), Arc::clone(&control));
        control
    }

    pub async fn emit(
        &self,
        provision: &ProvisionRun,
        kind: RunEventKind,
        mut details: serde_json::Value,
    ) -> Result<(), SupervisorError> {
        let mut journal = self.journal.lock().await;
        let sequence = journal.next_sequence(provision)?;
        if let Some(context) =
            forge_protocol::trace_context::TraceContext::parse(&provision.traceparent)
        {
            if let Some(details) = details.as_object_mut() {
                details.insert(
                    "traceparent".into(),
                    serde_json::Value::String(context.header()),
                );
            }
            tracing::info!(component="supervisor", trace_id=%context.trace_id(), span_id=%context.span_id(),
                run_id=%provision.run_id, environment_epoch=provision.environment_epoch,
                kind=?kind, sequence, "physical Run observation");
        }
        journal.append(SupervisorToCore {
            message: Some(supervisor_to_core::Message::ObservedRunEvent(
                ObservedRunEvent {
                    message_id: crate::new_id(),
                    run_id: provision.run_id.clone(),
                    lease_fencing_token: provision.lease_fencing_token,
                    environment_epoch: provision.environment_epoch,
                    sequence,
                    occurred_at_unix_ms: crate::now_millis(),
                    kind: kind as i32,
                    details_json: serde_json::to_string(&details)?,
                },
            )),
        })?;
        self.changed.notify_one();
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) enum EventSink {
    Durable {
        journal: Arc<Mutex<Journal>>,
        changed: Arc<Notify>,
    },
    #[cfg(test)]
    Channel(tokio::sync::mpsc::Sender<SupervisorToCore>),
}

impl EventSink {
    pub async fn send(&self, message: SupervisorToCore) -> Result<(), SupervisorError> {
        match self {
            Self::Durable { journal, changed } => {
                journal.lock().await.append(message)?;
                changed.notify_one();
                Ok(())
            }
            #[cfg(test)]
            Self::Channel(sender) => sender
                .send(message)
                .await
                .map_err(|_| SupervisorError::CoreStreamClosed),
        }
    }
}

#[cfg(test)]
impl From<tokio::sync::mpsc::Sender<SupervisorToCore>> for EventSink {
    fn from(value: tokio::sync::mpsc::Sender<SupervisorToCore>) -> Self {
        Self::Channel(value)
    }
}

pub(crate) struct RunControl {
    stopped: AtomicBool,
    stop_notification: Notify,
    stop_mode: std::sync::atomic::AtomicU8,
    grace_ms: std::sync::atomic::AtomicU64,
    detachable: bool,
}

impl RunControl {
    // Scope belongs to the registry key. Standalone adapter tests use this
    // constructor without installing a registry entry.
    pub(crate) fn new(_run_id: String, _fence: u64, _epoch: u64) -> Self {
        Self {
            stopped: AtomicBool::new(false),
            stop_notification: Notify::new(),
            stop_mode: std::sync::atomic::AtomicU8::new(0),
            grace_ms: std::sync::atomic::AtomicU64::new(2000),
            detachable: false,
        }
    }
    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(crate) fn request_stop(&self) {
        self.request_stop_mode(StopMode::Graceful, 2000);
    }
    fn request_stop_mode(&self, mode: StopMode, grace_ms: u64) {
        self.grace_ms.store(grace_ms, Ordering::Release);
        self.stop_mode.fetch_max(mode as u8, Ordering::AcqRel);
        self.stopped.store(true, Ordering::Release);
        self.stop_notification.notify_one();
    }
    pub(crate) fn stop_request(&self) -> Option<(StopMode, std::time::Duration)> {
        if !self.stopped() {
            return None;
        }
        let mode = StopMode::try_from(i32::from(self.stop_mode.load(Ordering::Acquire))).ok()?;
        Some((
            mode,
            std::time::Duration::from_millis(self.grace_ms.load(Ordering::Acquire)),
        ))
    }
    pub(crate) async fn stop_notified(&self) {
        self.stop_notification.notified().await;
    }
}
