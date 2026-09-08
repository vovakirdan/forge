-- Human/external stage decisions are durable handoffs, not synthetic Runs.
ALTER TABLE idempotency_keys ADD UNIQUE(project_id,command_id);
ALTER TABLE task_handoffs ADD COLUMN command_id UUID UNIQUE;
ALTER TABLE task_handoffs ADD CONSTRAINT handoff_command_scope_fk
    FOREIGN KEY(project_id,command_id) REFERENCES idempotency_keys(project_id,command_id);
ALTER TABLE task_handoffs DROP CONSTRAINT handoff_producer_exact;
ALTER TABLE task_handoffs ADD CONSTRAINT handoff_producer_exact CHECK(
    (run_id IS NOT NULL)::integer+(integration_id IS NOT NULL)::integer+
    (hook_invocation_id IS NOT NULL)::integer+(command_id IS NOT NULL)::integer=1);
ALTER TABLE task_handoffs ADD CONSTRAINT handoff_command_snapshot CHECK((command_id IS NULL OR
    (body->'producer'->>'kind'='command' AND body->'producer'->>'command_id'=command_id::text
     AND body->>'project_id'=project_id::text AND body->>'task_id'=task_id::text)) IS TRUE);
ALTER TABLE task_handoffs ADD CHECK(((body->'producer'->>'kind'='command')=(command_id IS NOT NULL)) IS TRUE);
CREATE FUNCTION forge_guard_command_handoff() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP<>'INSERT' AND OLD.command_id IS NOT NULL THEN
        RAISE EXCEPTION 'command handoff history is immutable';
    END IF;
    IF TG_OP='DELETE' THEN RETURN OLD; END IF;
    IF NEW.command_id IS NOT NULL AND NOT EXISTS(
        SELECT 1 FROM idempotency_keys k WHERE k.project_id=NEW.project_id
          AND k.command_id=NEW.command_id AND k.command_name='submit_external_stage_outcome'
          AND k.actor=NEW.body->'actor'
          AND k.receipt->'resource'->>'kind'='task'
          AND k.receipt->'resource'->>'id'=NEW.task_id::text
    ) THEN RAISE EXCEPTION 'command handoff must match its canonical receipt'; END IF;
    RETURN NEW;
END; $$;
CREATE TRIGGER command_handoff_guard BEFORE INSERT OR UPDATE OR DELETE ON task_handoffs
    FOR EACH ROW EXECUTE FUNCTION forge_guard_command_handoff();

CREATE FUNCTION forge_guard_handoff_command_receipt() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD IS DISTINCT FROM NEW AND EXISTS(
        SELECT 1 FROM task_handoffs h WHERE h.project_id=OLD.project_id AND h.command_id=OLD.command_id
    ) THEN RAISE EXCEPTION 'handoff command receipt is immutable'; END IF;
    RETURN NEW;
END; $$;
CREATE TRIGGER handoff_command_receipt_guard BEFORE UPDATE ON idempotency_keys
    FOR EACH ROW EXECUTE FUNCTION forge_guard_handoff_command_receipt();
