//! Durable conversations use the production command engine, not a simulated agent.
use anyhow::{Context, Result};
use forge_application::CommandTransaction;
use forge_domain::{EmployeeId, communication::EmployeeMessage};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_storage::PostgresStore;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    active_runs,
    atomicity::assert_faults,
    employees,
    fixture::{Backend, BackendKind, Fixture},
};

async fn open(fixture: &Fixture, employee: EmployeeId) -> Result<Uuid> {
    Ok(fixture
        .execute(
            CommandName::OpenEmployeeThread,
            json!({"employee_id":employee,"task_id":null}),
        )
        .await?
        .resource
        .context("thread")?
        .id
        .parse()?)
}

fn message(thread: Uuid, revision: u64, target: Value) -> Value {
    json!({"thread_id":thread,"expected_thread_revision":revision,"target":target,
        "kind":"instruction","requirement":"acknowledged",
        "body":"Use the documented migration path.","reply_to":null})
}

async fn load(fixture: &Fixture, id: Uuid) -> Result<EmployeeMessage> {
    match &fixture.backend {
        Backend::Memory(store) => store
            .snapshot()
            .await
            .employee_messages
            .get(&id)
            .cloned()
            .context("message"),
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            Ok(CommandTransaction::load_employee_message(&mut tx, id)
                .await?
                .context("message")?)
        }
    }
}

pub async fn ordered_durable_messages(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let employee = employees::create(&fixture, "Bob inbox").await?;
    let thread = open(&fixture, employee).await?;
    // Project is stopped: persistence is allowed without manufacturing a Task or Run.
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let envelope = fixture.envelope(
        CommandName::SendEmployeeMessage,
        revision,
        message(thread, 1, json!({"kind":"inbox"})),
        "first-message",
    );
    assert_faults(&fixture, &envelope).await?;
    let receipt = fixture.execute_as(&envelope, &fixture.context).await?;
    let id = receipt.resource.context("message")?.id.parse()?;
    let stored = load(&fixture, id).await?;
    assert_eq!(stored.data().sequence, 1);
    assert_eq!(stored.data().sender, fixture.context.actor);
    let snapshot = fixture.snapshot().await?;
    assert!(snapshot.tasks.is_empty());
    assert!(snapshot.rows("runs").is_empty());
    assert!(
        !snapshot.raw["event_log"]
            .to_string()
            .contains(&stored.data().body)
    );
    let replay = fixture.execute_as(&envelope, &fixture.context).await?;
    assert_eq!(replay.status, CommandStatus::Replayed);
    assert_eq!(snapshot.raw, fixture.snapshot().await?.raw);
    let next = fixture
        .execute(
            CommandName::SendEmployeeMessage,
            message(thread, 2, json!({"kind":"inbox"})),
        )
        .await?;
    let next_id = next.resource.context("message")?.id.parse()?;
    assert_eq!(load(&fixture, next_id).await?.data().sequence, 2);
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn exact_execution_scope(kind: BackendKind) -> Result<()> {
    let running = active_runs::running_task(kind).await?;
    let fixture = &running.fixture;
    let employee = running.run.require_employee_id()?;
    let task = fixture.task(running.task).await?;
    let thread = open(fixture, employee).await?;
    let target = json!({"kind":"exact_run", "context":{
        "task_id":task.id(),"pipeline_version_id":task.pipeline().pipeline_version_id(),
        "stage_id":task.current_stage_id(),"stage_visit":task.current_stage_visit().context("visit")?.get()},
        "run_id":running.run.id,"fencing_token":running.run.lease_fencing_token,
        "environment_epoch":running.run.environment_epoch});
    fixture
        .execute(
            CommandName::SendEmployeeMessage,
            message(thread, 1, target.clone()),
        )
        .await?;
    for (key, value) in [
        ("fencing_token", json!(running.run.lease_fencing_token + 1)),
        (
            "environment_epoch",
            json!(running.run.environment_epoch + 1),
        ),
    ] {
        let mut stale = target.clone();
        stale[key] = value;
        let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
        let command = fixture.envelope(
            CommandName::SendEmployeeMessage,
            revision,
            message(thread, 2, stale),
            key,
        );
        fixture
            .assert_unchanged_after(&command, &fixture.context)
            .await?;
    }
    let other = employees::create(fixture, "Alice").await?;
    let other_thread = open(fixture, other).await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let wrong_employee = fixture.envelope(
        CommandName::SendEmployeeMessage,
        revision,
        message(other_thread, 1, target),
        "wrong-recipient",
    );
    fixture
        .assert_unchanged_after(&wrong_employee, &fixture.context)
        .await?;
    Ok(())
}

pub async fn refusals_are_atomic(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let employee = employees::create(&fixture, "Bob").await?;
    let thread = open(&fixture, employee).await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let valid = message(thread, 1, json!({"kind":"inbox"}));
    let mut cases = vec![];
    let mut stale = valid.clone();
    stale["expected_thread_revision"] = json!(2);
    cases.push(("stale-thread", stale));
    let mut empty = valid.clone();
    empty["body"] = json!(" ");
    cases.push(("empty-body", empty));
    let mut bad_reply = valid.clone();
    bad_reply["reply_to"] = json!(Uuid::now_v7());
    cases.push(("missing-reply", bad_reply));
    for (key, payload) in cases {
        let command = fixture.envelope(CommandName::SendEmployeeMessage, revision, payload, key);
        fixture
            .assert_unchanged_after(&command, &fixture.context)
            .await?;
    }
    let command = fixture.envelope(
        CommandName::SendEmployeeMessage,
        revision,
        valid.clone(),
        "forbidden",
    );
    let mut denied = fixture.context.clone();
    denied.capabilities.clear();
    fixture.assert_unchanged_after(&command, &denied).await?;
    fixture
        .execute(
            CommandName::RetireEmployee,
            json!({"employee_id":employee,
        "expected_employee_revision":1}),
        )
        .await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let command = fixture.envelope(CommandName::SendEmployeeMessage, revision, valid, "retired");
    fixture
        .assert_unchanged_after(&command, &fixture.context)
        .await?;
    Ok(())
}

pub async fn operator_waiver_is_explicit_and_audited(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let employee = employees::create(&fixture, "Retained Bob").await?;
    let thread = open(&fixture, employee).await?;
    let id: Uuid = fixture
        .execute(
            CommandName::SendEmployeeMessage,
            message(thread, 1, json!({"kind":"inbox"})),
        )
        .await?
        .resource
        .context("message")?
        .id
        .parse()?;
    let original = load(&fixture, id).await?;
    fixture
        .execute(
            CommandName::RetireEmployee,
            json!({"employee_id":employee,
        "expected_employee_revision":1}),
        )
        .await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let envelope=fixture.envelope(CommandName::WaiveMessageRequirement,revision,
        json!({"message_id":id,"reason":"Recipient retired; replacement instruction issued separately."}),"explicit-waiver");
    let mut denied = fixture.context.clone();
    denied.capabilities.clear();
    fixture.assert_unchanged_after(&envelope, &denied).await?;
    let empty = fixture.envelope(
        CommandName::WaiveMessageRequirement,
        revision,
        json!({"message_id":id,"reason":" "}),
        "blank-waiver",
    );
    fixture
        .assert_unchanged_after(&empty, &fixture.context)
        .await?;
    assert_faults(&fixture, &envelope).await?;
    fixture.execute_as(&envelope, &fixture.context).await?;
    let before = fixture.snapshot().await?;
    assert_eq!(
        load(&fixture, id).await?,
        original,
        "waiver must not alter the original message"
    );
    assert_eq!(
        fixture
            .execute_as(&envelope, &fixture.context)
            .await?
            .status,
        CommandStatus::Replayed
    );
    assert_eq!(before.raw, fixture.snapshot().await?.raw);
    assert!(
        before
            .rows("event_log")
            .iter()
            .any(|event| event["event_type"] == "employee_message_requirement_waived")
    );
    before.assert_audit_atomic();
    Ok(())
}

pub async fn waiver_releases_stopped_exact_run_requirement(kind: BackendKind) -> Result<()> {
    let running = active_runs::running_task(kind).await?;
    let fixture = &running.fixture;
    let task = fixture.task(running.task).await?;
    let context = forge_domain::communication::TaskMessageContext {
        task_id: task.id(),
        pipeline_version_id: task.pipeline().pipeline_version_id(),
        stage_id: task.current_stage_id().context("stage")?.clone(),
        stage_visit: task.current_stage_visit().context("visit")?.get(),
    };
    let thread = open(fixture, running.run.require_employee_id()?).await?;
    let id:Uuid=fixture.execute(CommandName::SendEmployeeMessage,message(thread,1,
        json!({"kind":"exact_run","context":context,"run_id":running.run.id,
            "fencing_token":running.run.lease_fencing_token,"environment_epoch":running.run.environment_epoch})))
        .await?.resource.context("message")?.id.parse()?;
    fixture
        .execute(CommandName::StopProjectExecution, json!({}))
        .await?;
    if let Backend::Postgres(pool) = &fixture.backend {
        let store = PostgresStore::from_pool(pool.clone());
        let mut tx = store.begin().await?;
        tx.lock_project(fixture.project_id).await?;
        assert_eq!(
            tx.pending_task_messages(fixture.project_id, &context)
                .await?,
            vec![id]
        );
        tx.commit().await?;
    }
    fixture
        .execute(
            CommandName::WaiveMessageRequirement,
            json!({"message_id":id,
        "reason":"The targeted Run stopped; management will issue replacement instructions."}),
        )
        .await?;
    if let Backend::Postgres(pool) = &fixture.backend {
        let store = PostgresStore::from_pool(pool.clone());
        let mut tx = store.begin().await?;
        tx.lock_project(fixture.project_id).await?;
        assert!(
            tx.pending_task_messages(fixture.project_id, &context)
                .await?
                .is_empty()
        );
        tx.commit().await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM employee_message_receipts WHERE message_id=$1",
        )
        .bind(id)
        .fetch_one(pool)
        .await?;
        assert_eq!(count, 0, "waiver is not an Employee receipt");
    }
    Ok(())
}

pub async fn external_stages_reject_execution_directed_messages(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let employee = employees::create(&fixture, "Discussing Bob").await?;
    let thread = open(&fixture, employee).await?;
    let mut definition = forge_testkit::m0::single_stage_pipeline();
    definition["stages"][0]["executor_kind"] = json!("human");
    let version = fixture.pipeline(definition).await?;
    let task_id = fixture.create_task(version, "Human decision").await?;
    fixture
        .task_command(CommandName::ApproveTask, task_id, json!({}))
        .await?;
    let task = fixture.task(task_id).await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let command=fixture.envelope(CommandName::SendEmployeeMessage,revision,message(thread,1,
        json!({"kind":"task_execution","context":{"task_id":task_id,"pipeline_version_id":version,
            "stage_id":"work","stage_visit":task.current_stage_visit().context("visit")?.get()}})),"non-employee-stage");
    let error = fixture
        .assert_unchanged_after(&command, &fixture.context)
        .await?;
    assert!(matches!(
        error,
        forge_application::CommandError::InvalidTransport {
            field: "message.target",
            ..
        }
    ));
    Ok(())
}
