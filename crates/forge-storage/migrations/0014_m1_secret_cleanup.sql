CREATE TABLE run_secret_cleanup (
    run_id uuid PRIMARY KEY REFERENCES runs(id),
    outcome text NOT NULL CHECK (outcome IN ('removed','unsafe_auth_retained')),
    completed_at timestamptz NOT NULL DEFAULT now()
);
