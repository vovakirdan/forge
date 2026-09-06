-- Per-Run proxy intent is durable before LiteLLM issuance. Ciphertext only.
CREATE TABLE run_proxy_keys (
    run_id uuid PRIMARY KEY REFERENCES runs(id),
    spec jsonb NOT NULL CHECK(jsonb_typeof(spec)='object'),
    sealed_key jsonb NOT NULL CHECK(jsonb_typeof(sealed_key)='object'),
    key_hash text,
    revoked_at timestamptz,
    usage jsonb,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE run_auth_writeback_receipts (
    run_id uuid PRIMARY KEY REFERENCES runs(id),
    outcome text NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
