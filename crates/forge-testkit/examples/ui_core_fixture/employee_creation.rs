//! Dedicated stopped Project for Employee creation browser acceptance.
use anyhow::Result;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};

pub async fn seed(harness: &M0Harness) -> Result<Value> {
    let name = "Employee creation acceptance";
    let project = harness.create_project(name).await?;
    let version = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    Ok(json!({"id":project,"name":name,"version_id":version}))
}
