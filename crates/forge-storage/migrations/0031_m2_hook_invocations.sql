-- Hook execution has no Employee, Task writer lease, provider, or credentials.
CREATE TABLE hook_invocations (
 id UUID PRIMARY KEY,
 project_id UUID NOT NULL,
 task_id UUID NOT NULL,
 pipeline_version_id UUID NOT NULL,
 stage_id TEXT NOT NULL,
 stage_visit BIGINT NOT NULL CHECK(stage_visit>0),
 candidate_proposal_id UUID,
 hook_version_id UUID NOT NULL,
 run_id UUID UNIQUE,
 state TEXT NOT NULL CHECK(state IN ('running','completed','held')),
 core_instance UUID NOT NULL,
 expected_task_revision BIGINT NOT NULL CHECK(expected_task_revision>0),
 spec JSONB NOT NULL,
 result JSONB,
 artifact_id UUID REFERENCES artifacts(id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(project_id,id),
 FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id),
 FOREIGN KEY(pipeline_version_id) REFERENCES pipeline_versions(id),
 FOREIGN KEY(project_id,hook_version_id) REFERENCES project_hook_versions(project_id,id),
 FOREIGN KEY(project_id,candidate_proposal_id) REFERENCES git_stage_proposals(project_id,id),
 FOREIGN KEY(project_id,run_id) REFERENCES runs(project_id,id) DEFERRABLE INITIALLY DEFERRED,
 CHECK((spec->>'project_id'=project_id::text AND spec->'hook'->>'id'=hook_version_id::text AND
 ((run_id IS NOT NULL AND candidate_proposal_id IS NOT NULL AND spec->>'schema_version'='5'
  AND spec->'assignment'->>'invocation_id'=id::text
  AND spec->'assignment'->>'task_id'=task_id::text
  AND spec->'assignment'->>'pipeline_version_id'=pipeline_version_id::text
  AND spec->'assignment'->>'stage_id'=stage_id
  AND (spec->'assignment'->>'stage_visit')::bigint=stage_visit
  AND spec->'assignment'->>'candidate_proposal_id'=candidate_proposal_id::text
  AND spec->>'run_id'=run_id::text)
 OR (run_id IS NULL AND candidate_proposal_id IS NULL AND spec->>'schema_version'='1'
  AND spec->>'invocation_id'=id::text AND spec->>'task_id'=task_id::text
  AND spec->>'pipeline_version_id'=pipeline_version_id::text AND spec->>'stage_id'=stage_id
  AND (spec->>'stage_visit')::bigint=stage_visit AND NOT(spec ? 'run_id')
  AND NOT(spec ? 'candidate') AND result->>'verdict'='skipped'))) IS TRUE),
 CHECK(state<>'completed' OR (result IS NOT NULL AND artifact_id IS NOT NULL)),
 CHECK((run_id IS NOT NULL OR result->>'verdict'='skipped') IS TRUE)
);
CREATE UNIQUE INDEX hook_one_running_visit ON hook_invocations(task_id,pipeline_version_id,stage_id,stage_visit) WHERE state='running';
CREATE INDEX hook_candidate_acceptance ON hook_invocations(project_id,task_id,candidate_proposal_id,hook_version_id,stage_id);
CREATE FUNCTION forge_guard_hook_invocation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='INSERT' THEN
  IF NOT EXISTS(SELECT 1 FROM tasks t JOIN pipeline_versions v ON v.id=NEW.pipeline_version_id
   JOIN pipelines p ON p.id=v.pipeline_id
   JOIN project_hook_versions h ON h.id=NEW.hook_version_id
   WHERE t.id=NEW.task_id AND t.project_id=NEW.project_id AND t.pipeline_version_id=v.id
    AND p.project_id=NEW.project_id AND h.project_id=NEW.project_id AND h.canonical_snapshot=NEW.spec->'hook'
    AND (NEW.run_id IS NULL OR EXISTS(SELECT 1 FROM git_stage_proposals g WHERE g.id=NEW.candidate_proposal_id
      AND g.project_id=NEW.project_id AND g.task_id=t.id AND g.proposal_state='accepted'))) THEN
   RAISE EXCEPTION 'Hook source or immutable configuration scope mismatch';
  END IF;
  RETURN NEW;
 END IF;
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'Hook execution history cannot be deleted'; END IF;
 IF (NEW.id,NEW.project_id,NEW.task_id,NEW.pipeline_version_id,NEW.stage_id,NEW.stage_visit,
     NEW.candidate_proposal_id,NEW.hook_version_id,NEW.run_id,NEW.spec,NEW.core_instance,NEW.expected_task_revision,NEW.created_at)
  IS DISTINCT FROM
    (OLD.id,OLD.project_id,OLD.task_id,OLD.pipeline_version_id,OLD.stage_id,OLD.stage_visit,
     OLD.candidate_proposal_id,OLD.hook_version_id,OLD.run_id,OLD.spec,OLD.core_instance,OLD.expected_task_revision,OLD.created_at)
  OR OLD.state<>'running' OR NEW.state NOT IN ('completed','held') THEN
  RAISE EXCEPTION 'Hook identity and terminal results are immutable';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER hook_invocation_guard BEFORE INSERT OR UPDATE OR DELETE ON hook_invocations FOR EACH ROW EXECUTE FUNCTION forge_guard_hook_invocation();

ALTER TABLE leases ADD COLUMN hook_invocation_id UUID;
ALTER TABLE runs ADD COLUMN hook_invocation_id UUID;
ALTER TABLE run_environment_reservations ADD COLUMN hook_invocation_id UUID;
ALTER TABLE leases DROP CONSTRAINT leases_owner_exact;
ALTER TABLE leases ADD CONSTRAINT leases_owner_exact CHECK (
 (purpose='task_stage' AND task_id IS NOT NULL AND queue_entry_id IS NOT NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NULL)
 OR (purpose='communication' AND task_id IS NULL AND queue_entry_id IS NULL AND task_work_surface_id IS NULL AND communication_assignment_id IS NOT NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NULL)
 OR (purpose='resolution' AND task_id IS NULL AND queue_entry_id IS NULL AND task_work_surface_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NOT NULL AND hook_invocation_id IS NULL)
 OR (purpose='hook' AND task_id IS NULL AND queue_entry_id IS NULL AND task_work_surface_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NOT NULL)
);
ALTER TABLE leases ADD FOREIGN KEY(project_id,hook_invocation_id) REFERENCES hook_invocations(project_id,id);
ALTER TABLE leases ADD CONSTRAINT leases_hook_scope UNIQUE(project_id,id,hook_invocation_id);
ALTER TABLE runs DROP CONSTRAINT runs_owner_exact;
ALTER TABLE runs ADD CONSTRAINT runs_owner_exact CHECK (
 (purpose='task_stage' AND task_id IS NOT NULL AND queue_entry_id IS NOT NULL AND stage_id IS NOT NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NULL AND run_spec_version IN (1,2))
 OR (purpose='communication' AND task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND communication_assignment_id IS NOT NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NULL AND run_spec_version=3)
 OR (purpose='resolution' AND task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NOT NULL AND hook_invocation_id IS NULL AND run_spec_version=4)
 OR (purpose='hook' AND task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NOT NULL AND run_spec_version=5)
);
ALTER TABLE runs ADD FOREIGN KEY(project_id,lease_id,hook_invocation_id) REFERENCES leases(project_id,id,hook_invocation_id);
ALTER TABLE runs ADD CONSTRAINT runs_hook_scope UNIQUE(project_id,id,hook_invocation_id);
CREATE UNIQUE INDEX hook_invocation_run ON runs(hook_invocation_id) WHERE purpose='hook';
CREATE FUNCTION forge_guard_hook_run_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.purpose='hook' AND NOT EXISTS(SELECT 1 FROM hook_invocations h
   WHERE h.id=NEW.hook_invocation_id AND h.project_id=NEW.project_id
    AND h.run_id=NEW.id AND h.spec=NEW.run_spec) THEN
  RAISE EXCEPTION 'Hook RunSpec differs from its canonical invocation';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER hook_run_snapshot_guard BEFORE INSERT OR UPDATE ON runs FOR EACH ROW EXECUTE FUNCTION forge_guard_hook_run_snapshot();
ALTER TABLE run_environment_reservations DROP CONSTRAINT environment_owner_exact;
ALTER TABLE run_environment_reservations ADD CONSTRAINT environment_owner_exact CHECK (
 (purpose='task_stage' AND task_id IS NOT NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NULL)
 OR (purpose='communication' AND task_id IS NULL AND surface_id IS NULL AND communication_assignment_id IS NOT NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NULL)
 OR (purpose='resolution' AND task_id IS NULL AND surface_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NOT NULL AND hook_invocation_id IS NULL)
 OR (purpose='hook' AND task_id IS NULL AND surface_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NOT NULL)
);
ALTER TABLE run_environment_reservations ADD FOREIGN KEY(project_id,run_id,hook_invocation_id) REFERENCES runs(project_id,id,hook_invocation_id);
CREATE UNIQUE INDEX one_physical_hook_invocation ON run_environment_reservations(hook_invocation_id) WHERE released_at IS NULL AND purpose='hook';
CREATE OR REPLACE FUNCTION forge_lease_owns_run(l leases,r runs) RETURNS BOOLEAN LANGUAGE SQL IMMUTABLE AS $$
 SELECT l.id=r.lease_id AND l.project_id=r.project_id AND l.employee_id IS NOT DISTINCT FROM r.employee_id
  AND l.fencing_token=r.lease_fencing_token AND l.environment_epoch=r.environment_epoch
  AND l.purpose=r.purpose AND CASE r.purpose
   WHEN 'task_stage' THEN l.task_id=r.task_id AND l.queue_entry_id=r.queue_entry_id
   WHEN 'communication' THEN l.communication_assignment_id=r.communication_assignment_id
   WHEN 'resolution' THEN l.resolution_assignment_id=r.resolution_assignment_id
   WHEN 'hook' THEN l.hook_invocation_id=r.hook_invocation_id
   ELSE FALSE END;
$$;
CREATE OR REPLACE FUNCTION forge_guard_assignment_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (OLD.purpose,OLD.communication_assignment_id,OLD.resolution_assignment_id,OLD.hook_invocation_id) IS DISTINCT FROM
    (NEW.purpose,NEW.communication_assignment_id,NEW.resolution_assignment_id,NEW.hook_invocation_id) THEN
  RAISE EXCEPTION 'execution assignment is immutable';
 END IF;
 RETURN NEW;
END; $$;

ALTER TABLE task_handoffs ADD COLUMN hook_invocation_id UUID UNIQUE REFERENCES hook_invocations(id);
DO $$ DECLARE constraint_name text; BEGIN
 FOR constraint_name IN SELECT conname FROM pg_constraint
  WHERE conrelid='task_handoffs'::regclass AND contype='c'
   AND pg_get_constraintdef(oid) LIKE '%run_id IS NOT NULL%'
   AND pg_get_constraintdef(oid) LIKE '%integration_id IS NOT NULL%'
 LOOP EXECUTE format('ALTER TABLE task_handoffs DROP CONSTRAINT %I',constraint_name); END LOOP;
END $$;
ALTER TABLE task_handoffs ADD CONSTRAINT handoff_producer_exact CHECK(
 (run_id IS NOT NULL)::integer+(integration_id IS NOT NULL)::integer+(hook_invocation_id IS NOT NULL)::integer=1);
ALTER TABLE task_handoffs ADD CHECK((hook_invocation_id IS NULL OR
 (body->'producer'->>'kind'='system_action' AND body->'producer'->>'operation_id'=hook_invocation_id::text)) IS TRUE);
