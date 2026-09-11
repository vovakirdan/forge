-- Future-only Task settings have their own revision and immutable history.
CREATE TABLE task_git_source_policies (
 project_id UUID NOT NULL,
 task_id UUID NOT NULL,
 revision BIGINT NOT NULL CHECK (revision >= 2),
 policy JSONB NOT NULL CHECK (jsonb_typeof(policy) = 'object'),
 PRIMARY KEY(project_id, task_id, revision),
 FOREIGN KEY(project_id, task_id) REFERENCES tasks(project_id, id),
 CHECK (((policy->>'mode'='latest_target' AND policy='{"mode":"latest_target"}'::jsonb)
   OR (policy->>'mode'='pinned_commit' AND policy->>'commit' ~ '^([a-f0-9]{40}|[a-f0-9]{64})$'
       AND policy->>'commit' !~ '^0+$'
       AND policy=jsonb_build_object('mode','pinned_commit','commit',policy->>'commit'))) IS TRUE)
);
CREATE TRIGGER task_git_source_policy_immutable BEFORE UPDATE OR DELETE ON task_git_source_policies
 FOR EACH ROW EXECUTE FUNCTION forge_reject_repository_mutation();

CREATE TABLE run_git_source_selections (
 run_id UUID PRIMARY KEY REFERENCES runs(id),
 descriptor JSONB NOT NULL CHECK(jsonb_typeof(descriptor)='object')
);
CREATE TRIGGER run_git_source_selection_immutable BEFORE UPDATE OR DELETE ON run_git_source_selections
 FOR EACH ROW EXECUTE FUNCTION forge_reject_repository_mutation();

ALTER TABLE tasks DROP CONSTRAINT task_git_projection_present;
ALTER TABLE tasks ADD CONSTRAINT task_git_projection_present CHECK (
 project_repository_id IS NULL OR task_work_surface_id IS NOT NULL
);
CREATE OR REPLACE FUNCTION forge_validate_task_git_binding() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE binding JSONB; registered JSONB; initial JSONB;
BEGIN
 binding := NEW.canonical_snapshot->'work_surface'->'git';
 IF TG_OP='UPDATE' AND OLD.project_repository_id IS NOT NULL
   AND OLD.canonical_snapshot->'work_surface' IS DISTINCT FROM NEW.canonical_snapshot->'work_surface' THEN
  RAISE EXCEPTION 'Task Git binding is immutable';
 END IF;
 IF NEW.project_repository_id IS NULL THEN
  IF binding IS NOT NULL THEN RAISE EXCEPTION 'Task Git projection is missing'; END IF;
  RETURN NEW;
 END IF;
 SELECT canonical_snapshot INTO registered FROM project_repositories
  WHERE id=NEW.project_repository_id AND project_id=NEW.project_id;
 initial := binding->'initial_base';
 IF registered IS NULL OR binding IS NULL
  OR binding->>'repository_id' IS DISTINCT FROM NEW.project_repository_id::text
  OR binding->>'surface_id' IS DISTINCT FROM NEW.task_work_surface_id::text
  OR binding->>'source' IS DISTINCT FROM registered->>'source'
  OR binding->>'target_ref' IS DISTINCT FROM registered->>'target_ref'
  OR (((jsonb_typeof(initial)='string' AND binding->>'initial_base'=NEW.base_sha
       AND NEW.base_sha ~ '^([a-f0-9]{40}|[a-f0-9]{64})$' AND NEW.base_sha !~ '^0+$')
   OR (jsonb_typeof(initial)='object' AND initial->>'kind'='unborn'
       AND initial->>'object_format' IN ('sha1','sha256') AND NEW.base_sha IS NULL)) IS NOT TRUE) THEN
  RAISE EXCEPTION 'Task Git binding violates repository or surface scope';
 END IF;
 RETURN NEW;
END; $$;

ALTER TABLE runs DROP CONSTRAINT runs_owner_exact;
ALTER TABLE runs ADD CONSTRAINT runs_owner_exact CHECK (
 (purpose='task_stage' AND task_id IS NOT NULL AND queue_entry_id IS NOT NULL AND stage_id IS NOT NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NULL AND run_spec_version IN (1,2,6))
 OR (purpose='communication' AND task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND communication_assignment_id IS NOT NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NULL AND run_spec_version=3)
 OR (purpose='resolution' AND task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NOT NULL AND hook_invocation_id IS NULL AND run_spec_version=4)
 OR (purpose='hook' AND task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NOT NULL AND run_spec_version=5)
);
