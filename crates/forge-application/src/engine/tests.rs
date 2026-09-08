use super::*;
use crate::{CommandPayload, IdempotencyKey};
use forge_domain::ActorId;
use forge_protocol::wire::{CommandName, CommandRequest};
use serde_json::json;

fn fixture() -> (CommandEnvelope, CommandContext) {
    let project_id = ProjectId::new();
    let envelope = CommandEnvelope::parse(
        CommandName::CreateProject,
        CommandRequest {
            project_id: project_id.to_string(),
            expected_revision: 0,
            payload: json!({"name": "reference fixture"})
                .as_object()
                .expect("object")
                .clone(),
        },
        "fixture-key",
    )
    .expect("parsed fixture");
    let context = CommandContext::local_human(
        project_id,
        Actor::human(ActorId::new()),
        Actor::core(ActorId::new()),
    );
    (envelope, context)
}

fn prepared(envelope: &CommandEnvelope, context: &CommandContext) -> PreparedCommand {
    let request_hash = command_fingerprint(envelope, context.actor).expect("hash");
    PreparedCommand {
        state: PreparedCommandState::Ready {
            project: None,
            request_hash: request_hash.clone(),
            command_id: CommandId::new(),
            now: Timestamp::now_utc(),
        },
        project_id: envelope.project_id,
        actor: context.actor,
        core_actor: context.core_actor,
        request_hash,
        idempotency_key: envelope.idempotency_key.as_str().to_owned(),
    }
}

#[test]
fn trusted_delegation_is_explicit_and_role_independent() {
    let (envelope, original) = fixture();
    for actor in [
        Actor::employee(ActorId::new()),
        Actor::supervisor(ActorId::new()),
    ] {
        let mut context =
            CommandContext::local_human(envelope.project_id, actor, original.core_actor);
        assert!(matches!(
            context.authorize(&envelope),
            Err(CommandError::Forbidden)
        ));
        context.capabilities.push(CommandName::CreateProject);
        assert!(context.authorize(&envelope).is_ok());
    }
}

#[test]
fn typed_payload_changes_cannot_keep_the_original_fingerprint() {
    let (mut envelope, context) = fixture();
    let original_hash = command_fingerprint(&envelope, context.actor).expect("hash");
    let CommandPayload::CreateProject(command) = &mut envelope.payload else {
        panic!("fixture payload");
    };
    command.name = "unhashed replacement".to_owned();
    assert_eq!(
        command_fingerprint(&envelope, context.actor).expect("hash"),
        original_hash
    );
    assert!(matches!(
        context.authorize(&envelope),
        Err(CommandError::Application(_))
    ));
}

#[test]
fn route_authority_cannot_apply_a_different_payload() {
    let (mut envelope, context) = fixture();
    envelope.payload = CommandPayload::StartProjectExecution { reason: None };
    assert!(matches!(
        context.authorize(&envelope),
        Err(CommandError::Application(_))
    ));
}

#[test]
fn prepared_state_rejects_changed_actor_scope_key_and_revision() {
    let (envelope, context) = fixture();
    let mut changed_context = context.clone();
    changed_context.actor = Actor::human(ActorId::new());
    assert!(
        prepared(&envelope, &context)
            .into_state(&changed_context, &envelope)
            .is_err()
    );
    changed_context = context.clone();
    changed_context.core_actor = Actor::core(ActorId::new());
    assert!(
        prepared(&envelope, &context)
            .into_state(&changed_context, &envelope)
            .is_err()
    );

    let mut changed_envelope = envelope.clone();
    changed_envelope.project_id = ProjectId::new();
    changed_context = context.clone();
    changed_context.project_id = changed_envelope.project_id;
    assert!(
        prepared(&envelope, &context)
            .into_state(&changed_context, &changed_envelope)
            .is_err()
    );
    changed_envelope = envelope.clone();
    changed_envelope.idempotency_key = IdempotencyKey::new("replacement").expect("key");
    assert!(
        prepared(&envelope, &context)
            .into_state(&context, &changed_envelope)
            .is_err()
    );
    changed_envelope = envelope.clone();
    changed_envelope.expected_project_revision = 1;
    assert!(
        prepared(&envelope, &context)
            .into_state(&context, &changed_envelope)
            .is_err()
    );
}

#[test]
fn prepared_state_rechecks_revoked_authority() {
    let (envelope, mut context) = fixture();
    let prepared = prepared(&envelope, &context);
    context.capabilities.clear();
    assert!(matches!(
        prepared.into_state(&context, &envelope),
        Err(CommandError::Forbidden)
    ));
}

#[test]
fn m1_extensions_are_not_supported_by_the_m0_executor() {
    let (mut envelope, _) = fixture();
    for name in [
        CommandName::ConfigureEmployeeRuntime,
        CommandName::ConfigureProjectHook,
        CommandName::EnrollCredential,
        CommandName::ConfigureBootRecoveryPolicy,
        CommandName::AcceptRunRecoveryAssessment,
    ] {
        envelope.name = name;
        assert!(matches!(
            require_m0(&envelope),
            Err(CommandError::UnsupportedCommand)
        ));
    }
}
