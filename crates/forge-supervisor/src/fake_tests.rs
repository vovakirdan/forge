use std::{sync::Arc, time::Duration};

use forge_protocol::supervisor::v1::{
    ExecutorSubmission, ObservedRunEvent, ProvisionRun, RunEventKind, executor_submission,
    supervisor_to_core,
};
use forge_protocol::wire::{
    ArtifactMetadataError, MAX_ARTIFACT_METADATA_SERIALIZED_BYTES, MAX_INLINE_ARTIFACT_BODY_BYTES,
};
use serde_json::json;
use tokio::sync::mpsc;

use super::*;

#[test]
fn fake_spec_parses_valid_version_one_document() {
    let spec = FakeRunSpec::parse(
        1,
        r#"{
            "artifacts": [{"kind":"change_set","title":"Change","metadata":{},"body":{"files":1}}],
            "stage_outcome":{"outcome":"implemented","note":"done"},
            "delay_ms": 7
        }"#,
    )
    .unwrap();

    assert_eq!(spec.artifacts.len(), 1);
    assert_eq!(spec.stage_outcome.outcome, "implemented");
    assert_eq!(spec.delay, Duration::from_millis(7));
}

#[test]
fn fake_spec_rejects_unsupported_version_before_parsing_json() {
    let error = FakeRunSpec::parse(2, "not json").unwrap_err();

    assert!(matches!(
        error,
        FakeRunSpecError::UnsupportedVersion { received: 2 }
    ));
}

#[test]
fn fake_spec_rejects_non_object_artifact_metadata() {
    let error = FakeRunSpec::parse(
        1,
        r#"{"artifacts":[{"kind":"report","title":"Result","metadata":[],"body":null}],"stage_outcome":{"outcome":"passed"}}"#,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        FakeRunSpecError::MetadataMustBeObject { index: 0 }
    ));
}

#[test]
fn fake_spec_rejects_oversized_artifact_metadata() {
    let run_spec = json!({
        "artifacts": [{
            "kind": "report",
            "title": "Result",
            "metadata": {"details": "x".repeat(MAX_ARTIFACT_METADATA_SERIALIZED_BYTES)},
            "body": null
        }],
        "stage_outcome": {"outcome": "passed"}
    });

    let error = FakeRunSpec::parse(1, &run_spec.to_string()).unwrap_err();

    assert!(matches!(
        error,
        FakeRunSpecError::InvalidArtifactMetadata {
            index: 0,
            source: ArtifactMetadataError::TooLarge { .. }
        }
    ));
}

#[test]
fn fake_spec_rejects_oversized_artifact_body() {
    let run_spec = json!({
        "artifacts": [{
            "kind": "report",
            "title": "Result",
            "metadata": {},
            "body": "x".repeat(MAX_INLINE_ARTIFACT_BODY_BYTES)
        }],
        "stage_outcome": {"outcome": "passed"}
    });

    let error = FakeRunSpec::parse(1, &run_spec.to_string()).unwrap_err();

    assert!(matches!(
        error,
        FakeRunSpecError::ArtifactBodyTooLarge { index: 0 }
    ));
}

#[test]
fn fake_spec_rejects_duplicate_task_history_artifact_ids() {
    let error = FakeRunSpec::parse(
        1,
        r#"{
            "stage_outcome": {
                "outcome":"passed",
                "artifact_ids":["018f8b5e-0e6d-7b47-8ad2-3b66d5ddd3b2", "018f8b5e-0e6d-7b47-8ad2-3b66d5ddd3b2"]
            }
        }"#,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        FakeRunSpecError::DuplicateTaskHistoryArtifactIds
    ));
}

#[tokio::test]
async fn fake_run_preserves_fence_epoch_and_message_order() {
    let provision = ProvisionRun {
        command_id: "command-1".into(),
        run_id: "run-1".into(),
        task_id: "task-1".into(),
        employee_id: "employee-1".into(),
        stage_id: "work".into(),
        attempt: 1,
        lease_fencing_token: 42,
        environment_epoch: 7,
        context_snapshot_id: "snapshot-1".into(),
        run_spec_json: r#"{
            "artifacts":[
                {"kind":"change_set","title":"Change","metadata":{},"body":{}},
                {"kind":"verification","title":"Verification","metadata":{},"body":{}}
            ],
            "stage_outcome":{
                "outcome":"implemented",
                "artifact_ids":["018f8b5e-0e6d-7b47-8ad2-3b66d5ddd3b2"]
            }
        }"#
        .into(),
        run_spec_version: 1,
        traceparent: String::new(),
    };
    let control = Arc::new(RunControl::new(
        provision.run_id.clone(),
        provision.lease_fencing_token,
        provision.environment_epoch,
    ));
    let (sender, mut receiver) = mpsc::channel(8);

    execute_provision(provision.clone(), sender.into(), control)
        .await
        .unwrap();
    let mut messages = Vec::new();
    while let Some(message) = receiver.recv().await {
        messages.push(message);
    }

    assert_eq!(messages.len(), 6);
    let sequences: Vec<u64> = messages
        .iter()
        .map(|message| match message.message.as_ref().unwrap() {
            supervisor_to_core::Message::ObservedRunEvent(event) => {
                assert_eq!(event.run_id, provision.run_id);
                assert_eq!(event.lease_fencing_token, 42);
                assert_eq!(event.environment_epoch, 7);
                event.sequence
            }
            supervisor_to_core::Message::ExecutorSubmission(submission) => {
                assert_eq!(submission.run_id, provision.run_id);
                assert_eq!(submission.lease_fencing_token, 42);
                assert_eq!(submission.environment_epoch, 7);
                submission.sequence
            }
            _ => panic!("fake run must emit only run-scoped messages"),
        })
        .collect();
    assert_eq!(sequences, vec![1, 2, 3, 4, 5, 6]);
    assert!(matches!(
        messages[0].message.as_ref().unwrap(),
        supervisor_to_core::Message::ObservedRunEvent(ObservedRunEvent { kind, .. })
            if *kind == RunEventKind::Provisioning as i32
    ));
    assert!(matches!(
        messages[5].message.as_ref().unwrap(),
        supervisor_to_core::Message::ObservedRunEvent(ObservedRunEvent { kind, .. })
            if *kind == RunEventKind::Stopped as i32
    ));
    let artifact_submission_message_ids = messages[2..4]
        .iter()
        .map(|message| match message.message.as_ref().unwrap() {
            supervisor_to_core::Message::ExecutorSubmission(ExecutorSubmission {
                message_id,
                payload: Some(executor_submission::Payload::Artifact(_)),
                ..
            }) => message_id.clone(),
            _ => panic!("expected an ArtifactSubmission envelope"),
        })
        .collect::<Vec<_>>();
    let outcome = match messages[4].message.as_ref().unwrap() {
        supervisor_to_core::Message::ExecutorSubmission(ExecutorSubmission {
            payload: Some(executor_submission::Payload::StageOutcome(outcome)),
            ..
        }) => outcome,
        _ => panic!("expected a StageOutcomeSubmission envelope"),
    };
    assert_eq!(
        outcome.artifact_submission_message_ids,
        artifact_submission_message_ids
    );
    assert_eq!(
        outcome.artifact_ids,
        vec!["018f8b5e-0e6d-7b47-8ad2-3b66d5ddd3b2"]
    );
}

#[tokio::test]
async fn invalid_spec_emits_one_terminal_failed_observation() {
    let provision = ProvisionRun {
        command_id: "command-1".into(),
        run_id: "run-1".into(),
        task_id: "task-1".into(),
        employee_id: "employee-1".into(),
        stage_id: "work".into(),
        attempt: 1,
        lease_fencing_token: 42,
        environment_epoch: 7,
        context_snapshot_id: "snapshot-1".into(),
        run_spec_json: "not JSON".into(),
        run_spec_version: 1,
        traceparent: String::new(),
    };
    let control = Arc::new(RunControl::new(
        provision.run_id.clone(),
        provision.lease_fencing_token,
        provision.environment_epoch,
    ));
    let (sender, mut receiver) = mpsc::channel(4);

    execute_provision(provision, sender.into(), control)
        .await
        .unwrap();

    let messages: Vec<_> = std::iter::from_fn(|| receiver.try_recv().ok()).collect();
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        messages[0].message.as_ref().unwrap(),
        supervisor_to_core::Message::ObservedRunEvent(ObservedRunEvent { kind, sequence, .. })
            if *kind == RunEventKind::StartFailed as i32 && *sequence == 1
    ));
}
