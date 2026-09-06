CREATE TABLE run_evidence_streams (
    run_id uuid NOT NULL REFERENCES runs(id),
    stream text NOT NULL CHECK(stream IN ('stdout','stderr')),
    incomplete boolean NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(run_id,stream)
);
CREATE TABLE run_evidence_objects (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL REFERENCES runs(id),
    receipt jsonb NOT NULL CHECK(jsonb_typeof(receipt)='object'),
    stored boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX run_evidence_by_run ON run_evidence_objects(run_id,id);
