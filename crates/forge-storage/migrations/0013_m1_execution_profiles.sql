-- Reusing the same profile identity/revision with another value is forbidden.
CREATE TABLE execution_profile_versions (
    profile_id uuid NOT NULL,
    revision bigint NOT NULL CHECK (revision > 0),
    project_id uuid NOT NULL REFERENCES projects(id),
    snapshot jsonb NOT NULL CHECK (jsonb_typeof(snapshot) = 'object'),
    PRIMARY KEY (profile_id, revision)
);

-- Preserve historical Run definitions, including old bindings no longer assigned.
-- DISTINCT removes identical shared profiles; conflicting historical definitions
-- deliberately fail migration for explicit operator repair instead of choosing one.
INSERT INTO execution_profile_versions(profile_id,revision,project_id,snapshot)
SELECT DISTINCT (snapshot->>'id')::uuid,(snapshot->>'revision')::bigint,project_id,snapshot
FROM (
    SELECT project_id, binding->'execution_profile' AS snapshot FROM employee_runtime_bindings
    UNION ALL
    SELECT project_id, run_spec->'binding'->'execution_profile' AS snapshot FROM runs WHERE run_spec_version=2
) profiles;
