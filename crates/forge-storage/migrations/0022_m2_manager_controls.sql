-- One-shot, visit-scoped admission intent. It never reassigns an existing Run.
CREATE TABLE task_next_run_constraints (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id),
    task_id UUID NOT NULL,
    employee_id UUID NOT NULL,
    pipeline_version_id UUID NOT NULL REFERENCES pipeline_versions(id),
    stage_id TEXT NOT NULL,
    stage_visit BIGINT NOT NULL CHECK (stage_visit > 0),
    constraint_state TEXT NOT NULL CHECK (constraint_state IN ('pending','blocked','consumed','cancelled')),
    consumed_run_id UUID,
    canonical_snapshot JSONB NOT NULL CHECK (jsonb_typeof(canonical_snapshot)='object'),
    created_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (project_id,task_id) REFERENCES tasks(project_id,id),
    FOREIGN KEY (project_id,employee_id) REFERENCES employees(project_id,id),
    FOREIGN KEY (project_id,consumed_run_id) REFERENCES runs(project_id,id),
    CHECK (canonical_snapshot->>'id'=id::text),
    CHECK (canonical_snapshot->>'project_id'=project_id::text),
    CHECK (canonical_snapshot->>'task_id'=task_id::text),
    CHECK (canonical_snapshot->>'employee_id'=employee_id::text),
    CHECK (canonical_snapshot->>'pipeline_version_id'=pipeline_version_id::text),
    CHECK (canonical_snapshot->>'stage_id'=stage_id),
    CHECK ((canonical_snapshot->>'stage_visit')::bigint=stage_visit),
    CHECK (canonical_snapshot->'state'->>'status'=constraint_state),
    CHECK ((constraint_state='consumed')=(consumed_run_id IS NOT NULL)),
    CHECK (consumed_run_id IS NOT DISTINCT FROM (canonical_snapshot->'state'->>'run_id')::uuid)
);
CREATE UNIQUE INDEX task_next_run_single_constraint ON task_next_run_constraints(task_id)
    WHERE constraint_state IN ('pending','blocked');

CREATE FUNCTION forge_guard_dispatch_constraint() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'dispatch constraint history is retained'; END IF;
    IF OLD.canonical_snapshot-'state' IS DISTINCT FROM NEW.canonical_snapshot-'state'
        OR OLD.id<>NEW.id OR OLD.project_id<>NEW.project_id OR OLD.task_id<>NEW.task_id
        OR OLD.employee_id<>NEW.employee_id OR OLD.pipeline_version_id<>NEW.pipeline_version_id
        OR OLD.stage_id<>NEW.stage_id OR OLD.stage_visit<>NEW.stage_visit OR OLD.created_at<>NEW.created_at THEN
        RAISE EXCEPTION 'dispatch constraint identity is immutable';
    END IF;
    IF NOT ((OLD.constraint_state='pending' AND NEW.constraint_state IN ('blocked','consumed','cancelled'))
        OR (OLD.constraint_state='blocked' AND NEW.constraint_state='cancelled')) THEN
        RAISE EXCEPTION 'dispatch constraint transition is forbidden';
    END IF;
    IF NEW.constraint_state='consumed' AND NOT EXISTS (
        SELECT 1 FROM runs r JOIN queue_entries q ON q.id=r.queue_entry_id
        WHERE r.id=NEW.consumed_run_id AND r.project_id=NEW.project_id
          AND r.task_id=NEW.task_id AND r.employee_id=NEW.employee_id
          AND q.pipeline_version_id=NEW.pipeline_version_id AND q.stage_id=NEW.stage_id
          AND (r.context_manifest->>'stage_visit')::bigint=NEW.stage_visit
    ) THEN RAISE EXCEPTION 'consuming Run does not own the constrained Task visit'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER task_next_run_constraint_guard BEFORE UPDATE OR DELETE ON task_next_run_constraints
    FOR EACH ROW EXECUTE FUNCTION forge_guard_dispatch_constraint();

-- Polling is a durable wake source; outbox delivery is only a latency optimization.
CREATE TABLE task_resume_schedules (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id),
    task_id UUID NOT NULL,
    wait_condition_id UUID NOT NULL,
    not_before TIMESTAMPTZ NOT NULL,
    schedule_state TEXT NOT NULL CHECK (schedule_state IN ('pending','applied','rejected','cancelled')),
    canonical_snapshot JSONB NOT NULL CHECK (jsonb_typeof(canonical_snapshot)='object'),
    FOREIGN KEY (project_id,task_id) REFERENCES tasks(project_id,id),
    CHECK (canonical_snapshot->>'id'=id::text),
    CHECK (canonical_snapshot->>'project_id'=project_id::text),
    CHECK (canonical_snapshot->>'task_id'=task_id::text),
    CHECK (canonical_snapshot->>'wait_condition_id'=wait_condition_id::text),
    CHECK (canonical_snapshot->'state'->>'status'=schedule_state)
);
CREATE UNIQUE INDEX task_resume_one_pending_wait ON task_resume_schedules(project_id,task_id,wait_condition_id)
    WHERE schedule_state='pending';
CREATE INDEX task_resume_due ON task_resume_schedules(not_before,id) WHERE schedule_state='pending';
CREATE FUNCTION forge_guard_resume_schedule() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'resume schedule history is retained'; END IF;
    IF OLD.canonical_snapshot-'state' IS DISTINCT FROM NEW.canonical_snapshot-'state'
        OR OLD.id<>NEW.id OR OLD.project_id<>NEW.project_id OR OLD.task_id<>NEW.task_id
        OR OLD.wait_condition_id<>NEW.wait_condition_id OR OLD.not_before<>NEW.not_before THEN
        RAISE EXCEPTION 'resume schedule identity is immutable';
    END IF;
    IF OLD.schedule_state<>'pending' OR NEW.schedule_state='pending' THEN
        RAISE EXCEPTION 'resume schedule has one terminal result';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER task_resume_schedule_guard BEFORE UPDATE OR DELETE ON task_resume_schedules
    FOR EACH ROW EXECUTE FUNCTION forge_guard_resume_schedule();
