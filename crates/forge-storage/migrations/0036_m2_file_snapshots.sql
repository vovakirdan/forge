-- Captures reserve a stopped surface until its immutable files have been sealed.
CREATE TABLE task_file_snapshots (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    task_id uuid NOT NULL,
    artifact_id uuid NOT NULL UNIQUE,
    state text NOT NULL CHECK (state IN ('pending','sealed','failed')),
    capture_surface boolean NOT NULL,
    canonical_snapshot jsonb NOT NULL,
    created_at timestamptz NOT NULL,
    UNIQUE(project_id,artifact_id),
    FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id),
    CHECK ((canonical_snapshot->>'id'=id::text
        AND canonical_snapshot->>'project_id'=project_id::text
        AND canonical_snapshot->>'task_id'=task_id::text
        AND canonical_snapshot->>'artifact_id'=artifact_id::text
        AND canonical_snapshot->>'state'=state
        AND (canonical_snapshot->'source'->>'kind'='task_surface')=capture_surface) IS TRUE),
    CHECK ((state='sealed' AND jsonb_typeof(canonical_snapshot->'manifest')='object' AND canonical_snapshot->'error_code'='null'::jsonb)
        OR (state='pending' AND canonical_snapshot->'manifest'='null'::jsonb AND canonical_snapshot->'error_code'='null'::jsonb)
        OR (state='failed' AND canonical_snapshot->'manifest'='null'::jsonb AND jsonb_typeof(canonical_snapshot->'error_code')='string'))
);
CREATE UNIQUE INDEX task_file_snapshots_pending_capture
    ON task_file_snapshots(task_id) WHERE state='pending' AND capture_surface;
CREATE INDEX task_file_snapshots_pending ON task_file_snapshots(id) WHERE state='pending';

CREATE TABLE task_file_inputs (
    project_id uuid NOT NULL REFERENCES projects(id),
    task_id uuid NOT NULL,
    artifact_id uuid NOT NULL REFERENCES artifacts(id),
    canonical_snapshot jsonb NOT NULL,
    PRIMARY KEY(task_id,artifact_id),
    FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id),
    FOREIGN KEY(project_id,artifact_id) REFERENCES task_file_snapshots(project_id,artifact_id),
    CHECK ((canonical_snapshot->>'artifact_id'=artifact_id::text
        AND canonical_snapshot->'manifest'->>'project_id'=project_id::text
        AND canonical_snapshot->'manifest'->>'source_task_id'<>task_id::text) IS TRUE)
);

CREATE FUNCTION forge_guard_file_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'file snapshot history is retained'; END IF;
    IF OLD.state<>'pending' OR NEW.state NOT IN ('sealed','failed')
        OR (OLD.id,OLD.project_id,OLD.task_id,OLD.artifact_id,OLD.capture_surface,OLD.created_at,
            OLD.canonical_snapshot - ARRAY['state','manifest','error_code'])
        IS DISTINCT FROM
           (NEW.id,NEW.project_id,NEW.task_id,NEW.artifact_id,NEW.capture_surface,NEW.created_at,
            NEW.canonical_snapshot - ARRAY['state','manifest','error_code']) THEN
        RAISE EXCEPTION 'file snapshot identity and completed result are immutable';
    END IF;
    RETURN NEW;
END; $$;
CREATE TRIGGER file_snapshot_guard BEFORE UPDATE OR DELETE ON task_file_snapshots
    FOR EACH ROW EXECUTE FUNCTION forge_guard_file_snapshot();
CREATE TRIGGER file_input_immutable BEFORE UPDATE OR DELETE ON task_file_inputs
    FOR EACH ROW EXECUTE FUNCTION forge_reject_repository_mutation();
