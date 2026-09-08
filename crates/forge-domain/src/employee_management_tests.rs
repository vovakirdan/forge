use super::*;
use crate::ActorId;

fn employee() -> Employee {
    Employee::new(
        EmployeeId::new(),
        ProjectId::new(),
        "Bob",
        EmployeeRole::new("developer").expect("role"),
        StageEligibility::Any,
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    )
    .expect("employee")
}

#[test]
fn legacy_employee_snapshot_defaults_revision_and_capacity_to_one() {
    let original = employee();
    let mut json = serde_json::to_value(&original).expect("serialize");
    let fields = json.as_object_mut().expect("object");
    fields.remove("revision");
    fields.remove("max_concurrent_runs");
    let restored: Employee = serde_json::from_value(json).expect("legacy snapshot");
    assert_eq!(restored, original);
}

#[test]
fn amendments_advance_revision_without_changing_frozen_prior_value() {
    let mut employee = employee();
    let prior = employee.clone();
    employee
        .amend(
            &EmployeeAmendment {
                role: Some(EmployeeRole::new("reviewer").expect("role")),
                max_concurrent_runs: NonZeroU16::new(3),
                ..EmployeeAmendment::default()
            },
            employee.updated_at(),
        )
        .expect("amend");
    assert_eq!(
        (employee.revision(), employee.max_concurrent_runs()),
        (2, 3)
    );
    assert_eq!((prior.revision(), prior.max_concurrent_runs()), (1, 1));
    assert_eq!(prior.role().as_str(), "developer");
}

#[test]
fn invalid_amendment_does_not_partially_change_employee() {
    let mut employee = employee();
    let prior = employee.clone();
    let error = employee.amend(
        &EmployeeAmendment {
            name: Some(" ".into()),
            max_concurrent_runs: NonZeroU16::new(3),
            ..EmployeeAmendment::default()
        },
        employee.updated_at(),
    );
    assert!(error.is_err());
    assert_eq!(employee, prior);
}

#[test]
fn lifecycle_mutations_have_durable_revisions_and_retirement_is_final() {
    let mut employee = employee();
    let now = employee.updated_at();
    employee.disable(now).expect("disable");
    employee.enable(now).expect("enable");
    employee.retire(now).expect("retire");
    assert_eq!(
        (employee.revision(), employee.state()),
        (4, EmployeeState::Retired)
    );
    let prior = employee.clone();
    assert!(employee.enable(now).is_err());
    assert!(employee.disable(now).is_err());
    assert!(
        employee
            .amend(
                &EmployeeAmendment {
                    name: Some("Another".into()),
                    ..EmployeeAmendment::default()
                },
                now
            )
            .is_err()
    );
    assert_eq!(employee, prior);
}

#[test]
fn zero_capacity_is_not_a_valid_snapshot_or_amendment() {
    let mut json = serde_json::to_value(employee()).expect("employee");
    json["max_concurrent_runs"] = serde_json::json!(0);
    assert!(serde_json::from_value::<Employee>(json).is_err());
    assert!(serde_json::from_str::<EmployeeAmendment>("{\"max_concurrent_runs\":0}").is_err());
}

#[test]
fn exhausted_revision_does_not_partially_apply_a_lifecycle_change() {
    let mut employee = employee();
    employee.revision = u64::MAX;
    let prior = employee.clone();
    assert!(employee.disable(employee.updated_at()).is_err());
    assert_eq!(employee, prior);
}
