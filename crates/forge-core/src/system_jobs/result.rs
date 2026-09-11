use super::{digest, invalid};
use crate::{CoreError, CoreService};
use forge_domain::{
    CommandId, EmployeeId, Project, Timestamp,
    knowledge::{DerivedMemoryEntry, DerivedMemoryKind, KnowledgeSourceRef, SourceCoverage},
    runtime::SystemJobRunSpec,
    system_job::SystemJobKind,
};
use forge_storage::StorageTransaction;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SystemJobResult {
    source_digest: String,
    entries: Vec<GeneratedEntry>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GeneratedEntry {
    subject: DerivedMemoryKind,
    markdown: String,
    source_refs: Vec<KnowledgeSourceRef>,
}

impl SystemJobResult {
    pub(crate) fn decode(value: Value, spec: &SystemJobRunSpec) -> Result<Self, CoreError> {
        if serde_json::to_vec(&value)
            .map_err(|_| invalid("invalid result"))?
            .len()
            > spec.max_result_bytes as usize
        {
            return Err(invalid("result exceeds pinned byte allowance"));
        }
        let result: Self = serde_json::from_value(value)
            .map_err(|_| invalid("result does not match the schema"))?;
        let source_bytes = serde_json::to_vec(&spec.input.context)
            .map_err(|_| invalid("invalid frozen source"))?;
        if result.source_digest != spec.input.source_digest
            || result.source_digest != digest(&source_bytes)
            || result.entries.is_empty()
            || result.entries.len() > 18
        {
            return Err(invalid("result source identity or entry count is invalid"));
        }
        let refs: Vec<KnowledgeSourceRef> = serde_json::from_value(
            spec.input
                .context
                .get("source_refs")
                .cloned()
                .ok_or_else(|| invalid("input has no source allowlist"))?,
        )
        .map_err(|_| invalid("invalid source allowlist"))?;
        let employees: Vec<EmployeeId> = serde_json::from_value(
            spec.input
                .context
                .get("allowed_employee_ids")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|_| invalid("invalid employee allowlist"))?;
        for (i, entry) in result.entries.iter().enumerate() {
            if entry.markdown.trim().is_empty()
                || entry.markdown.len() > 65536
                || entry.source_refs.is_empty()
                || entry.source_refs.len() > 128
                || result.entries[..i]
                    .iter()
                    .any(|prior| prior.subject == entry.subject)
            {
                return Err(invalid("empty, duplicated or oversized result entry"));
            }
            for (index, source) in entry.source_refs.iter().enumerate() {
                source.validate()?;
                if !refs.contains(source) || entry.source_refs[..index].contains(source) {
                    return Err(invalid("result cites a source outside its frozen input"));
                }
            }
            match spec.assignment.kind {
                SystemJobKind::Summarization => {
                    if entry.subject.task_id() != spec.input.source_task_id {
                        return Err(invalid("summary must remain attached to its source Task"));
                    }
                    if let DerivedMemoryKind::EmployeeMemoryEntry { employee_id, .. } =
                        entry.subject
                        && !employees.contains(&employee_id)
                    {
                        return Err(invalid("summary targets an unrelated Employee"));
                    }
                }
                SystemJobKind::Onboarding => {
                    if result.entries.len() != 1
                        || !matches!(entry.subject,DerivedMemoryKind::EmployeeMemoryEntry{employee_id,task_id:None} if Some(employee_id)==spec.input.target_employee_id)
                    {
                        return Err(invalid(
                            "onboarding may only create the target Employee's personal note",
                        ));
                    }
                }
            }
        }
        if spec.assignment.kind == SystemJobKind::Summarization
            && !result
                .entries
                .iter()
                .any(|e| matches!(e.subject, DerivedMemoryKind::TaskSummary { .. }))
        {
            return Err(invalid("summarization requires a TaskSummary"));
        }
        Ok(result)
    }
    pub(crate) async fn validate_sources(
        &self,
        tx: &mut StorageTransaction<'_>,
        spec: &SystemJobRunSpec,
    ) -> Result<(), CoreError> {
        for entry in &self.entries {
            tx.validate_knowledge_sources(
                spec.project_id,
                entry.subject.scope(),
                &entry.source_refs,
            )
            .await?;
        }
        Ok(())
    }
}

impl CoreService {
    pub(crate) async fn materialize_system_job_result(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &Project,
        spec: &SystemJobRunSpec,
        result: Value,
        now: Timestamp,
    ) -> Result<Vec<Value>, CoreError> {
        let result = SystemJobResult::decode(result, spec)?;
        result.validate_sources(tx, spec).await?;
        let coverage = if spec.input.covered_sequence > 0 {
            let first = spec
                .input
                .context
                .get("events")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| e.get("sequence").and_then(Value::as_u64))
                .min()
                .unwrap_or(spec.input.covered_sequence);
            Some(SourceCoverage {
                first_event_sequence: first,
                last_event_sequence: spec.input.covered_sequence,
            })
        } else {
            None
        };
        let mut receipts = Vec::new();
        for output in result.entries {
            let key =
                serde_json::to_string(&output.subject).map_err(|_| invalid("invalid subject"))?;
            let (id, revision) = tx
                .next_system_job_memory_revision(spec.assignment.job_id, &key)
                .await?;
            let entry = DerivedMemoryEntry {
                id,
                project_id: project.id(),
                revision,
                subject: output.subject,
                content_hash: digest(output.markdown.as_bytes()),
                markdown: output.markdown,
                source_refs: output.source_refs,
                coverage,
                created_by_job_id: spec.assignment.job_id,
                created_at: now,
                withdrawn: false,
            };
            self.persist_derived_memory(tx, project, &entry, CommandId::new(), now)
                .await?;
            receipts.push(serde_json::json!({"id":id,"revision":revision,"kind":entry.subject,"content_hash":entry.content_hash}));
        }
        Ok(receipts)
    }
}
