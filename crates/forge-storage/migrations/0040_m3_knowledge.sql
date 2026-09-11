-- Forge owns revision history and ACLs. The external index is a rebuildable projection.
CREATE TABLE knowledge_pages (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    revision bigint NOT NULL CHECK (revision > 0),
    kind text NOT NULL CHECK (kind IN ('introduction','architecture','guide','policy','decision')),
    status text NOT NULL CHECK (status IN ('draft','published','withdrawn')),
    content_hash text NOT NULL CHECK (content_hash ~ '^[0-9a-f]{64}$'),
    canonical_snapshot jsonb NOT NULL CHECK (jsonb_typeof(canonical_snapshot) = 'object'),
    UNIQUE(project_id,id)
);
CREATE TABLE knowledge_page_revisions (
    page_id uuid NOT NULL REFERENCES knowledge_pages(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    revision bigint NOT NULL CHECK (revision > 0),
    canonical_snapshot jsonb NOT NULL CHECK (jsonb_typeof(canonical_snapshot) = 'object'),
    PRIMARY KEY(page_id,revision),
    FOREIGN KEY(project_id,page_id) REFERENCES knowledge_pages(project_id,id)
);
CREATE TRIGGER knowledge_page_revisions_append_only BEFORE UPDATE OR DELETE ON knowledge_page_revisions
    FOR EACH ROW EXECUTE FUNCTION forge_reject_append_only_write();
CREATE INDEX knowledge_pages_published_idx ON knowledge_pages(project_id,kind,id) WHERE status='published';

CREATE TABLE derived_memory_entries (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    revision bigint NOT NULL CHECK (revision > 0),
    employee_id uuid,
    task_id uuid,
    content_hash text NOT NULL CHECK (content_hash ~ '^[0-9a-f]{64}$'),
    withdrawn boolean NOT NULL DEFAULT false,
    canonical_snapshot jsonb NOT NULL CHECK (jsonb_typeof(canonical_snapshot) = 'object'),
    UNIQUE(project_id,id),
    FOREIGN KEY(project_id,employee_id) REFERENCES employees(project_id,id),
    FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id)
);
CREATE TABLE derived_memory_revisions (
    entry_id uuid NOT NULL REFERENCES derived_memory_entries(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    revision bigint NOT NULL CHECK (revision > 0),
    canonical_snapshot jsonb NOT NULL CHECK (jsonb_typeof(canonical_snapshot) = 'object'),
    PRIMARY KEY(entry_id,revision),
    FOREIGN KEY(project_id,entry_id) REFERENCES derived_memory_entries(project_id,id)
);
CREATE TRIGGER derived_memory_revisions_append_only BEFORE UPDATE OR DELETE ON derived_memory_revisions
    FOR EACH ROW EXECUTE FUNCTION forge_reject_append_only_write();
CREATE INDEX derived_memory_scope_idx ON derived_memory_entries(project_id,employee_id,id) WHERE NOT withdrawn;

CREATE TABLE knowledge_projection_operations (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    object_kind text NOT NULL CHECK (object_kind IN ('knowledge_page','derived_memory')),
    object_id uuid NOT NULL,
    revision bigint NOT NULL CHECK (revision > 0),
    content_hash text NOT NULL CHECK (content_hash ~ '^[0-9a-f]{64}$'),
    index_ready boolean NOT NULL DEFAULT false,
    retirement_requested boolean NOT NULL DEFAULT false,
    retired boolean NOT NULL DEFAULT false,
    attempt_count integer NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_error_code text,
    not_before timestamptz NOT NULL DEFAULT clock_timestamp(),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(object_kind,object_id,revision),
    CHECK (last_error_code IS NULL OR length(last_error_code) BETWEEN 1 AND 100)
);
CREATE INDEX knowledge_projection_pending_idx ON knowledge_projection_operations(project_id,id) WHERE NOT index_ready AND NOT retired;
