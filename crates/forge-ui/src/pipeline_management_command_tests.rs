use serde_json::{Value, json};

use crate::{command::CommandTarget, pipeline_management_command::PipelineManagementCommand};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const PIPELINE: &str = "01988000-0000-7000-8000-000000000002";
const VERSION: &str = "01988000-0000-7000-8000-000000000003";

fn request(payload: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":3,"payload":payload}))
        .unwrap()
}

#[test]
fn exact_named_pipeline_commands_and_receipts() {
    let definition = json!({"task_kinds":["delivery"],"entry_stage_id":"work","stages":[{"id":"work","name":"Work","executor_kind":"employee","outcomes":["done"]}],"transitions":[{"from_stage_id":"work","outcome":"done","target":{"kind":"done"}}]});
    for (name, target, payload, kind, id) in [
        (
            "publish_pipeline_version",
            CommandTarget::PublishPipelineVersion,
            json!({"pipeline_id":PIPELINE,"expected_pipeline_revision":2,"definition":definition,"make_default":true}),
            "pipeline_version",
            VERSION,
        ),
        (
            "set_pipeline_default_version",
            CommandTarget::SetPipelineDefaultVersion,
            json!({"pipeline_id":PIPELINE,"expected_pipeline_revision":2,"pipeline_version_id":VERSION}),
            "pipeline",
            PIPELINE,
        ),
        (
            "delete_pipeline",
            CommandTarget::DeletePipeline,
            json!({"pipeline_id":PIPELINE,"expected_pipeline_revision":2}),
            "pipeline",
            PIPELINE,
        ),
    ] {
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{name}")).is_some());
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{name}/")).is_none());
        let command = PipelineManagementCommand::parse(&request(payload), target).unwrap();
        let receipt = serde_json::to_vec(&json!({"command_id":VERSION,"status":"applied","project_revision":4,"event_ids":[VERSION],"resource":{"kind":kind,"id":id}})).unwrap();
        command.validate_receipt(&receipt).unwrap();
    }
}

#[test]
fn pipeline_commands_reject_unknown_fields_and_wrong_receipt() {
    for payload in [
        json!({"pipeline_id":PIPELINE,"expected_pipeline_revision":0}),
        json!({"pipeline_id":PIPELINE,"expected_pipeline_revision":2,"hard":true}),
        json!({"pipeline_id":PIPELINE,"expected_pipeline_revision":2,"operation":"delete"}),
    ] {
        assert!(
            PipelineManagementCommand::parse(&request(payload), CommandTarget::DeletePipeline)
                .is_err()
        );
    }
    let command = PipelineManagementCommand::parse(
        &request(json!({"pipeline_id":PIPELINE,"expected_pipeline_revision":2})),
        CommandTarget::DeletePipeline,
    )
    .unwrap();
    let bad = serde_json::to_vec(&json!({"command_id":VERSION,"status":"applied","project_revision":4,"event_ids":[VERSION],"resource":{"kind":"pipeline","id":VERSION}})).unwrap();
    assert!(command.validate_receipt(&bad).is_err());
}
