-- Only provider-free Hooks omit an Employee. Existing provider owners stay strict.
ALTER TABLE leases ALTER COLUMN employee_id DROP NOT NULL;
ALTER TABLE runs ALTER COLUMN employee_id DROP NOT NULL;
ALTER TABLE run_environment_reservations ALTER COLUMN employee_id DROP NOT NULL;
ALTER TABLE leases ADD CONSTRAINT leases_employee_purpose_exact
    CHECK ((purpose='hook') = (employee_id IS NULL));
ALTER TABLE runs ADD CONSTRAINT runs_employee_purpose_exact
    CHECK ((purpose='hook') = (employee_id IS NULL));
ALTER TABLE run_environment_reservations ADD CONSTRAINT environment_employee_purpose_exact
    CHECK ((purpose='hook') = (employee_id IS NULL));

-- A nullable component makes MATCH SIMPLE skip the old composite foreign key.
-- Keep those Employee checks, and additionally bind every purpose through a
-- completely non-null physical/logical scope (including environment epoch).
ALTER TABLE leases ADD CONSTRAINT leases_runtime_identity
    UNIQUE(project_id,id,purpose,fencing_token,environment_epoch);
ALTER TABLE runs ADD CONSTRAINT runs_runtime_identity
    UNIQUE(project_id,id,purpose,lease_fencing_token,environment_epoch);
ALTER TABLE runs ADD CONSTRAINT runs_runtime_lease_fk
    FOREIGN KEY(project_id,lease_id,purpose,lease_fencing_token,environment_epoch)
    REFERENCES leases(project_id,id,purpose,fencing_token,environment_epoch);
ALTER TABLE run_environment_reservations ADD CONSTRAINT environment_runtime_owner_fk
    FOREIGN KEY(project_id,run_id,purpose,fencing_token,environment_epoch)
    REFERENCES runs(project_id,id,purpose,lease_fencing_token,environment_epoch);

-- Hook admission and its exact invocation-owner branch are introduced by 0031.
-- Until then the existing purpose XOR and ELSE FALSE reject Hook rows entirely.
CREATE OR REPLACE FUNCTION forge_lease_owns_run(l leases,r runs) RETURNS BOOLEAN LANGUAGE SQL IMMUTABLE AS $$
 SELECT l.id=r.lease_id AND l.project_id=r.project_id
  AND l.employee_id IS NOT DISTINCT FROM r.employee_id
  AND l.fencing_token=r.lease_fencing_token AND l.environment_epoch=r.environment_epoch
  AND l.purpose=r.purpose AND CASE r.purpose
   WHEN 'task_stage' THEN l.task_id=r.task_id AND l.queue_entry_id=r.queue_entry_id
   WHEN 'communication' THEN l.communication_assignment_id=r.communication_assignment_id
   WHEN 'resolution' THEN l.resolution_assignment_id=r.resolution_assignment_id
   ELSE FALSE END;
$$;
