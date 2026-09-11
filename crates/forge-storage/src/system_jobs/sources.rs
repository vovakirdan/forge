//! Canonical outbox watermarks and bounded, attributable semantic inputs.
use super::*;
use forge_domain::knowledge::{KnowledgePage, KnowledgeSourceRef};

// Both queue coverage and frozen evidence use the same canonical Task owner.
// Supervisor Git inspection nests its Run under `result`; incidents use root
// `run_id`. Restrict Run lookup to these semantic events and to their Project.
// Compare UUIDs as text so malformed optional payload fields cannot poison a cursor.
const EVENT_TASK: &str = "COALESCE(
    CASE WHEN e.aggregate_type='task' THEN e.aggregate_id::text END,
    e.payload->>'task_id', e.payload->'data'->>'task_id',
    (SELECT r.task_id::text FROM runs r WHERE r.project_id=e.project_id
      AND e.event_type IN ('run_incident_raised','git_candidate_inspected')
      AND r.id::text=COALESCE(e.payload->>'run_id',e.payload->'result'->>'run_id'))
)";

impl PostgresStore {
    pub async fn system_job_project_ids(&self) -> Result<Vec<ProjectId>, StorageError> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT project_id FROM project_system_job_settings ORDER BY project_id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().map(ProjectId::from).collect())
    }
}
impl StorageTransaction<'_> {
    /// Establishes the opt-in boundary. Historical Tasks require an explicit backfill command.
    pub async fn initialize_system_job_cursor(
        &mut self,
        project: ProjectId,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO system_job_event_cursors(project_id,last_sequence) SELECT $1,coalesce(max(project_sequence),0) FROM event_log WHERE project_id=$1 ON CONFLICT DO NOTHING")
            .bind(project.as_uuid()).execute(&mut *self.transaction).await?;
        Ok(())
    }
    /// Reading an outbox intent does not depend on NATS delivery timing. This cursor
    /// and the job queue advance in one Project transaction; a crash replays neither partially.
    pub async fn ingest_system_job_events(
        &mut self,
        project: ProjectId,
        delay: u32,
    ) -> Result<usize, StorageError> {
        let last: Option<i64> = sqlx::query_scalar(
            "SELECT last_sequence FROM system_job_event_cursors WHERE project_id=$1 FOR UPDATE",
        )
        .bind(project.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        let Some(last) = last else { return Ok(0) };
        let query = format!(
            "SELECT e.project_sequence,e.event_type,({EVENT_TASK}) AS task_id,coalesce(e.payload->'scope'->>'visibility','project') AS visibility FROM event_log e JOIN outbox o ON o.project_id=e.project_id AND o.event_id=e.id WHERE e.project_id=$1 AND e.project_sequence>$2 ORDER BY e.project_sequence LIMIT 256"
        );
        let rows = sqlx::query(&query)
            .bind(project.as_uuid())
            .bind(last)
            .fetch_all(&mut *self.transaction)
            .await?;
        let mut count = 0;
        let mut through = last;
        for row in rows {
            let sequence: i64 = row.try_get("project_sequence")?;
            through = sequence;
            let event: String = row.try_get("event_type")?;
            if row.try_get::<String, _>("visibility")? != "project"
                || !matches!(
                    event.as_str(),
                    "task_artifact_attached"
                        | "task_stage_advanced"
                        | "task_completed"
                        | "task_cancelled"
                        | "task_waiting"
                        | "task_resumed"
                        | "git_candidate_inspected"
                        | "candidate_reviewed"
                        | "run_incident_raised"
                )
            {
                continue;
            }
            let task = row
                .try_get::<Option<String>, _>("task_id")?
                .and_then(|id| Uuid::parse_str(&id).ok());
            if let Some(task) = task {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM tasks WHERE project_id=$1 AND id=$2)",
                )
                .bind(project.as_uuid())
                .bind(task)
                .fetch_one(&mut *self.transaction)
                .await?;
                if exists {
                    self.enqueue_task_summary(
                        project,
                        TaskId::from(task),
                        u64::try_from(sequence).map_err(|_| invalid())?,
                        delay,
                    )
                    .await?;
                    count += 1;
                }
            }
        }
        sqlx::query("UPDATE system_job_event_cursors SET last_sequence=$2 WHERE project_id=$1")
            .bind(project.as_uuid())
            .bind(through)
            .execute(&mut *self.transaction)
            .await?;
        Ok(count)
    }
    pub async fn system_job_source_context(
        &mut self,
        job: &SystemJobRecord,
    ) -> Result<Value, StorageError> {
        match job.kind {
            SystemJobKind::Summarization => self.summary_context(job).await,
            SystemJobKind::Onboarding => self.onboarding_context(job).await,
        }
    }
    async fn summary_context(&mut self, job: &SystemJobRecord) -> Result<Value, StorageError> {
        let task = job.source_task_id.ok_or_else(invalid)?;
        let stored = self.lock_task(task).await?.ok_or_else(invalid)?;
        if stored.task.project_id() != job.project_id {
            return Err(invalid());
        }
        let query = format!(
            "SELECT jsonb_build_object('id',e.id,'sequence',e.project_sequence,'event_type',e.event_type,'actor',e.actor,'occurred_at',e.occurred_at,'payload_preview',left(e.payload::text,2048),'truncated',length(e.payload::text)>2048) FROM event_log e WHERE e.project_id=$1 AND e.project_sequence<=$3 AND ({EVENT_TASK})=$2::uuid::text AND coalesce(e.payload->'scope'->>'visibility','project')='project' AND e.event_type NOT IN ('tool_gateway_call_allowed','tool_gateway_call_denied','run_progress_reported','run_observed','run_observation_ignored','derived_memory_changed','run_proxy_key_changed','run_evidence_recorded') ORDER BY e.project_sequence DESC LIMIT 24"
        );
        let events: Vec<Value> = sqlx::query_scalar(&query)
            .bind(job.project_id.as_uuid())
            .bind(task.as_uuid())
            .bind(u64_to_i64(job.covered_sequence, "system_job.coverage")?)
            .fetch_all(&mut *self.transaction)
            .await?;
        let artifacts:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'kind',kind,'title',title,'stage_id',stage_id,'body_preview',left(body_json::text,4096),'truncated',length(body_json::text)>4096) FROM artifacts WHERE project_id=$1 AND task_id=$2 ORDER BY id DESC LIMIT 12")
            .bind(job.project_id.as_uuid()).bind(task.as_uuid()).fetch_all(&mut *self.transaction).await?;
        let handoffs:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'summary_preview',left(body::text,4096),'truncated',length(body::text)>4096) FROM task_handoffs WHERE project_id=$1 AND task_id=$2 ORDER BY id DESC LIMIT 1")
            .bind(job.project_id.as_uuid()).bind(task.as_uuid()).fetch_all(&mut *self.transaction).await?;
        let employees:Vec<Uuid>=sqlx::query_scalar("SELECT DISTINCT employee_id FROM runs WHERE project_id=$1 AND task_id=$2 AND employee_id IS NOT NULL ORDER BY employee_id LIMIT 16")
            .bind(job.project_id.as_uuid()).bind(task.as_uuid()).fetch_all(&mut *self.transaction).await?;
        let mut refs = Vec::new();
        for (values, kind) in [
            (&events, "event"),
            (&artifacts, "artifact"),
            (&handoffs, "task_handoff"),
        ] {
            for v in values {
                let id = v.get("id").cloned().ok_or_else(invalid)?;
                refs.push(match kind {
                    "event" => serde_json::json!({"kind":"event","event_id":id}),
                    "artifact" => serde_json::json!({"kind":"artifact","artifact_id":id}),
                    _ => serde_json::json!({"kind":"task_handoff","handoff_id":id}),
                });
            }
        }
        // Deserializing catches accidental drift from the canonical source-ref wire format.
        let _: Vec<KnowledgeSourceRef> = decode(Value::Array(refs.clone()))?;
        Ok(
            serde_json::json!({"task":{"id":task,"key":stored.task.key(),"title":stored.task.spec().title(),"description":stored.task.spec().description(),"stage":stored.task.current_stage_id(),"lifecycle":stored.task.lifecycle(),"revision":stored.task.revision().get()},
            "events":events,"artifacts":artifacts,"handoffs":handoffs,"allowed_employee_ids":employees,"source_refs":refs,"source_window":"bounded_latest_canonical_evidence_not_full_history"}),
        )
    }
    async fn onboarding_context(&mut self, job: &SystemJobRecord) -> Result<Value, StorageError> {
        let employee = job.target_employee_id.ok_or_else(invalid)?;
        let employee_value:Option<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'name',name,'roles',roles,'skills',skills,'revision',revision,'stage_eligibility',stage_eligibility) FROM employees WHERE project_id=$1 AND id=$2 AND employee_state='active'")
            .bind(job.project_id.as_uuid()).bind(employee.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        let employee_value = employee_value.ok_or_else(invalid)?;
        let binding = self.runtime_binding(employee).await?.ok_or_else(invalid)?;
        let pages:Vec<Value>=sqlx::query_scalar("SELECT canonical_snapshot FROM knowledge_pages WHERE project_id=$1 AND status='published' AND kind IN ('introduction','architecture','policy','decision') ORDER BY id")
            .bind(job.project_id.as_uuid()).fetch_all(&mut *self.transaction).await?;
        let mut refs = Vec::new();
        for page in &pages {
            let p: KnowledgePage = decode(page.clone())?;
            refs.push(KnowledgeSourceRef::KnowledgePage {
                page_id: p.id,
                revision: p.revision,
            });
        }
        let event:Option<Uuid>=sqlx::query_scalar("SELECT id FROM event_log WHERE project_id=$1 AND aggregate_type='employee' AND aggregate_id=$2 AND event_type='employee_created' ORDER BY project_sequence LIMIT 1")
            .bind(job.project_id.as_uuid()).bind(employee.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        if let Some(id) = event {
            refs.push(KnowledgeSourceRef::Event {
                event_id: id.into(),
            });
        }
        let goal: Option<Value> =
            sqlx::query_scalar("SELECT properties->'current_goal' FROM projects WHERE id=$1")
                .bind(job.project_id.as_uuid())
                .fetch_one(&mut *self.transaction)
                .await?;
        Ok(
            serde_json::json!({"employee":employee_value,"system_prompt":binding.system_prompt,"employee_prompt":binding.employee_prompt,
            "execution_profile_id":binding.execution_profile.id(),"execution_profile_revision":binding.execution_profile.revision(),
            "capabilities":binding.execution_profile.capability_profile(),"knowledge_pages":pages,"current_goal":goal,"source_refs":refs}),
        )
    }
}
