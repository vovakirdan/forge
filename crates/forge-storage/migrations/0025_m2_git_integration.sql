CREATE SEQUENCE git_integration_fence_seq AS BIGINT MINVALUE 1;
CREATE TABLE git_integrations (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    task_id UUID NOT NULL,
    candidate_proposal_id UUID NOT NULL,
    fence BIGINT NOT NULL UNIQUE CHECK(fence>0),
    canonical_snapshot JSONB NOT NULL,
    intent JSONB,
    state TEXT NOT NULL CHECK(state IN ('preparing','prepared','applying','held','completed','retired')),
    core_instance UUID NOT NULL,
    expected_task_revision BIGINT NOT NULL CHECK(expected_task_revision>0),
    command JSONB,
    result JSONB,
    wait_condition_id UUID,
    resolution TEXT CHECK(resolution IN ('retry','accept')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(project_id,id),
    FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id),
    FOREIGN KEY(project_id,candidate_proposal_id) REFERENCES git_stage_proposals(project_id,id),
    CHECK(canonical_snapshot->>'id'=id::text),
    CHECK(canonical_snapshot->>'project_id'=project_id::text),
    CHECK(canonical_snapshot->>'task_id'=task_id::text),
    CHECK(canonical_snapshot->>'candidate_proposal_id'=candidate_proposal_id::text),
    CHECK((canonical_snapshot->>'fence')::bigint=fence)
);
CREATE UNIQUE INDEX git_integration_unresolved_task ON git_integrations(task_id) WHERE state NOT IN ('completed','retired');
CREATE TABLE git_integration_receipts (
    command_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES git_integrations(id),
    request JSONB NOT NULL,
    result JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE FUNCTION forge_guard_git_integration() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'integration history is retained'; END IF;
    IF (OLD.id,OLD.project_id,OLD.task_id,OLD.candidate_proposal_id,OLD.fence,OLD.canonical_snapshot)
       IS DISTINCT FROM (NEW.id,NEW.project_id,NEW.task_id,NEW.candidate_proposal_id,NEW.fence,NEW.canonical_snapshot)
       OR (OLD.intent IS NOT NULL AND OLD.intent IS DISTINCT FROM NEW.intent) THEN
        RAISE EXCEPTION 'integration identity and prepared intent are immutable';
    END IF;
    IF OLD.state IN ('completed','retired') THEN RAISE EXCEPTION 'terminal integration is immutable'; END IF;
    RETURN NEW;
END; $$;
CREATE TRIGGER git_integration_guard BEFORE UPDATE OR DELETE ON git_integrations FOR EACH ROW EXECUTE FUNCTION forge_guard_git_integration();
CREATE FUNCTION forge_guard_git_integration_receipt() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' OR (OLD.command_id,OLD.operation_id,OLD.request,OLD.created_at) IS DISTINCT FROM (NEW.command_id,NEW.operation_id,NEW.request,NEW.created_at)
        OR OLD.result IS NOT NULL THEN RAISE EXCEPTION 'integration receipt is immutable'; END IF;
    RETURN NEW;
END; $$;
CREATE TRIGGER git_integration_receipt_guard BEFORE UPDATE OR DELETE ON git_integration_receipts FOR EACH ROW EXECUTE FUNCTION forge_guard_git_integration_receipt();

-- A deterministic System stage produces a handoff without inventing a Run.
ALTER TABLE task_handoffs ADD COLUMN id UUID;
UPDATE task_handoffs SET id=(body->>'id')::uuid;
ALTER TABLE task_handoffs ALTER COLUMN id SET NOT NULL;
ALTER TABLE task_handoffs DROP CONSTRAINT task_handoffs_pkey;
ALTER TABLE task_handoffs ALTER COLUMN run_id DROP NOT NULL;
ALTER TABLE task_handoffs ADD PRIMARY KEY(id);
ALTER TABLE task_handoffs ADD UNIQUE(run_id);
ALTER TABLE task_handoffs ADD COLUMN integration_id UUID UNIQUE REFERENCES git_integrations(id);
ALTER TABLE task_handoffs ADD CHECK((run_id IS NOT NULL)::integer+(integration_id IS NOT NULL)::integer=1);
ALTER TABLE task_handoffs ADD CHECK(body->>'id'=id::text);
ALTER TABLE task_handoffs ADD CHECK(integration_id IS NULL OR
    (body->'producer'->>'kind'='system_action' AND body->'producer'->>'operation_id'=integration_id::text));
