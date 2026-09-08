-- Immutable operator-selected source allowlist. Registration does not verify host Git state.
CREATE TABLE project_repositories (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    canonical_snapshot JSONB NOT NULL CHECK (jsonb_typeof(canonical_snapshot) = 'object'),
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (project_id, id),
    UNIQUE (project_id, name),
    CHECK (canonical_snapshot->>'id' = id::text),
    CHECK (canonical_snapshot->>'project_id' = project_id::text),
    CHECK (canonical_snapshot->>'name' = name)
);

CREATE FUNCTION forge_reject_repository_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'project repository is immutable';
END;
$$;
CREATE TRIGGER project_repositories_immutable BEFORE UPDATE OR DELETE ON project_repositories
FOR EACH ROW EXECUTE FUNCTION forge_reject_repository_mutation();

ALTER TABLE tasks ADD COLUMN project_repository_id UUID;
ALTER TABLE tasks ADD CONSTRAINT task_project_repository_scope
    FOREIGN KEY (project_id, project_repository_id) REFERENCES project_repositories(project_id, id);
ALTER TABLE tasks ADD CONSTRAINT task_git_projection_present CHECK (
    project_repository_id IS NULL OR (base_sha IS NOT NULL AND task_work_surface_id IS NOT NULL)
);
CREATE UNIQUE INDEX tasks_git_surface_unique ON tasks (task_work_surface_id)
    WHERE project_repository_id IS NOT NULL;

CREATE FUNCTION forge_validate_task_git_binding() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    binding JSONB;
    registered JSONB;
BEGIN
    binding := NEW.canonical_snapshot->'work_surface'->'git';
    IF TG_OP = 'UPDATE' AND OLD.project_repository_id IS NOT NULL
        AND OLD.canonical_snapshot->'work_surface' IS DISTINCT FROM NEW.canonical_snapshot->'work_surface' THEN
        RAISE EXCEPTION 'Task Git binding is immutable';
    END IF;
    IF NEW.project_repository_id IS NULL THEN
        IF binding IS NOT NULL THEN RAISE EXCEPTION 'Task Git projection is missing'; END IF;
        RETURN NEW;
    END IF;
    SELECT canonical_snapshot INTO registered FROM project_repositories
        WHERE id=NEW.project_repository_id AND project_id=NEW.project_id;
    IF registered IS NULL OR binding IS NULL
        OR binding->>'repository_id' IS DISTINCT FROM NEW.project_repository_id::text
        OR binding->>'surface_id' IS DISTINCT FROM NEW.task_work_surface_id::text
        OR binding->>'initial_base' IS DISTINCT FROM NEW.base_sha
        OR binding->>'source' IS DISTINCT FROM registered->>'source'
        OR binding->>'target_ref' IS DISTINCT FROM registered->>'target_ref' THEN
        RAISE EXCEPTION 'Task Git binding violates repository or surface scope';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER tasks_git_binding_guard BEFORE INSERT OR UPDATE ON tasks
FOR EACH ROW EXECUTE FUNCTION forge_validate_task_git_binding();
