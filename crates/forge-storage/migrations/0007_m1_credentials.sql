-- Only authenticated ciphertext envelopes enter canonical storage.
CREATE TABLE provider_credentials (
    secret_id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    version bigint NOT NULL CHECK (version > 0),
    sealed_record jsonb NOT NULL CHECK (jsonb_typeof(sealed_record)='object'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE run_credential_snapshots (
    run_id uuid PRIMARY KEY REFERENCES runs(id),
    secret_id uuid NOT NULL REFERENCES provider_credentials(secret_id),
    version bigint NOT NULL CHECK (version > 0),
    sealed_record jsonb NOT NULL CHECK (jsonb_typeof(sealed_record)='object')
);
CREATE TABLE credential_writeback_conflicts (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL REFERENCES runs(id),
    reason text NOT NULL,
    recovery jsonb NOT NULL CHECK (jsonb_typeof(recovery)='object'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
