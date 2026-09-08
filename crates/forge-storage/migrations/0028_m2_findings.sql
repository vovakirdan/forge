-- Reporting an observation never creates or queues a Task.
CREATE TABLE findings (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id),
    source_task_id UUID NOT NULL,
    source_run_id UUID,
    revision BIGINT NOT NULL CHECK(revision>0),
    state TEXT NOT NULL CHECK(state IN ('open','attached','promoted','ignored')),
    target_task_id UUID,
    canonical_snapshot JSONB NOT NULL CHECK(jsonb_typeof(canonical_snapshot)='object'),
    FOREIGN KEY(project_id,source_task_id) REFERENCES tasks(project_id,id),
    FOREIGN KEY(project_id,source_run_id) REFERENCES runs(project_id,id),
    FOREIGN KEY(project_id,target_task_id) REFERENCES tasks(project_id,id),
    CHECK(canonical_snapshot->>'id'=id::text),
    CHECK(canonical_snapshot->>'project_id'=project_id::text),
    CHECK(canonical_snapshot->>'source_task_id'=source_task_id::text),
    CHECK(source_run_id IS NOT DISTINCT FROM (canonical_snapshot->'source_run'->>'run_id')::uuid),
    CHECK((canonical_snapshot->>'revision')::bigint=revision),
    CHECK(canonical_snapshot->'state'->>'status'=state),
    CHECK(target_task_id IS NOT DISTINCT FROM (canonical_snapshot->'state'->>'task_id')::uuid),
    CHECK((state IN ('attached','promoted'))=(target_task_id IS NOT NULL))
);
CREATE INDEX findings_project_page ON findings(project_id,id);
CREATE INDEX findings_source ON findings(project_id,source_task_id,id);
CREATE UNIQUE INDEX finding_promoted_task ON findings(target_task_id) WHERE state='promoted';
CREATE FUNCTION forge_guard_finding() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'finding history is retained'; END IF;
    IF OLD.canonical_snapshot-'state'-'revision' IS DISTINCT FROM NEW.canonical_snapshot-'state'-'revision'
        OR (OLD.id,OLD.project_id,OLD.source_task_id,OLD.source_run_id) IS DISTINCT FROM (NEW.id,NEW.project_id,NEW.source_task_id,NEW.source_run_id) THEN
        RAISE EXCEPTION 'finding report is immutable';
    END IF;
    IF OLD.state NOT IN ('open','attached') OR NEW.state='open' OR NEW.revision<>OLD.revision+1 THEN
        RAISE EXCEPTION 'finding triage transition is forbidden';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER findings_immutable_report BEFORE UPDATE OR DELETE ON findings
    FOR EACH ROW EXECUTE FUNCTION forge_guard_finding();
