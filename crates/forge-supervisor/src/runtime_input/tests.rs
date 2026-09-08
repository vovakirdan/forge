use super::*;
use crate::{journal::Journal, surface::private_directory};
use forge_domain::{
    Actor, ActorId, CredentialBinding, CredentialDeliveryMode, EmployeeId, ExecutionProfileInput,
    PipelineVersionId, ProjectId, StageId, TaskId, Timestamp,
    communication::{DeliveryRequirement, MessageInput, MessageKind, TaskMessageContext},
    runtime::{RuntimeBinding, SandboxRunSpec, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::supervisor::v1::{
    AcknowledgementDisposition, CoreAcknowledgement, RuntimeMessageInput, supervisor_to_core,
};
use forge_provider_claude::ClaudeAdapter;
use std::collections::BTreeSet;

fn fixture(
    live: bool,
) -> (
    SupervisorConfig,
    ProvisionRun,
    RunRegistry,
    DeliverRuntimeInput,
) {
    let root = std::env::temp_dir().join(format!("forge-input-transport-{}", new_id()));
    private_directory(&root).unwrap();
    let mut config = SupervisorConfig::new(
        root.join("core.sock"),
        "test-host".into(),
        "test-boot".into(),
    );
    config.state_directory = root.clone();
    config.grants_directory = root.join("grants");
    let project = ProjectId::new();
    let mut provision = crate::tests::provision();
    let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let spec = SandboxRunSpec {
        schema_version: 2,
        project_id: project,
        surface_id: Uuid::now_v7(),
        instruction: "Offline transport contract".into(),
        binding: RuntimeBinding {
            execution_profile: ExecutionProfileInput {
                id: Uuid::now_v7(),
                revision: 1,
                project_id: project,
                adapter_id: "claude_code_cli".into(),
                adapter_version: "2.1.263".into(),
                provider_id: "anthropic".into(),
                model: "synthetic".into(),
                credential_binding: CredentialBinding {
                    id: Uuid::now_v7(),
                    project_id: project,
                    secret_id: Uuid::now_v7(),
                    account_id: Some("synthetic".into()),
                    allowed_delivery_modes: BTreeSet::from([mode]),
                },
                credential_delivery: mode,
                capability_profile: if live {
                    ClaudeAdapter::live_capabilities()
                } else {
                    ClaudeAdapter::capabilities()
                },
            }
            .try_into()
            .unwrap(),
            image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
            surface: SurfaceSpec::FilesystemSandbox,
            access: SurfaceAccess::ReadWrite,
            limits: Default::default(),
            budget: Default::default(),
            system_prompt: "Synthetic rules".into(),
            employee_prompt: "Synthetic employee".into(),
        },
    };
    provision.run_spec_version = 2;
    provision.run_spec_json = serde_json::to_string(&spec).unwrap();
    let source = EmployeeMessage::new(MessageInput {
        id: Uuid::now_v7(),
        thread_id: Uuid::now_v7(),
        project_id: project,
        employee_id: EmployeeId::from(provision.employee_id.parse::<Uuid>().unwrap()),
        sequence: 1,
        sender: Actor::human(ActorId::new()),
        target: MessageTarget::TaskExecution {
            context: TaskMessageContext {
                task_id: TaskId::from(provision.task_id.parse::<Uuid>().unwrap()),
                pipeline_version_id: PipelineVersionId::new(),
                stage_id: StageId::new("work").unwrap(),
                stage_visit: 1,
            },
        },
        kind: MessageKind::Instruction,
        requirement: DeliveryRequirement::Acknowledged,
        body: "Keep backwards compatibility.".into(),
        reply_to: None,
        created_at: Timestamp::now_utc(),
    })
    .unwrap();
    let input = DeliverRuntimeInput {
        command_id: new_id(),
        run_id: provision.run_id.clone(),
        lease_fencing_token: provision.lease_fencing_token,
        environment_epoch: provision.environment_epoch,
        sequence: 1,
        action: Some(Action::Message(RuntimeMessageInput {
            source_message_json: serde_json::to_string(&source).unwrap(),
        })),
    };
    let grant = scoped_directory(&config.grants_directory, &provision);
    private_directory(&config.grants_directory).unwrap();
    private_directory(grant.parent().unwrap()).unwrap();
    private_directory(&grant).unwrap();
    let registry = RunRegistry::new(Journal::open(&root, "test-host", 1024 * 1024).unwrap());
    (config, provision, registry, input)
}

#[tokio::test]
async fn mailbox_effect_is_not_acceptance_and_native_receipt_replays_after_restart() {
    let (config, provision, registry, input) = fixture(true);
    registry.register(&provision, "test-boot").await.unwrap();
    deliver(&config, &registry, input.clone()).await.unwrap();
    deliver(&config, &registry, input.clone()).await.unwrap();
    assert_eq!(
        registry.journal.lock().await.pending().count(),
        0,
        "filesystem write cannot claim native receipt"
    );
    let evidence = scoped_directory(&config.state_directory.join("evidence"), &provision);
    private_directory(evidence.parent().unwrap().parent().unwrap()).unwrap();
    private_directory(evidence.parent().unwrap()).unwrap();
    private_directory(&evidence).unwrap();
    private_directory(&evidence.join("input-receipts")).unwrap();
    let native = receipt(&input, Status::RuntimeAccepted);
    immutable(
        &evidence
            .join("input-receipts")
            .join(format!("{}.json", input.command_id)),
        &SecretBytes::new(serde_json::to_vec(&native).unwrap()),
    )
    .unwrap();
    collect(&config, &registry, &provision).await.unwrap();
    let wire = registry
        .journal
        .lock()
        .await
        .pending()
        .next()
        .unwrap()
        .clone();
    assert!(
        matches!(&wire.message,Some(supervisor_to_core::Message::RuntimeInputReceipt(value)) if *value==native)
    );
    drop(registry);
    let mut journal = Journal::open(&config.state_directory, "test-host", 1024 * 1024).unwrap();
    assert_eq!(journal.pending().next(), Some(&wire));
    journal.acknowledge(&ack(native.message_id)).unwrap();
    cleanup_mailbox(&config, &input).unwrap();
    assert_eq!(journal.pending().count(), 0);
    assert!(
        !journal.register_input(&input).unwrap(),
        "duplicate input replays original receipt without a second write"
    );
    assert_eq!(journal.pending().next(), Some(&wire));
    let mut changed = input;
    changed.sequence += 1;
    assert!(matches!(
        journal.register_input(&changed),
        Err(SupervisorError::ConflictingProvision)
    ));
}

#[tokio::test]
async fn legacy_lane_is_explicitly_unsupported_and_stale_epoch_is_rejected() {
    let (config, provision, registry, input) = fixture(false);
    registry.register(&provision, "test-boot").await.unwrap();
    let mut stale = input.clone();
    stale.environment_epoch += 1;
    assert!(deliver(&config, &registry, stale).await.is_err());
    deliver(&config, &registry, input).await.unwrap();
    assert!(
        matches!(&registry.journal.lock().await.pending().next().unwrap().message,Some(supervisor_to_core::Message::RuntimeInputReceipt(value)) if value.status==Status::Unsupported as i32)
    );
}

#[tokio::test]
async fn acknowledged_inputs_do_not_consume_outstanding_capacity_forever() {
    let (_config, provision, registry, mut input) = fixture(true);
    registry.register(&provision, "test-boot").await.unwrap();
    let mut journal = registry.journal.lock().await;
    for sequence in 1..=201 {
        input.command_id = new_id();
        input.sequence = sequence;
        assert!(journal.register_input(&input).unwrap());
        let native = receipt(&input, Status::RuntimeAccepted);
        journal.record_input_receipt(native.clone()).unwrap();
        journal.acknowledge(&ack(native.message_id)).unwrap();
    }
    assert_eq!(journal.pending().count(), 0);
}

fn ack(id: String) -> CoreAcknowledgement {
    CoreAcknowledgement {
        message_id: new_id(),
        acknowledged_message_id: id,
        disposition: AcknowledgementDisposition::Accepted as i32,
        reason_code: "input_observed".into(),
        message: String::new(),
    }
}
