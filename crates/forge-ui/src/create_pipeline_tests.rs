use serde_json::json;

use crate::{command::CommandTarget, create_pipeline::CreatePipeline};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const PIPELINE: &str = "01988000-0000-7000-8000-000000000002";

#[test]
fn create_pipeline_accepts_complete_graph_and_exact_receipt() {
    let graph = json!({"name":"Initial Pipeline","task_kinds":["delivery"],"entry_stage_id":"work","stages":[{"id":"work","name":"Work","executor_kind":"employee","outcomes":["done"]}],"transitions":[{"from_stage_id":"work","outcome":"done","target":{"kind":"done"}}]});
    let body =
        serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":3,"payload":graph}))
            .unwrap();
    let command = CreatePipeline::parse(&body).unwrap();
    let receipt = serde_json::to_vec(&json!({"command_id":"01988000-0000-7000-8000-000000000003","status":"applied","project_revision":4,"event_ids":["01988000-0000-7000-8000-000000000004","01988000-0000-7000-8000-000000000005"],"resource":{"kind":"pipeline","id":PIPELINE}})).unwrap();
    command.validate_receipt(&receipt).unwrap();
    assert!(CommandTarget::from_browser_path("/api/commands/create_pipeline").is_some());
    assert!(CommandTarget::from_browser_path("/api/commands/create_pipeline/").is_none());
    let forged = serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":3,"payload":{"name":"Initial Pipeline","task_kinds":["delivery"],"entry_stage_id":"work","stages":[{"id":"work","name":"Work","executor_kind":"employee","outcomes":["done"],"actor":"forged"}],"transitions":[{"from_stage_id":"work","outcome":"done","target":{"kind":"done"}}]}})).unwrap();
    assert!(CreatePipeline::parse(&forged).is_err());
}
