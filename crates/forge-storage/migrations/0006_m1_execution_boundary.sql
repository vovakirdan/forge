-- Physical ownership outlives the logical Lease and accepted stage outcome.
CREATE TABLE run_environment_reservations (
    run_id uuid PRIMARY KEY REFERENCES runs(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    task_id uuid NOT NULL REFERENCES tasks(id),
    employee_id uuid NOT NULL REFERENCES employees(id),
    surface_id uuid,
    fencing_token bigint NOT NULL CHECK (fencing_token > 0),
    environment_epoch bigint NOT NULL CHECK (environment_epoch > 0),
    state text NOT NULL DEFAULT 'reserved' CHECK (state IN ('reserved', 'active', 'unknown', 'quiescent')),
    host_id text,
    boot_id text,
    environment_id text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    observed_at timestamptz,
    released_at timestamptz
);
CREATE UNIQUE INDEX one_physical_writer_per_task ON run_environment_reservations(task_id) WHERE released_at IS NULL;
CREATE UNIQUE INDEX one_physical_writer_per_surface ON run_environment_reservations(surface_id) WHERE released_at IS NULL AND surface_id IS NOT NULL;
CREATE UNIQUE INDEX one_physical_run_per_employee ON run_environment_reservations(employee_id) WHERE released_at IS NULL;

CREATE TABLE run_incidents (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    run_id uuid NOT NULL REFERENCES runs(id),
    kind text NOT NULL,
    assessment text NOT NULL CHECK (assessment IN ('not_started_confirmed', 'partial_work_observed', 'external_effect_possible', 'unknown')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    resolved_at timestamptz,
    resolution text,
    UNIQUE (run_id, kind)
);

CREATE TABLE task_handoffs (
    run_id uuid PRIMARY KEY REFERENCES runs(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    task_id uuid NOT NULL REFERENCES tasks(id),
    kind text NOT NULL CHECK (kind IN ('accepted', 'interrupted')),
    body jsonb NOT NULL CHECK (jsonb_typeof(body) = 'object'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE employee_runtime_bindings (
    employee_id uuid PRIMARY KEY REFERENCES employees(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    binding jsonb NOT NULL CHECK (jsonb_typeof(binding) = 'object'),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- Gateway requests do not consume the independently ordered Supervisor stream.
CREATE TABLE gateway_submissions (
    message_id uuid PRIMARY KEY,
    run_id uuid NOT NULL REFERENCES runs(id),
    fencing_token bigint NOT NULL CHECK (fencing_token > 0),
    environment_epoch bigint NOT NULL CHECK (environment_epoch > 0),
    payload_hash text NOT NULL,
    receipt jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
