//! Employee management runs through exactly the same command engine on both stores.
use anyhow::{Context, Result};
use forge_application::{CommandContext, CommandError, CommandTransaction, RepositoryError};
use forge_domain::{Employee, EmployeeId, EmployeeState};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_storage::PostgresStore;
use serde_json::json;

use super::{
    atomicity::assert_faults,
    fixture::{Backend, BackendKind, Fixture},
};

pub async fn create(fixture: &Fixture, name: &str) -> Result<EmployeeId> {
    Ok(fixture
        .execute(
            CommandName::CreateEmployee,
            json!({
                "name":name, "role":"developer", "stage_eligibility":{"mode":"any"}
            }),
        )
        .await?
        .resource
        .context("employee")?
        .id
        .parse()?)
}

pub async fn load(fixture: &Fixture, id: EmployeeId) -> Result<Employee> {
    match &fixture.backend {
        Backend::Memory(store) => store
            .snapshot()
            .await
            .employees
            .get(&id)
            .cloned()
            .context("employee"),
        Backend::Postgres(pool) => Ok(PostgresStore::from_pool(pool.clone())
            .load_employee(id)
            .await?
            .context("employee")?
            .employee),
    }
}

pub async fn employee_management_round_trip(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let pipeline = fixture
        .pipeline(forge_testkit::m0::single_stage_pipeline())
        .await?;
    let initial_eligibility = json!({"mode":"only","stages":[{
        "pipeline_version_id":pipeline,"stage_id":"work"
    }]});
    let id = create(&fixture, "Bob").await?;
    let original = load(&fixture, id).await?;
    assert_eq!(
        (original.revision(), original.max_concurrent_runs()),
        (1, 1)
    );
    let before = fixture.snapshot().await?;
    let amendment = fixture.envelope(
        CommandName::AmendEmployee,
        before.projects[&fixture.project_id].revision(),
        json!({
            "employee_id":id,"expected_employee_revision":1,
            "patch":{"name":"Bob developer","role":"engineer","max_concurrent_runs":3,
                "stage_eligibility":initial_eligibility}
        }),
        "amend-bob",
    );
    assert_faults(&fixture, &amendment).await?;
    let applied = fixture.execute_as(&amendment, &fixture.context).await?;
    let current = load(&fixture, id).await?;
    assert_eq!((current.revision(), current.max_concurrent_runs()), (2, 3));
    assert_eq!(current.role().as_str(), "engineer");
    assert_eq!(original.name(), "Bob");
    fixture
        .execute(
            CommandName::AmendEmployee,
            json!({"employee_id":id,"expected_employee_revision":2,
                "patch":{"role":"reviewer","stage_eligibility":{"mode":"any"}}}),
        )
        .await?;
    let history = fixture.snapshot().await?.raw;
    let events = history["event_log"].as_array().context("audit events")?;
    let created = events
        .iter()
        .find(|event| event["event_type"] == "employee_created")
        .context("initial immutable Employee configuration")?;
    assert_eq!(created["payload"]["role"], "developer");
    assert_eq!(
        created["payload"]["stage_eligibility"],
        json!({"mode":"any"})
    );
    let first_amendment = events
        .iter()
        .find(|event| event["id"] == applied.event_ids[0])
        .context("first immutable amendment")?;
    assert_eq!(first_amendment["payload"]["role"], "engineer");
    assert_eq!(
        first_amendment["payload"]["stage_eligibility"],
        initial_eligibility
    );
    assert_eq!(load(&fixture, id).await?.role().as_str(), "reviewer");
    for (name, state) in [
        (CommandName::DisableEmployee, EmployeeState::Disabled),
        (CommandName::EnableEmployee, EmployeeState::Enabled),
        (CommandName::RetireEmployee, EmployeeState::Retired),
    ] {
        let revision = load(&fixture, id).await?.revision();
        fixture
            .execute(
                name,
                json!({"employee_id":id,"expected_employee_revision":revision}),
            )
            .await?;
        let current = load(&fixture, id).await?;
        assert_eq!((current.revision(), current.state()), (revision + 1, state));
    }
    let before_replay = fixture.snapshot().await?;
    let replay = fixture.execute_as(&amendment, &fixture.context).await?;
    assert_eq!(replay.status, CommandStatus::Replayed);
    assert_eq!(replay.command_id, applied.command_id);
    assert_eq!(replay.event_ids, applied.event_ids);
    assert_eq!(before_replay.raw, fixture.snapshot().await?.raw);
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn employee_management_refusals_are_atomic(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let id = create(&fixture, "Bob").await?;
    let other = create(&fixture, "Alice").await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let stale = fixture.envelope(
        CommandName::DisableEmployee,
        revision,
        json!({"employee_id":id,"expected_employee_revision":2}),
        "stale-employee",
    );
    assert!(matches!(
        fixture
            .assert_unchanged_after(&stale, &fixture.context)
            .await?,
        CommandError::Repository(RepositoryError::StaleRevision {
            aggregate: "employee"
        })
    ));
    for (key, patch) in [
        (
            "name-conflict",
            json!({"name":load(&fixture, other).await?.name()}),
        ),
        ("blank-name", json!({"name":" "})),
        (
            "foreign-stage",
            json!({"stage_eligibility":{"mode":"only","stages":[{"pipeline_version_id":forge_domain::PipelineVersionId::new(),"stage_id":"work"}]}}),
        ),
    ] {
        let envelope = fixture.envelope(
            CommandName::AmendEmployee,
            revision,
            json!({"employee_id":id,"expected_employee_revision":1,"patch":patch}),
            key,
        );
        fixture
            .assert_unchanged_after(&envelope, &fixture.context)
            .await?;
    }
    let valid = fixture.envelope(
        CommandName::AmendEmployee,
        revision,
        json!({"employee_id":id,"expected_employee_revision":1,"patch":{"max_concurrent_runs":3}}),
        "forbidden-amend",
    );
    let mut denied = fixture.context.clone();
    denied.capabilities.clear();
    assert!(matches!(
        fixture.assert_unchanged_after(&valid, &denied).await?,
        CommandError::Forbidden
    ));
    fixture
        .execute(
            CommandName::RetireEmployee,
            json!({"employee_id":id,"expected_employee_revision":1}),
        )
        .await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let enable = fixture.envelope(
        CommandName::EnableEmployee,
        revision,
        json!({"employee_id":id,"expected_employee_revision":2}),
        "retirement-is-final",
    );
    fixture
        .assert_unchanged_after(&enable, &fixture.context)
        .await?;
    let other_project = forge_domain::ProjectId::new();
    let foreign = Fixture {
        project_id: other_project,
        context: CommandContext::local_human(
            other_project,
            fixture.context.actor,
            fixture.context.core_actor,
        ),
        ..fixture.clone()
    };
    foreign
        .execute(CommandName::CreateProject, json!({"name":"Other Project"}))
        .await?;
    let cross_project = foreign.envelope(
        CommandName::EnableEmployee,
        foreign.snapshot().await?.projects[&other_project].revision(),
        json!({"employee_id":id,"expected_employee_revision":2}),
        "foreign-employee",
    );
    assert!(matches!(
        foreign
            .assert_unchanged_after(&cross_project, &foreign.context)
            .await?,
        CommandError::NotFound {
            aggregate: "employee"
        }
    ));
    Ok(())
}

pub async fn employee_repository_rejects_stale_overwrite(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let id = create(&fixture, "CAS Bob").await?;
    let mut stale = load(&fixture, id).await?;
    fixture
        .execute(
            CommandName::AmendEmployee,
            json!({"employee_id":id,
        "expected_employee_revision":1,"patch":{"max_concurrent_runs":3}}),
        )
        .await?;
    stale.disable(stale.updated_at())?;
    let before = fixture.snapshot().await?.raw;
    let result = match &fixture.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            tx.update_employee(&stale, 1).await
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            CommandTransaction::update_employee(&mut tx, &stale, 1).await
        }
    };
    assert!(matches!(
        result,
        Err(RepositoryError::StaleRevision {
            aggregate: "employee"
        })
    ));
    assert_eq!(fixture.snapshot().await?.raw, before);
    Ok(())
}
