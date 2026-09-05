-- Maps an ExecutorSubmission ArtifactSubmission transport receipt to the
-- canonical Artifact that Core accepted. The mapping is append-only so a
-- duplicate or later StageOutcomeSubmission can resolve the original evidence
-- without trusting an in-memory Core cache.

CREATE TABLE executor_artifact_receipts (
    message_id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES runs (id) ON DELETE RESTRICT,
    lease_fencing_token BIGINT NOT NULL,
    environment_epoch BIGINT NOT NULL,
    artifact_id UUID NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT executor_artifact_receipts_fencing_token_positive
        CHECK (lease_fencing_token > 0),
    CONSTRAINT executor_artifact_receipts_environment_epoch_positive
        CHECK (environment_epoch > 0),
    CONSTRAINT executor_artifact_receipts_artifact_fk
        FOREIGN KEY (artifact_id)
        REFERENCES artifacts (id)
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX executor_artifact_receipts_scope_message_idx
    ON executor_artifact_receipts (
        run_id,
        lease_fencing_token,
        environment_epoch,
        message_id
    );

CREATE FUNCTION forge_guard_executor_artifact_receipt_fence()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    current_fencing_token BIGINT;
    current_environment_epoch BIGINT;
BEGIN
    SELECT r.lease_fencing_token, r.environment_epoch
    INTO current_fencing_token, current_environment_epoch
    FROM runs AS r
    JOIN leases AS l
        ON l.id = r.lease_id
        AND l.project_id = r.project_id
        AND l.task_id = r.task_id
        AND l.employee_id = r.employee_id
        AND l.fencing_token = r.lease_fencing_token
        AND l.environment_epoch = r.environment_epoch
        AND l.lease_state = 'active'
    WHERE r.id = NEW.run_id
    FOR UPDATE OF r, l;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'executor artifact receipt run has no matching active lease'
            USING ERRCODE = '23503';
    END IF;

    IF NEW.lease_fencing_token <> current_fencing_token
        OR NEW.environment_epoch <> current_environment_epoch THEN
        RAISE EXCEPTION 'executor artifact receipt fence does not match the active run epoch'
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER executor_artifact_receipts_fence_matches_run
BEFORE INSERT ON executor_artifact_receipts
FOR EACH ROW
EXECUTE FUNCTION forge_guard_executor_artifact_receipt_fence();

CREATE TRIGGER executor_artifact_receipts_append_only
BEFORE UPDATE OR DELETE ON executor_artifact_receipts
FOR EACH ROW
EXECUTE FUNCTION forge_reject_append_only_write();
