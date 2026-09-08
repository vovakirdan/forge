-- A source-bound Communication queue shares the canonical Run/Lease ledger.
-- Its identity equals the immutable source message identity, not a Task identity.
ALTER TABLE employee_messages ADD CONSTRAINT messages_recipient_source_unique
    UNIQUE (project_id, employee_id, thread_id, id);

CREATE TABLE communication_assignments (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    employee_id UUID NOT NULL,
    thread_id UUID NOT NULL,
    source_message_id UUID NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued' CHECK (state IN ('queued','leased','completed','held')),
    attempt_number INTEGER NOT NULL DEFAULT 0 CHECK (attempt_number >= 0),
    retry_authorized BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    completed_at TIMESTAMPTZ,
    UNIQUE (project_id, id),
    UNIQUE (project_id, employee_id, id),
    UNIQUE (source_message_id),
    FOREIGN KEY (project_id, employee_id, thread_id, source_message_id)
        REFERENCES employee_messages(project_id, employee_id, thread_id, id),
    CHECK ((state = 'completed') = (completed_at IS NOT NULL))
);
CREATE UNIQUE INDEX communication_thread_active ON communication_assignments(thread_id) WHERE state = 'leased';
CREATE INDEX communication_ready ON communication_assignments(project_id, created_at, id) WHERE state = 'queued';

CREATE FUNCTION forge_guard_communication_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF (OLD.id,OLD.project_id,OLD.employee_id,OLD.thread_id,OLD.source_message_id,OLD.created_at)
        IS DISTINCT FROM (NEW.id,NEW.project_id,NEW.employee_id,NEW.thread_id,NEW.source_message_id,NEW.created_at) THEN
        RAISE EXCEPTION 'communication source identity is immutable';
    END IF;
    IF NEW.attempt_number <> OLD.attempt_number AND NOT
        (OLD.state='queued' AND NEW.state='leased' AND NEW.attempt_number=OLD.attempt_number+1) THEN
        RAISE EXCEPTION 'communication attempt must advance only at admission';
    END IF;
    IF NEW.state <> OLD.state AND NOT (
        (OLD.state='queued' AND NEW.state='leased') OR
        (OLD.state='leased' AND NEW.state IN ('held','completed')) OR
        (OLD.state='held' AND NEW.state='queued')
    ) THEN RAISE EXCEPTION 'invalid communication lifecycle transition'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER communication_identity_immutable BEFORE UPDATE ON communication_assignments
    FOR EACH ROW EXECUTE FUNCTION forge_guard_communication_identity();

CREATE FUNCTION forge_enqueue_communication() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.canonical_snapshot->'target'->>'kind' = 'inbox' AND NOT (
        NEW.canonical_snapshot->'sender'->>'kind' = 'employee'
        AND NEW.canonical_snapshot->'sender'->>'id' = NEW.employee_id::text
    ) THEN
        INSERT INTO communication_assignments(id,project_id,employee_id,thread_id,source_message_id,created_at)
        VALUES (NEW.id,NEW.project_id,NEW.employee_id,NEW.thread_id,NEW.id,NEW.created_at);
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enqueue_communication AFTER INSERT ON employee_messages FOR EACH ROW EXECUTE FUNCTION forge_enqueue_communication();
INSERT INTO communication_assignments(id,project_id,employee_id,thread_id,source_message_id,created_at)
    SELECT id,project_id,employee_id,thread_id,id,created_at FROM employee_messages
    WHERE canonical_snapshot->'target'->>'kind' = 'inbox' AND NOT (
        canonical_snapshot->'sender'->>'kind' = 'employee'
        AND canonical_snapshot->'sender'->>'id' = employee_id::text
    );

ALTER TABLE leases ADD COLUMN purpose TEXT NOT NULL DEFAULT 'task_stage';
ALTER TABLE leases ADD COLUMN communication_assignment_id UUID;
ALTER TABLE leases ALTER COLUMN task_id DROP NOT NULL;
ALTER TABLE leases ALTER COLUMN queue_entry_id DROP NOT NULL;
ALTER TABLE leases ADD CONSTRAINT leases_owner_exact CHECK (
    (purpose='task_stage' AND task_id IS NOT NULL AND queue_entry_id IS NOT NULL AND communication_assignment_id IS NULL)
    OR (purpose='communication' AND task_id IS NULL AND queue_entry_id IS NULL AND task_work_surface_id IS NULL AND communication_assignment_id IS NOT NULL)
);
ALTER TABLE leases ADD CONSTRAINT leases_communication_owner_fk FOREIGN KEY(project_id,employee_id,communication_assignment_id)
    REFERENCES communication_assignments(project_id,employee_id,id);
ALTER TABLE leases ADD CONSTRAINT leases_common_owner_unique UNIQUE(project_id,id,purpose,employee_id,fencing_token);
ALTER TABLE leases ADD CONSTRAINT leases_task_owner_unique UNIQUE(project_id,id,task_id,queue_entry_id);
ALTER TABLE leases ADD CONSTRAINT leases_communication_scope_unique UNIQUE(project_id,id,communication_assignment_id);
CREATE UNIQUE INDEX leases_active_communication ON leases(communication_assignment_id) WHERE lease_state='active' AND purpose='communication';

ALTER TABLE runs ADD COLUMN purpose TEXT NOT NULL DEFAULT 'task_stage';
ALTER TABLE runs ADD COLUMN communication_assignment_id UUID;
ALTER TABLE runs ALTER COLUMN task_id DROP NOT NULL;
ALTER TABLE runs ALTER COLUMN queue_entry_id DROP NOT NULL;
ALTER TABLE runs ALTER COLUMN stage_id DROP NOT NULL;
ALTER TABLE runs ADD CONSTRAINT runs_owner_exact CHECK (
    (purpose='task_stage' AND task_id IS NOT NULL AND queue_entry_id IS NOT NULL AND stage_id IS NOT NULL AND communication_assignment_id IS NULL AND run_spec_version IN (1,2))
    OR (purpose='communication' AND task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND communication_assignment_id IS NOT NULL AND run_spec_version=3)
);
ALTER TABLE runs ADD CONSTRAINT runs_common_lease_owner_fk FOREIGN KEY(project_id,lease_id,purpose,employee_id,lease_fencing_token)
    REFERENCES leases(project_id,id,purpose,employee_id,fencing_token);
ALTER TABLE runs ADD CONSTRAINT runs_task_lease_owner_fk FOREIGN KEY(project_id,lease_id,task_id,queue_entry_id)
    REFERENCES leases(project_id,id,task_id,queue_entry_id);
ALTER TABLE runs ADD CONSTRAINT runs_communication_lease_owner_fk FOREIGN KEY(project_id,lease_id,communication_assignment_id)
    REFERENCES leases(project_id,id,communication_assignment_id);
ALTER TABLE runs ADD CONSTRAINT runs_communication_attempt_unique UNIQUE(communication_assignment_id,attempt_number);
ALTER TABLE runs ADD CONSTRAINT runs_reservation_scope_unique UNIQUE(project_id,id,purpose,employee_id,lease_fencing_token);
ALTER TABLE runs ADD CONSTRAINT runs_communication_owner_unique UNIQUE(project_id,id,communication_assignment_id);

ALTER TABLE run_environment_reservations ADD COLUMN purpose TEXT NOT NULL DEFAULT 'task_stage';
ALTER TABLE run_environment_reservations ADD COLUMN communication_assignment_id UUID;
ALTER TABLE run_environment_reservations ALTER COLUMN task_id DROP NOT NULL;
ALTER TABLE run_environment_reservations ADD CONSTRAINT environment_owner_exact CHECK (
    (purpose='task_stage' AND task_id IS NOT NULL AND communication_assignment_id IS NULL)
    OR (purpose='communication' AND task_id IS NULL AND surface_id IS NULL AND communication_assignment_id IS NOT NULL)
);
ALTER TABLE run_environment_reservations ADD CONSTRAINT environment_common_owner_fk
    FOREIGN KEY(project_id,run_id,purpose,employee_id,fencing_token)
    REFERENCES runs(project_id,id,purpose,employee_id,lease_fencing_token);
ALTER TABLE run_environment_reservations ADD CONSTRAINT environment_communication_owner_fk
    FOREIGN KEY(project_id,run_id,communication_assignment_id)
    REFERENCES runs(project_id,id,communication_assignment_id);
CREATE UNIQUE INDEX one_physical_communication_assignment ON run_environment_reservations(communication_assignment_id)
    WHERE released_at IS NULL AND purpose='communication';

-- No NULL equality shortcut may accidentally authorize an unrelated owner.
CREATE FUNCTION forge_lease_owns_run(l leases,r runs) RETURNS BOOLEAN LANGUAGE SQL IMMUTABLE AS $$
    SELECT l.id=r.lease_id AND l.project_id=r.project_id AND l.employee_id=r.employee_id
      AND l.fencing_token=r.lease_fencing_token AND l.environment_epoch=r.environment_epoch
      AND l.purpose=r.purpose AND CASE r.purpose
        WHEN 'task_stage' THEN l.task_id=r.task_id AND l.queue_entry_id=r.queue_entry_id
        WHEN 'communication' THEN l.communication_assignment_id=r.communication_assignment_id
        ELSE FALSE END;
$$;

CREATE FUNCTION forge_guard_assignment_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF (OLD.purpose,OLD.communication_assignment_id) IS DISTINCT FROM (NEW.purpose,NEW.communication_assignment_id) THEN
        RAISE EXCEPTION 'execution assignment is immutable';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER leases_assignment_immutable BEFORE UPDATE ON leases FOR EACH ROW EXECUTE FUNCTION forge_guard_assignment_identity();
CREATE TRIGGER runs_assignment_immutable BEFORE UPDATE ON runs FOR EACH ROW EXECUTE FUNCTION forge_guard_assignment_identity();
CREATE TRIGGER environment_assignment_immutable BEFORE UPDATE ON run_environment_reservations FOR EACH ROW EXECUTE FUNCTION forge_guard_assignment_identity();
