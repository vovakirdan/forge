-- Canonical messages/outbox own initial intent. These immutable deliveries pin
-- that source to one already-authorized execution, never a replacement Run.
ALTER TABLE runs ADD CONSTRAINT runs_runtime_input_scope_unique
    UNIQUE(project_id,id,employee_id,lease_fencing_token,environment_epoch);
CREATE TABLE runtime_input_deliveries (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    employee_id UUID NOT NULL,
    run_id UUID NOT NULL,
    fencing_token BIGINT NOT NULL CHECK(fencing_token>0),
    environment_epoch BIGINT NOT NULL CHECK(environment_epoch>0),
    sequence BIGINT GENERATED ALWAYS AS IDENTITY UNIQUE,
    source_message_id UUID,
    kind TEXT NOT NULL CHECK(kind IN ('message','close_after_turn')),
    canonical_snapshot JSONB NOT NULL CHECK(jsonb_typeof(canonical_snapshot)='object'),
    created_at TIMESTAMPTZ NOT NULL,
    CHECK((kind='message')=(source_message_id IS NOT NULL)),
    UNIQUE(run_id,source_message_id),
    FOREIGN KEY(project_id,run_id,employee_id,fencing_token,environment_epoch)
        REFERENCES runs(project_id,id,employee_id,lease_fencing_token,environment_epoch),
    FOREIGN KEY(project_id,source_message_id) REFERENCES employee_messages(project_id,id)
);
CREATE UNIQUE INDEX runtime_input_closed_once ON runtime_input_deliveries(run_id) WHERE kind='close_after_turn';
CREATE INDEX runtime_inputs_run ON runtime_input_deliveries(run_id,sequence);

CREATE TABLE runtime_input_observations (
    input_id UUID NOT NULL REFERENCES runtime_input_deliveries(id),
    receipt_id UUID PRIMARY KEY,
    outcome TEXT NOT NULL CHECK(outcome IN ('runtime_accepted','input_closed','unsupported','stale_scope','conflict','delivery_unknown')),
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX runtime_input_observations_input ON runtime_input_observations(input_id);
CREATE TRIGGER runtime_input_delivery_immutable BEFORE UPDATE OR DELETE ON runtime_input_deliveries
    FOR EACH ROW EXECUTE FUNCTION forge_reject_append_only_write();
CREATE TRIGGER runtime_input_observation_immutable BEFORE UPDATE OR DELETE ON runtime_input_observations
    FOR EACH ROW EXECUTE FUNCTION forge_reject_append_only_write();
