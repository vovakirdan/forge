//! Persisted pre-TASK-05 values must remain readable after handler extraction.

use forge_protocol::wire::{CommandName, CommandReceipt, CommandStatus};
use serde_json::{Value, json};

pub const PROJECT_ID: &str = "01900000-0000-7000-8000-000000000001";
pub const HUMAN_ID: &str = "01900000-0000-7000-8000-000000000002";
pub const CORE_ID: &str = "01900000-0000-7000-8000-000000000003";
pub const CREATE_HASH: &str = "33f9b70e140b2e5fcd6393dc5036191892b2c9458f0352f7dc5db87136c4b17e";
pub const RECOVERY_HASH: &str = "4863da0f495813a8b5587e4790e34eb3bd1c261a4a9b1eaf5a421d86384d33e8";

pub fn legacy_receipt() -> Value {
    json!({
        "command_id": "01900000-0000-7000-8000-000000000004",
        "status": "applied",
        "project_revision": 8,
        "event_ids": ["01900000-0000-7000-8000-000000000005"],
        "resource": {"kind": "project", "id": PROJECT_ID}
    })
}

#[test]
fn pre_extraction_m1_receipt_roundtrips_without_changing_persisted_shape() {
    let value = legacy_receipt();
    let receipt: CommandReceipt = serde_json::from_value(value.clone()).expect("legacy receipt");
    assert_eq!(receipt.status, CommandStatus::Applied);
    assert_eq!(serde_json::to_value(receipt).expect("receipt JSON"), value);
}

#[tokio::test]
async fn pre_extraction_hashes_include_identical_m0_and_m1_fields() -> anyhow::Result<()> {
    let fixture = super::fixture::Fixture::new(super::fixture::BackendKind::Memory).await?;
    for (name, revision, payload, expected) in [
        (
            CommandName::CreateProject,
            0,
            json!({"name":"Legacy project"}),
            CREATE_HASH,
        ),
        (
            CommandName::ConfigureBootRecoveryPolicy,
            7,
            json!({"policy":"manual_hold"}),
            RECOVERY_HASH,
        ),
    ] {
        let envelope = fixture.envelope(name, revision, payload, "old-format");
        assert_eq!(
            forge_application::command_fingerprint(&envelope, fixture.context.actor)?,
            expected
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; validates production M1 replay"]
async fn production_m1_command_keeps_legacy_hash_and_original_receipt() -> anyhow::Result<()> {
    use super::fixture::{Backend, BackendKind, Fixture};
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    for _ in 0..6 {
        fixture
            .execute(CommandName::StartProjectExecution, json!({}))
            .await?;
    }
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL fixture")
    };
    let core = fixture.core(pool);
    let envelope = fixture.envelope(
        CommandName::ConfigureBootRecoveryPolicy,
        7,
        json!({"policy":"manual_hold"}),
        "legacy-recovery",
    );
    let receipt = core.execute_command(envelope.clone()).await?;
    let snapshot = fixture.snapshot().await?;
    let record = snapshot
        .rows("idempotency_keys")
        .iter()
        .find(|record| record["idempotency_key"] == "legacy-recovery")
        .expect("M1 record");
    assert_eq!(record["request_hash"], RECOVERY_HASH);
    assert_eq!(record["receipt"], serde_json::to_value(&receipt)?);
    fixture
        .execute(CommandName::StopProjectExecution, json!({}))
        .await?;
    let before = fixture.snapshot().await?.raw;
    let replay = core.execute_command(envelope).await?;
    let mut expected = serde_json::to_value(receipt)?;
    expected["status"] = json!("replayed");
    assert_eq!(serde_json::to_value(replay)?, expected);
    assert_eq!(fixture.snapshot().await?.raw, before);
    Ok(())
}
