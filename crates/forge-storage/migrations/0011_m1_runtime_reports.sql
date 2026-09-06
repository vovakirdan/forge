-- Diagnostic provider receipts are not Task outcomes or acceptance decisions.
CREATE TABLE run_runtime_reports (
    run_id uuid PRIMARY KEY REFERENCES runs(id),
    report jsonb NOT NULL CHECK (jsonb_typeof(report) = 'object'),
    recorded_at timestamptz NOT NULL DEFAULT now()
);
