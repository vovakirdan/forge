//! Project Task-property schema configuration is initial-only and audited.

use anyhow::Result;
use forge_application::{CommandEnvelope, CommandError};
use forge_domain::DomainError;
use forge_protocol::wire::{CommandName, CommandRequest, CommandStatus};
use forge_testkit::m0::single_stage_pipeline;
use serde_json::{Value, json};
use uuid::Uuid;

use super::fixture::{BackendKind, Fixture};

fn impact_schema() -> Value {
    json!({"definitions":{"impact":{
        "key":"impact","display_name":"Impact","property_type":"enum",
        "required":true,"default_value":{"type":"enum","value":"medium"},
        "allowed_choices":["high","medium"]
    }}})
}

pub async fn configure_initial_schema_without_reinterpreting_tasks(
    kind: BackendKind,
) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let before = fixture.snapshot().await?;
    let revision = before.projects[&fixture.project_id].revision();
    let envelope = fixture.envelope(
        CommandName::ConfigureTaskPropertySchema,
        revision,
        json!({"schema":impact_schema()}),
        "initial-task-property-schema",
    );
    let receipt = fixture.execute_as(&envelope, &fixture.context).await?;
    assert_eq!(receipt.status, CommandStatus::Applied);
    assert_eq!(receipt.project_revision, revision + 1);
    let configured = fixture.snapshot().await?;
    assert_eq!(
        serde_json::to_value(configured.projects[&fixture.project_id].property_schema())?,
        impact_schema()
    );
    assert_eq!(
        configured.rows("event_log").len(),
        before.rows("event_log").len() + 1
    );
    assert!(
        configured
            .rows("event_log")
            .iter()
            .any(|event| event["id"] == receipt.event_ids[0]
                && event["event_type"] == "task_property_schema_configured")
    );
    configured.assert_audit_atomic();

    let replay = fixture.execute_as(&envelope, &fixture.context).await?;
    assert_eq!(replay.status, CommandStatus::Replayed);
    assert_eq!(replay.command_id, receipt.command_id);
    assert_eq!(fixture.snapshot().await?.raw, configured.raw);

    let stale = fixture.envelope(
        CommandName::ConfigureTaskPropertySchema,
        revision,
        json!({"schema":{"definitions":{}}}),
        "stale-task-property-schema",
    );
    assert!(matches!(
        fixture
            .assert_unchanged_after(&stale, &fixture.context)
            .await?,
        CommandError::Domain(DomainError::StaleProjectRevision { .. })
    ));

    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let task = fixture.create_task(pipeline, "Existing draft").await?;
    fixture
        .task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    let task_before = fixture.task(task).await?;
    assert_eq!(
        serde_json::to_value(task_before.spec().properties())?,
        json!({"impact":{"type":"enum","value":"medium"}})
    );
    let project_revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let forbidden = fixture.envelope(
        CommandName::ConfigureTaskPropertySchema,
        project_revision,
        json!({"schema":{"definitions":{}}}),
        "schema-after-task",
    );
    assert!(matches!(
        fixture
            .assert_unchanged_after(&forbidden, &fixture.context)
            .await?,
        CommandError::Domain(DomainError::TaskPropertySchemaLocked)
    ));
    assert_eq!(fixture.task(task).await?, task_before);
    Ok(())
}

#[test]
fn configured_schema_rejects_invalid_shape_and_large_payload() {
    let project_id = forge_domain::ProjectId::new();
    let parse = |schema: Value| {
        CommandEnvelope::parse(
            CommandName::ConfigureTaskPropertySchema,
            CommandRequest {
                project_id: project_id.to_string(),
                expected_revision: 1,
                payload: json!({"schema":schema}).as_object().unwrap().clone(),
            },
            Uuid::now_v7().to_string(),
        )
    };
    assert!(
        parse(json!({"definitions":{"impact":{
            "key":"other","display_name":"Impact","property_type":"text",
            "required":false,"default_value":null,"allowed_choices":null
        }}}))
        .is_err()
    );
    assert!(
        parse(json!({"definitions":{"impact":{
            "key":"impact","display_name":"Impact","property_type":"enum",
            "required":true,"default_value":{"type":"enum","value":"low"},
            "allowed_choices":["high","medium"]
        }}}))
        .is_err()
    );
    let definitions = (0..65)
        .map(|index| {
            let key = format!("field_{index}");
            (
                key.clone(),
                json!({
                    "key":key,"display_name":"Field","property_type":"text",
                    "required":false,"default_value":null,"allowed_choices":null
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    assert!(parse(json!({"definitions":definitions})).is_err());
    assert!(parse(json!({"definitions":{},"unsupported":true})).is_err());
    assert!(
        parse(json!({"definitions":{"large":{
            "key":"large","display_name":"Large","property_type":"text",
            "required":false,"default_value":{"type":"text","value":"x".repeat(65536)},
            "allowed_choices":null
        }}}))
        .is_err()
    );
}
