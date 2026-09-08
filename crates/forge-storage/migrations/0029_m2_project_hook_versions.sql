-- Owner-configured immutable versions. Reusing a name never repoints a Pipeline.
CREATE TABLE project_hook_versions (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    canonical_snapshot JSONB NOT NULL CHECK (jsonb_typeof(canonical_snapshot)='object'),
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (project_id,id),
    CHECK ((canonical_snapshot->>'id' = id::text) IS TRUE),
    CHECK ((canonical_snapshot->>'project_id' = project_id::text) IS TRUE),
    CHECK ((canonical_snapshot->>'name' = name) IS TRUE)
);
CREATE INDEX project_hook_versions_project ON project_hook_versions(project_id,created_at,id);
CREATE FUNCTION forge_reject_hook_version_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'project hook versions are immutable';
END;
$$;
CREATE TRIGGER project_hook_versions_immutable BEFORE UPDATE OR DELETE ON project_hook_versions
FOR EACH ROW EXECUTE FUNCTION forge_reject_hook_version_mutation();
