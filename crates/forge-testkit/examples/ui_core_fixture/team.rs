use anyhow::Result;
use forge_domain::ProjectId;
use forge_protocol::wire::CommandName;
use forge_testkit::m0::M0Harness;
use serde_json::{Value, json};

pub async fn seed(harness: &M0Harness) -> Result<Value> {
    let project_id: ProjectId = harness.create_project("Team roster acceptance").await?;
    for index in 0..22 {
        let name = format!("Team member {index:02}");
        harness.create_employee(project_id, &name).await?;
    }
    let employees = harness.store.list_employees(project_id).await?;
    for (index, command) in [CommandName::DisableEmployee, CommandName::RetireEmployee]
        .into_iter()
        .enumerate()
    {
        let employee = &employees[index].employee;
        harness
            .execute(
                project_id,
                command,
                json!({
                    "employee_id": employee.id(),
                    "expected_employee_revision": employee.revision()
                }),
            )
            .await?;
    }
    Ok(json!({
        "id":project_id,
        "name":"Team roster acceptance",
        "total":22,
        "disabled_id":employees[0].employee.id(),
        "retired_id":employees[1].employee.id()
    }))
}
