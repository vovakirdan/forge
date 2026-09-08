-- Explicit management decisions, never synthetic Employee acknowledgements.
CREATE TABLE employee_message_waivers (
    message_id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    canonical_snapshot JSONB NOT NULL CHECK (jsonb_typeof(canonical_snapshot)='object'),
    created_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (project_id,message_id) REFERENCES employee_messages(project_id,id)
);
