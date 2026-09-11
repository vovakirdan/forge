use super::*;
use crate::{Actor, ActorId, EmployeeId, PipelineVersionId, ProjectId, StageId, TaskId, Timestamp};
use uuid::Uuid;

fn thread() -> EmployeeThread {
    let now = Timestamp::now_utc();
    EmployeeThread::new(EmployeeThreadInput {
        id: Uuid::now_v7(),
        project_id: ProjectId::new(),
        employee_id: EmployeeId::new(),
        task_id: None,
        created_by: Actor::human(ActorId::new()),
        created_at: now,
        updated_at: now,
        revision: 1,
        last_sequence: 0,
    })
    .unwrap()
}

fn context() -> TaskMessageContext {
    TaskMessageContext {
        task_id: TaskId::new(),
        pipeline_version_id: PipelineVersionId::new(),
        stage_id: StageId::new("work").unwrap(),
        stage_visit: 1,
    }
}

fn send_input(thread: &EmployeeThread) -> SendMessageInput {
    SendMessageInput {
        id: Uuid::now_v7(),
        expected_thread_revision: thread.data().revision,
        sender: thread.data().created_by,
        target: MessageTarget::Inbox,
        kind: MessageKind::Question,
        requirement: DeliveryRequirement::Answered,
        body: "What are you working on?".into(),
        reply_to: None,
        created_at: thread.data().updated_at,
    }
}

fn message() -> EmployeeMessage {
    let mut thread = thread();
    thread.send(send_input(&thread)).unwrap()
}

#[test]
fn communication_context_is_versioned_and_cannot_inherit_task_tools() {
    let source = message();
    let data = source.data();
    let input = CommunicationContextInput {
        schema_version: 3,
        context_snapshot_id: Uuid::now_v7(),
        project_id: data.project_id,
        employee_id: data.employee_id,
        run_id: Uuid::now_v7(),
        assignment: crate::CommunicationAssignmentRef {
            assignment_id: Uuid::now_v7(),
            thread_id: data.thread_id,
            source_message_id: data.id,
        },
        context_task_id: Some(TaskId::new()),
        capability_grants: vec!["inbox.reply".into(), "communication.complete".into()],
        source_message: source.clone(),
        knowledge_context: None,
        created_at: data.created_at,
    };
    assert!(CommunicationContext::new(input.clone()).is_ok());
    for grant in [
        "outcome.submit",
        "artifact.submit",
        "human.request",
        "unknown",
    ] {
        let mut invalid = input.clone();
        invalid.capability_grants.push(grant.into());
        assert!(CommunicationContext::new(invalid).is_err());
    }
    let mut invalid = input.clone();
    invalid.schema_version = 2;
    assert!(CommunicationContext::new(invalid).is_err());
    let mut invalid = input.clone();
    invalid.assignment.source_message_id = Uuid::now_v7();
    assert!(CommunicationContext::new(invalid).is_err());
    let mut invalid = input.clone();
    invalid.employee_id = EmployeeId::new();
    assert!(CommunicationContext::new(invalid).is_err());
    let mut followup = input.clone();
    let mut source = followup.source_message.data().clone();
    source.kind = MessageKind::Reply;
    source.requirement = DeliveryRequirement::Informational;
    source.reply_to = Some(Uuid::now_v7());
    followup.source_message = EmployeeMessage::new(source.clone()).unwrap();
    assert!(
        CommunicationContext::new(followup.clone()).is_ok(),
        "human follow-up is a new input"
    );
    source.sender = Actor::employee(ActorId::from(input.employee_id.as_uuid()));
    followup.source_message = EmployeeMessage::new(source.clone()).unwrap();
    assert!(
        CommunicationContext::new(followup.clone()).is_err(),
        "own Employee output cannot recurse"
    );
    source.sender = Actor::employee(ActorId::new());
    followup.source_message = EmployeeMessage::new(source).unwrap();
    assert!(
        CommunicationContext::new(followup).is_ok(),
        "a different Employee is an external sender"
    );
}

fn scope(message: &EmployeeMessage) -> DeliveryScope {
    DeliveryScope {
        project_id: message.data().project_id,
        employee_id: message.data().employee_id,
        thread_id: message.data().thread_id,
        run_id: Uuid::now_v7(),
        fencing_token: 1,
        environment_epoch: 1,
        task_context: message.data().target.task_context().cloned(),
    }
}

#[test]
fn taskless_thread_allocates_ordered_immutable_messages() {
    let mut thread = thread();
    let first = thread.send(send_input(&thread)).unwrap();
    let second = thread.send(send_input(&thread)).unwrap();
    assert_eq!(
        (
            first.data().sequence,
            second.data().sequence,
            thread.data().revision
        ),
        (1, 2, 3)
    );
}

#[test]
fn failed_send_does_not_consume_sequence_or_revision() {
    let mut thread = thread();
    let before = thread.clone();
    let mut input = send_input(&thread);
    input.body.clear();
    assert!(thread.send(input).is_err());
    assert_eq!(thread, before);
}

#[test]
fn stale_thread_revision_is_rejected() {
    let mut thread = thread();
    let input = send_input(&thread);
    thread.send(send_input(&thread)).unwrap();
    assert!(thread.send(input).is_err());
}

#[test]
fn message_text_is_not_in_debug_output() {
    let message = message();
    assert!(!format!("{message:?}").contains(&message.data().body));
}

#[test]
fn invalid_message_snapshot_is_rejected_during_decode() {
    let mut json = serde_json::to_value(message()).unwrap();
    json["sequence"] = 0.into();
    assert!(serde_json::from_value::<EmployeeMessage>(json).is_err());
}

#[test]
fn invalid_thread_snapshot_is_rejected_during_decode() {
    let mut json = serde_json::to_value(thread()).unwrap();
    json["revision"] = 10.into();
    assert!(serde_json::from_value::<EmployeeThread>(json).is_err());
}

#[test]
fn cross_project_delivery_is_rejected() {
    let message = message();
    let mut scope = scope(&message);
    scope.project_id = ProjectId::new();
    assert!(scope.validate_message(&message).is_err());
}

#[test]
fn task_assignment_cannot_consume_unaddressed_inbox_message() {
    let message = message();
    let mut scope = scope(&message);
    scope.task_context = Some(context());
    assert!(scope.validate_message(&message).is_err());
}

#[test]
fn same_employee_cannot_acknowledge_another_task_visit() {
    let mut thread = thread();
    let mut input = send_input(&thread);
    input.target = MessageTarget::TaskExecution { context: context() };
    let message = thread.send(input).unwrap();
    let mut scope = scope(&message);
    scope.task_context.as_mut().unwrap().stage_visit += 1;
    assert!(scope.validate_message(&message).is_err());
}

#[test]
fn exact_run_delivery_rejects_a_newer_fence() {
    let mut thread = thread();
    let mut input = send_input(&thread);
    let run_id = Uuid::now_v7();
    input.target = MessageTarget::ExactRun {
        context: context(),
        run_id,
        fencing_token: 4,
        environment_epoch: 2,
    };
    let message = thread.send(input).unwrap();
    let mut scope = scope(&message);
    scope.run_id = run_id;
    scope.fencing_token = 5;
    scope.environment_epoch = 2;
    assert!(scope.validate_message(&message).is_err());
}

#[test]
fn runtime_acceptance_does_not_satisfy_employee_acknowledgement() {
    let mut thread = thread();
    let mut input = send_input(&thread);
    input.requirement = DeliveryRequirement::Acknowledged;
    let message = thread.send(input).unwrap();
    let receipt = DeliveryReceipt::new(
        &message,
        scope(&message),
        DeliveryReceiptKind::RuntimeAccepted,
        None,
        message.data().created_at,
    )
    .unwrap();
    assert!(!receipt.satisfies(&message));
}

#[test]
fn acknowledgement_does_not_satisfy_required_answer() {
    let message = message();
    let receipt = DeliveryReceipt::new(
        &message,
        scope(&message),
        DeliveryReceiptKind::Acknowledged,
        None,
        message.data().created_at,
    )
    .unwrap();
    assert!(!receipt.satisfies(&message));
}

#[test]
fn answer_requires_a_reply_reference() {
    let message = message();
    assert!(
        DeliveryReceipt::new(
            &message,
            scope(&message),
            DeliveryReceiptKind::Answered,
            None,
            message.data().created_at
        )
        .is_err()
    );
}

#[test]
fn required_answer_satisfied_by_attributed_reply() {
    let message = message();
    let receipt = DeliveryReceipt::new(
        &message,
        scope(&message),
        DeliveryReceiptKind::Answered,
        Some(Uuid::now_v7()),
        message.data().created_at,
    )
    .unwrap();
    assert!(receipt.satisfies(&message));
}

#[test]
fn notification_does_not_gate_task_completion() {
    let mut thread = thread();
    let mut input = send_input(&thread);
    let context = context();
    input.target = MessageTarget::TaskExecution {
        context: context.clone(),
    };
    input.kind = MessageKind::Notification;
    input.requirement = DeliveryRequirement::Informational;
    let message = thread.send(input).unwrap();
    assert!(!message.gates_completion(&context));
}

#[test]
fn required_instruction_gates_only_its_target_task_context() {
    let mut thread = thread();
    let mut input = send_input(&thread);
    let context = context();
    input.target = MessageTarget::TaskExecution {
        context: context.clone(),
    };
    input.kind = MessageKind::Instruction;
    input.requirement = DeliveryRequirement::Acknowledged;
    let message = thread.send(input).unwrap();
    assert!(message.gates_completion(&context));
    let mut other = context;
    other.task_id = TaskId::new();
    assert!(!message.gates_completion(&other));
}
