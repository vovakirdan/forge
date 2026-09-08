//! Canonical management queue; callers retain the Project lock through commit.
use crate::{StorageError, StorageTransaction, database_timestamp, encode_snapshot, u64_to_i64};
use forge_domain::{
    ProjectId,
    resolution::{
        Escalation, ResolutionAssignment, ResolutionAssignmentState, Resolver, ResolverRoute,
    },
};
use uuid::Uuid;

impl StorageTransaction<'_> {
    pub async fn communication_escalation_is_current(
        &mut self,
        escalation: &Escalation,
    ) -> Result<bool, StorageError> {
        let Some(source) = escalation.source.communication() else {
            return Ok(false);
        };
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r JOIN communication_assignments c ON c.id=r.communication_assignment_id AND c.project_id=r.project_id WHERE r.id=$1 AND r.project_id=$2 AND r.purpose='communication' AND r.lease_fencing_token=$3 AND r.environment_epoch=$4 AND c.id=$5 AND c.thread_id=$6 AND c.source_message_id=$7 AND c.state='held' AND c.attempt_number=r.attempt_number)")
            .bind(source.run_id).bind(escalation.project_id.as_uuid()).bind(u64_to_i64(source.fencing_token,"source.fence")?).bind(u64_to_i64(source.environment_epoch,"source.epoch")?)
            .bind(source.assignment.assignment_id).bind(source.assignment.thread_id).bind(source.assignment.source_message_id).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn load_escalation_for_wait(
        &mut self,
        project: ProjectId,
        task: forge_domain::TaskId,
        wait: forge_domain::WaitConditionId,
    ) -> Result<Option<Escalation>, StorageError> {
        let value:Option<String>=sqlx::query_scalar("SELECT canonical_snapshot::text FROM escalations WHERE project_id=$1 AND task_id=$2 AND wait_condition_id=$3 FOR UPDATE")
            .bind(project.as_uuid()).bind(task.as_uuid()).bind(wait.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        value
            .map(|value| decode(&value, "escalation", Escalation::validate_snapshot))
            .transpose()
    }
    pub async fn load_resolver_route(
        &mut self,
        project: ProjectId,
        key: &str,
    ) -> Result<Option<ResolverRoute>, StorageError> {
        let value:Option<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM resolver_routes WHERE project_id=$1 AND route_key=$2 FOR UPDATE")
            .bind(project.as_uuid()).bind(key).fetch_optional(&mut *self.transaction).await?;
        value
            .map(|value| {
                decode::<ResolverRoute>(&value, "resolver_route", ResolverRoute::validate_snapshot)
            })
            .transpose()
    }
    pub async fn save_resolver_route(
        &mut self,
        route: &ResolverRoute,
        expected: Option<u64>,
    ) -> Result<(), StorageError> {
        route.validate_snapshot().map_err(|_| invalid())?;
        let snapshot = encode_snapshot(route, "resolver_route")?;
        let revision = u64_to_i64(route.revision, "route.revision")?;
        let changed = if let Some(expected) = expected {
            if route.revision != expected.checked_add(1).ok_or_else(invalid)? {
                return Err(invalid());
            }
            sqlx::query("UPDATE resolver_routes SET revision=$3,canonical_snapshot=$4::jsonb WHERE project_id=$1 AND route_key=$2 AND revision=$5")
                .bind(route.project_id.as_uuid()).bind(&route.key).bind(revision).bind(snapshot).bind(u64_to_i64(expected,"route.expected")?).execute(&mut *self.transaction).await?
        } else {
            if route.revision != 1 {
                return Err(invalid());
            }
            sqlx::query("INSERT INTO resolver_routes(project_id,route_key,revision,canonical_snapshot) VALUES($1,$2,$3,$4::jsonb)")
                .bind(route.project_id.as_uuid()).bind(&route.key).bind(revision).bind(snapshot).execute(&mut *self.transaction).await?
        };
        if changed.rows_affected() != 1 {
            return Err(invalid());
        }
        Ok(())
    }
    pub async fn load_escalation(&mut self, id: Uuid) -> Result<Option<Escalation>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM escalations WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|value| decode(&value, "escalation", Escalation::validate_snapshot))
            .transpose()
    }
    pub async fn save_escalation(
        &mut self,
        value: &Escalation,
        expected: Option<u64>,
    ) -> Result<(), StorageError> {
        value.validate_snapshot().map_err(|_| invalid())?;
        self.validate_escalation_source(value).await?;
        if let Some(expected) = expected {
            let prior = self.load_escalation(value.id).await?.ok_or_else(invalid)?;
            if prior.revision != expected
                || value.generation < prior.generation
                || value.generation > prior.generation.checked_add(1).ok_or_else(invalid)?
            {
                return Err(invalid());
            }
            let mut permitted = prior;
            permitted.generation = value.generation;
            permitted.next_candidate = value.next_candidate;
            permitted
                .advance(value.state.clone())
                .map_err(|_| invalid())?;
            if &permitted != value {
                return Err(invalid());
            }
        }
        let snapshot = encode_snapshot(value, "escalation")?;
        let changed = if let Some(expected) = expected {
            sqlx::query("UPDATE escalations SET revision=$3,generation=$4,escalation_state=$5,canonical_snapshot=$6::jsonb WHERE id=$1 AND project_id=$2 AND revision=$7")
                .bind(value.id).bind(value.project_id.as_uuid()).bind(u64_to_i64(value.revision,"escalation.revision")?)
                .bind(u64_to_i64(value.generation,"escalation.generation")?).bind(value.state.key()).bind(snapshot)
                .bind(u64_to_i64(expected,"escalation.expected")?).execute(&mut *self.transaction).await?
        } else {
            sqlx::query("INSERT INTO escalations(id,project_id,task_id,wait_condition_id,revision,generation,escalation_state,canonical_snapshot) VALUES($1,$2,$3,$4,$5,$6,$7,$8::jsonb)")
                .bind(value.id).bind(value.project_id.as_uuid()).bind(value.source.task().map(|source|source.task_id.as_uuid())).bind(value.source.task().map(|source|source.wait_condition_id.as_uuid()))
                .bind(u64_to_i64(value.revision,"escalation.revision")?).bind(u64_to_i64(value.generation,"escalation.generation")?)
                .bind(value.state.key()).bind(snapshot).execute(&mut *self.transaction).await?
        };
        if changed.rows_affected() != 1 {
            return Err(invalid());
        }
        Ok(())
    }
    pub async fn load_resolution_assignment(
        &mut self,
        id: Uuid,
    ) -> Result<Option<ResolutionAssignment>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM resolution_assignments WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|value| {
                decode(
                    &value,
                    "resolution_assignment",
                    ResolutionAssignment::validate_snapshot,
                )
            })
            .transpose()
    }

    async fn validate_escalation_source(&mut self, value: &Escalation) -> Result<(), StorageError> {
        if let Some(source) = value.source.communication() {
            if value.created_by.kind() != forge_domain::ActorKind::Employee {
                return Err(invalid());
            }
            let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r JOIN communication_assignments c ON c.id=r.communication_assignment_id AND c.project_id=r.project_id WHERE r.id=$1 AND r.project_id=$2 AND r.purpose='communication' AND r.lease_fencing_token=$3 AND r.environment_epoch=$4 AND r.employee_id=$5 AND c.id=$6 AND c.thread_id=$7 AND c.source_message_id=$8)")
                .bind(source.run_id).bind(value.project_id.as_uuid()).bind(u64_to_i64(source.fencing_token,"source.fence")?).bind(u64_to_i64(source.environment_epoch,"source.epoch")?)
                .bind(value.created_by.id().as_uuid()).bind(source.assignment.assignment_id).bind(source.assignment.thread_id).bind(source.assignment.source_message_id)
                .fetch_one(&mut *self.transaction).await?;
            if !valid {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub async fn insert_resolution_assignment(
        &mut self,
        value: &ResolutionAssignment,
    ) -> Result<(), StorageError> {
        value.validate_snapshot().map_err(|_| invalid())?;
        let owner = self
            .load_escalation(value.escalation_id)
            .await?
            .ok_or_else(invalid)?;
        value.validate_owner(&owner).map_err(|_| invalid())?;
        if value.state != ResolutionAssignmentState::Active {
            return Err(invalid());
        }
        let employee = match value.resolver {
            Resolver::Human => None,
            Resolver::Employee { employee_id } => Some(employee_id.as_uuid()),
        };
        sqlx::query("INSERT INTO resolution_assignments(id,project_id,escalation_id,generation,employee_id,expires_at,assignment_state,canonical_snapshot) VALUES($1,$2,$3,$4,$5,$6,'active',$7::jsonb)")
            .bind(value.id).bind(value.project_id.as_uuid()).bind(value.escalation_id).bind(u64_to_i64(value.lease.generation,"assignment.generation")?)
            .bind(employee).bind(value.lease.expires_at.map(database_timestamp)).bind(encode_snapshot(value,"resolution_assignment")?).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn update_resolution_assignment(
        &mut self,
        value: &ResolutionAssignment,
    ) -> Result<(), StorageError> {
        value.validate_snapshot().map_err(|_| invalid())?;
        if value.state == ResolutionAssignmentState::Active {
            return Err(invalid());
        }
        let result=sqlx::query("UPDATE resolution_assignments SET assignment_state=$3,canonical_snapshot=$4::jsonb WHERE id=$1 AND project_id=$2 AND assignment_state='active'")
            .bind(value.id).bind(value.project_id.as_uuid()).bind(value.state.key()).bind(encode_snapshot(value,"resolution_assignment")?).execute(&mut *self.transaction).await?;
        if result.rows_affected() != 1 {
            return Err(invalid());
        }
        Ok(())
    }
}

fn decode<T: serde::de::DeserializeOwned>(
    value: &str,
    aggregate: &'static str,
    validate: impl FnOnce(&T) -> Result<(), forge_domain::DomainError>,
) -> Result<T, StorageError> {
    let value = serde_json::from_str(value)
        .map_err(|source| StorageError::Snapshot { aggregate, source })?;
    validate(&value).map_err(|_| invalid())?;
    Ok(value)
}
fn invalid() -> StorageError {
    StorageError::InvalidInput {
        reason: "invalid resolution scope, revision, or state".into(),
    }
}
