ALTER TABLE runs ADD COLUMN liveness_observed_at TIMESTAMPTZ;
ALTER TABLE runs ADD COLUMN stop_requested_at TIMESTAMPTZ;

CREATE FUNCTION forge_run_stop_clock() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.desired_state IN ('stop_requested','force_stop_requested')
       AND OLD.desired_state NOT IN ('stop_requested','force_stop_requested') THEN
        NEW.stop_requested_at := clock_timestamp();
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER runs_stop_clock BEFORE UPDATE ON runs
    FOR EACH ROW EXECUTE FUNCTION forge_run_stop_clock();

CREATE TABLE local_execution_host (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
    host_id TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    reconciled_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE FUNCTION forge_reservation_host() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    SELECT host_id,boot_id INTO NEW.host_id,NEW.boot_id FROM local_execution_host WHERE singleton;
    RETURN NEW;
END;
$$;
CREATE TRIGGER reservation_host BEFORE INSERT ON run_environment_reservations
    FOR EACH ROW EXECUTE FUNCTION forge_reservation_host();

CREATE TABLE project_recovery_settings (
    project_id UUID PRIMARY KEY REFERENCES projects(id) ON DELETE RESTRICT,
    boot_policy TEXT NOT NULL DEFAULT 'recover_safe_then_hold'
        CHECK(boot_policy IN ('manual_hold','recover_safe_then_hold','reconcile_then_resume_queue')),
    hold BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE TABLE run_recovery_decisions (
    run_id UUID PRIMARY KEY REFERENCES runs(id) ON DELETE RESTRICT,
    assessment TEXT NOT NULL CHECK(assessment IN ('not_started_confirmed','partial_work_observed','external_effect_possible','unknown')),
    accepted_command_id UUID NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    recovery_queue_entry_id UUID REFERENCES queue_entries(id) ON DELETE RESTRICT
);
CREATE TABLE run_inventory_receipts (
    message_id UUID NOT NULL,
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE RESTRICT,
    PRIMARY KEY(message_id,run_id)
);
CREATE TABLE project_boot_reconciliations (
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    host_id TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    PRIMARY KEY(project_id,host_id,boot_id)
);
