-- Routes are catalogs; each escalation freezes its selected configuration.
CREATE TABLE resolver_routes (
    project_id UUID NOT NULL REFERENCES projects(id),
    route_key TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    canonical_snapshot JSONB NOT NULL,
    PRIMARY KEY(project_id, route_key)
);

CREATE TABLE escalations (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id),
    task_id UUID,
    wait_condition_id UUID,
    revision BIGINT NOT NULL CHECK (revision > 0),
    generation BIGINT NOT NULL CHECK (generation >= 0),
    escalation_state TEXT NOT NULL CHECK (escalation_state IN ('queued','assigned','resolved','needs_management_change','superseded')),
    canonical_snapshot JSONB NOT NULL,
    source_run_id UUID GENERATED ALWAYS AS ((canonical_snapshot->'source'->>'run_id')::uuid) STORED,
    UNIQUE(project_id,id),
    UNIQUE(project_id,task_id,wait_condition_id),
    FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id),
    FOREIGN KEY(project_id,source_run_id) REFERENCES runs(project_id,id),
    UNIQUE(source_run_id)
);

CREATE TABLE resolution_assignments (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    escalation_id UUID NOT NULL,
    generation BIGINT NOT NULL CHECK (generation > 0),
    employee_id UUID,
    expires_at TIMESTAMPTZ,
    assignment_state TEXT NOT NULL CHECK (assignment_state IN ('active','answered','retired')),
    canonical_snapshot JSONB NOT NULL,
    UNIQUE(project_id,id),
    UNIQUE(escalation_id,generation),
    FOREIGN KEY(project_id,escalation_id) REFERENCES escalations(project_id,id),
    FOREIGN KEY(project_id,employee_id) REFERENCES employees(project_id,id),
    CHECK ((employee_id IS NULL) = (expires_at IS NULL))
);
CREATE UNIQUE INDEX resolution_one_active_assignment ON resolution_assignments(escalation_id) WHERE assignment_state='active';
CREATE INDEX resolution_expiry_queue ON resolution_assignments(expires_at,id) WHERE assignment_state='active' AND employee_id IS NOT NULL;
CREATE INDEX escalation_pending_queue ON escalations(project_id,id) WHERE escalation_state='queued';

ALTER TABLE resolver_routes ADD CONSTRAINT resolver_route_snapshot_scope CHECK ((
  canonical_snapshot->>'project_id'=project_id::text AND canonical_snapshot->>'key'=route_key
  AND (canonical_snapshot->>'revision')::bigint=revision) IS TRUE);
ALTER TABLE escalations ADD CONSTRAINT escalation_snapshot_scope CHECK ((
  canonical_snapshot->>'id'=id::text AND canonical_snapshot->>'project_id'=project_id::text
  AND ((canonical_snapshot->'source'->>'kind'='task' AND source_run_id IS NULL
        AND canonical_snapshot->'source'->>'task_id'=task_id::text
        AND canonical_snapshot->'source'->>'wait_condition_id'=wait_condition_id::text)
       OR (canonical_snapshot->'source'->>'kind'='communication' AND source_run_id IS NOT NULL
           AND task_id IS NULL AND wait_condition_id IS NULL))
  AND (canonical_snapshot->>'revision')::bigint=revision
  AND (canonical_snapshot->>'generation')::bigint=generation
  AND canonical_snapshot->'state'->>'status'=escalation_state) IS TRUE);
ALTER TABLE resolution_assignments ADD CONSTRAINT resolution_snapshot_scope CHECK ((
  canonical_snapshot->>'id'=id::text AND canonical_snapshot->>'project_id'=project_id::text
  AND canonical_snapshot->>'escalation_id'=escalation_id::text
  AND (canonical_snapshot->'lease'->>'generation')::bigint=generation
  AND canonical_snapshot->'state'->>'status'=assignment_state
  AND ((canonical_snapshot->'resolver'->>'kind'='human' AND employee_id IS NULL)
    OR (canonical_snapshot->'resolver'->>'kind'='employee' AND canonical_snapshot->'resolver'->>'employee_id'=employee_id::text))) IS TRUE);

CREATE FUNCTION forge_resolution_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP='DELETE' THEN RAISE EXCEPTION 'resolution history cannot be deleted'; END IF;
  IF TG_TABLE_NAME='resolver_routes' THEN
    IF NEW.project_id<>OLD.project_id OR NEW.route_key<>OLD.route_key OR NEW.revision<>OLD.revision+1 THEN
      RAISE EXCEPTION 'resolver route revision mismatch';
    END IF;
  ELSIF TG_TABLE_NAME='escalations' THEN
    IF (NEW.canonical_snapshot - ARRAY['revision','generation','next_candidate','state']) IS DISTINCT FROM
       (OLD.canonical_snapshot - ARRAY['revision','generation','next_candidate','state'])
       OR NEW.id<>OLD.id OR NEW.project_id<>OLD.project_id OR NEW.task_id<>OLD.task_id OR NEW.wait_condition_id<>OLD.wait_condition_id
       OR NEW.revision<>OLD.revision+1 OR NEW.generation<OLD.generation OR NEW.generation>OLD.generation+1
       OR OLD.escalation_state IN ('resolved','superseded') THEN RAISE EXCEPTION 'immutable escalation scope or invalid revision'; END IF;
  ELSE
    IF (NEW.canonical_snapshot - 'state') IS DISTINCT FROM (OLD.canonical_snapshot - 'state')
       OR NEW.id<>OLD.id OR NEW.project_id<>OLD.project_id OR NEW.escalation_id<>OLD.escalation_id
       OR NEW.generation<>OLD.generation OR NEW.employee_id IS DISTINCT FROM OLD.employee_id
       OR NEW.expires_at IS DISTINCT FROM OLD.expires_at OR OLD.assignment_state<>'active' OR NEW.assignment_state='active'
       THEN RAISE EXCEPTION 'immutable resolution assignment or terminal state'; END IF;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER resolver_route_guard BEFORE UPDATE OR DELETE ON resolver_routes FOR EACH ROW EXECUTE FUNCTION forge_resolution_guard();
CREATE TRIGGER escalation_guard BEFORE UPDATE OR DELETE ON escalations FOR EACH ROW EXECUTE FUNCTION forge_resolution_guard();
CREATE TRIGGER resolution_assignment_guard BEFORE UPDATE OR DELETE ON resolution_assignments FOR EACH ROW EXECUTE FUNCTION forge_resolution_guard();

-- Resolution executions have a real Employee but never a Task writer identity.
ALTER TABLE resolution_assignments ADD CONSTRAINT resolution_employee_scope UNIQUE(project_id,employee_id,id);
ALTER TABLE leases ADD COLUMN resolution_assignment_id UUID;
ALTER TABLE runs ADD COLUMN resolution_assignment_id UUID;
ALTER TABLE run_environment_reservations ADD COLUMN resolution_assignment_id UUID;
ALTER TABLE leases DROP CONSTRAINT leases_owner_exact;
ALTER TABLE leases ADD CONSTRAINT leases_owner_exact CHECK (
 (purpose='task_stage' AND task_id IS NOT NULL AND queue_entry_id IS NOT NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL)
 OR (purpose='communication' AND task_id IS NULL AND queue_entry_id IS NULL AND task_work_surface_id IS NULL AND communication_assignment_id IS NOT NULL AND resolution_assignment_id IS NULL)
 OR (purpose='resolution' AND task_id IS NULL AND queue_entry_id IS NULL AND task_work_surface_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NOT NULL)
);
ALTER TABLE leases ADD CONSTRAINT leases_resolution_owner_fk FOREIGN KEY(project_id,employee_id,resolution_assignment_id) REFERENCES resolution_assignments(project_id,employee_id,id);
ALTER TABLE leases ADD CONSTRAINT leases_resolution_scope_unique UNIQUE(project_id,id,resolution_assignment_id);
CREATE UNIQUE INDEX leases_active_resolution ON leases(resolution_assignment_id) WHERE lease_state='active' AND purpose='resolution';
ALTER TABLE runs DROP CONSTRAINT runs_owner_exact;
ALTER TABLE runs ADD CONSTRAINT runs_owner_exact CHECK (
 (purpose='task_stage' AND task_id IS NOT NULL AND queue_entry_id IS NOT NULL AND stage_id IS NOT NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND run_spec_version IN (1,2))
 OR (purpose='communication' AND task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND communication_assignment_id IS NOT NULL AND resolution_assignment_id IS NULL AND run_spec_version=3)
 OR (purpose='resolution' AND task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NOT NULL AND run_spec_version=4)
);
ALTER TABLE runs ADD CONSTRAINT runs_resolution_lease_fk FOREIGN KEY(project_id,lease_id,resolution_assignment_id) REFERENCES leases(project_id,id,resolution_assignment_id);
ALTER TABLE runs ADD CONSTRAINT runs_resolution_owner_unique UNIQUE(project_id,id,resolution_assignment_id);
CREATE UNIQUE INDEX resolution_assignment_run ON runs(resolution_assignment_id) WHERE purpose='resolution';
ALTER TABLE run_environment_reservations DROP CONSTRAINT environment_owner_exact;
ALTER TABLE run_environment_reservations ADD CONSTRAINT environment_owner_exact CHECK (
 (purpose='task_stage' AND task_id IS NOT NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL)
 OR (purpose='communication' AND task_id IS NULL AND surface_id IS NULL AND communication_assignment_id IS NOT NULL AND resolution_assignment_id IS NULL)
 OR (purpose='resolution' AND task_id IS NULL AND surface_id IS NULL AND communication_assignment_id IS NULL AND resolution_assignment_id IS NOT NULL)
);
ALTER TABLE run_environment_reservations ADD CONSTRAINT environment_resolution_owner_fk FOREIGN KEY(project_id,run_id,resolution_assignment_id) REFERENCES runs(project_id,id,resolution_assignment_id);
CREATE UNIQUE INDEX one_physical_resolution_assignment ON run_environment_reservations(resolution_assignment_id) WHERE released_at IS NULL AND purpose='resolution';

CREATE OR REPLACE FUNCTION forge_lease_owns_run(l leases,r runs) RETURNS BOOLEAN LANGUAGE SQL IMMUTABLE AS $$
 SELECT l.id=r.lease_id AND l.project_id=r.project_id AND l.employee_id=r.employee_id
  AND l.fencing_token=r.lease_fencing_token AND l.environment_epoch=r.environment_epoch
  AND l.purpose=r.purpose AND CASE r.purpose
   WHEN 'task_stage' THEN l.task_id=r.task_id AND l.queue_entry_id=r.queue_entry_id
   WHEN 'communication' THEN l.communication_assignment_id=r.communication_assignment_id
   WHEN 'resolution' THEN l.resolution_assignment_id=r.resolution_assignment_id
   ELSE FALSE END;
$$;
CREATE OR REPLACE FUNCTION forge_guard_assignment_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (OLD.purpose,OLD.communication_assignment_id,OLD.resolution_assignment_id) IS DISTINCT FROM
    (NEW.purpose,NEW.communication_assignment_id,NEW.resolution_assignment_id) THEN
  RAISE EXCEPTION 'execution assignment is immutable';
 END IF;
 RETURN NEW;
END;
$$;
