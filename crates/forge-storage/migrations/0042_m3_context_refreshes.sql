-- Explicit tool refresh evidence never overwrites the initial Run context.
CREATE TABLE run_knowledge_context_refreshes (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    run_id uuid NOT NULL,
    employee_id uuid NOT NULL,
    initial_context_snapshot_id uuid NOT NULL,
    message_id uuid NOT NULL UNIQUE REFERENCES gateway_submissions(message_id),
    content_hash text NOT NULL CHECK (content_hash ~ '^[0-9a-f]{64}$'),
    context_snapshot jsonb NOT NULL CHECK (jsonb_typeof(context_snapshot)='object'),
    delivery text NOT NULL DEFAULT 'tool_response' CHECK (delivery='tool_response'),
    created_at timestamptz NOT NULL,
    FOREIGN KEY (project_id,run_id) REFERENCES runs(project_id,id),
    FOREIGN KEY (project_id,employee_id) REFERENCES employees(project_id,id),
    CHECK ((context_snapshot->>'context_snapshot_id')::uuid=id),
    CHECK ((context_snapshot->>'run_id')::uuid=run_id),
    CHECK ((context_snapshot->>'project_id')::uuid=project_id),
    CHECK ((context_snapshot->>'employee_id')::uuid=employee_id)
);
CREATE INDEX run_knowledge_context_refresh_history ON run_knowledge_context_refreshes(project_id,run_id,id);
CREATE TRIGGER run_knowledge_context_refresh_immutable BEFORE UPDATE OR DELETE ON run_knowledge_context_refreshes
    FOR EACH ROW EXECUTE FUNCTION forge_reject_append_only_write();
