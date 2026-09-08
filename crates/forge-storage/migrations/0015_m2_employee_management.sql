-- Employee identity can schedule concurrent independent Runs. Logical authority
-- and non-quiescent physical environments both consume its configured capacity.
ALTER TABLE employees ADD COLUMN max_concurrent_runs INTEGER NOT NULL DEFAULT 1
    CHECK (max_concurrent_runs BETWEEN 1 AND 65535);

-- Old canonical snapshots had no revision; the indexed value is authoritative.
UPDATE employees SET revision = GREATEST(revision, 1),
    canonical_snapshot = canonical_snapshot || jsonb_build_object(
        'revision', GREATEST(revision, 1), 'max_concurrent_runs', 1);

DROP INDEX one_physical_run_per_employee;
CREATE INDEX active_employee_run_reservations
    ON run_environment_reservations(employee_id) WHERE released_at IS NULL;
CREATE INDEX active_employee_leases
    ON leases(employee_id) WHERE lease_state = 'active';
