-- One database currently owns one local Supervisor. This row is also the short
-- transaction lock shared by every admission and operator configuration change.
CREATE TABLE local_admission_policy (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    host_max_runs INTEGER NOT NULL CHECK (host_max_runs BETWEEN 1 AND 65535),
    project_max_runs INTEGER NOT NULL CHECK (project_max_runs BETWEEN 1 AND 65535),
    credential_account_max_runs INTEGER NOT NULL CHECK (credential_account_max_runs BETWEEN 1 AND 65535),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
INSERT INTO local_admission_policy(host_max_runs,project_max_runs,credential_account_max_runs)
VALUES(16,8,4);

-- A logical Lease and its physical environment represent ONE occupied Run.
-- Revocation, provider failure and elapsed Lease expiry do not prove quiescence.
CREATE VIEW occupied_execution_runs AS
SELECT r.id,r.project_id,r.purpose,r.run_spec
FROM runs r JOIN leases l ON forge_lease_owns_run(l,r)
WHERE l.lease_state='active'
   OR EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=r.id AND e.released_at IS NULL);

-- A read predicate for selection only. Callers MUST hold local_admission_policy
-- FOR UPDATE, then evaluate this in a fresh READ COMMITTED statement and retain
-- the lock through Run insertion/commit. The final issuance repeats the check.
CREATE FUNCTION forge_admission_available(p_project UUID,p_profile JSONB)
RETURNS BOOLEAN LANGUAGE SQL STABLE AS $$
WITH occupied AS MATERIALIZED (SELECT * FROM occupied_execution_runs)
SELECT (SELECT count(*) FROM occupied)<p.host_max_runs
   AND (SELECT count(*) FROM occupied WHERE project_id=p_project)<p.project_max_runs
   AND (p_profile IS NULL OR
       (SELECT count(*) FROM occupied WHERE purpose<>'hook' AND (
          run_spec->'binding'->'execution_profile'->'credential_binding'->>'secret_id'
              =p_profile->'credential_binding'->>'secret_id'
          OR (p_profile->'credential_binding'->>'account_id' IS NOT NULL
              AND run_spec->'binding'->'execution_profile'->>'provider_id'=p_profile->>'provider_id'
              AND run_spec->'binding'->'execution_profile'->'credential_binding'->>'account_id'
                  =p_profile->'credential_binding'->>'account_id')))<p.credential_account_max_runs)
FROM local_admission_policy p WHERE singleton
$$;
