-- M0 domain snapshots retain private typed fields while queryable columns stay
-- normalized for scheduler, audit, and API projections. Core writes both forms
-- in the same transaction; snapshots are not a second source of truth.

ALTER TABLE projects
    ADD COLUMN canonical_snapshot JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD CONSTRAINT projects_canonical_snapshot_is_object
        CHECK (jsonb_typeof(canonical_snapshot) = 'object');

ALTER TABLE employees
    ADD COLUMN canonical_snapshot JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD CONSTRAINT employees_canonical_snapshot_is_object
        CHECK (jsonb_typeof(canonical_snapshot) = 'object');

ALTER TABLE pipelines
    ADD COLUMN canonical_snapshot JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD CONSTRAINT pipelines_canonical_snapshot_is_object
        CHECK (jsonb_typeof(canonical_snapshot) = 'object');

ALTER TABLE tasks
    ADD COLUMN canonical_snapshot JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD CONSTRAINT tasks_canonical_snapshot_is_object
        CHECK (jsonb_typeof(canonical_snapshot) = 'object');

ALTER TABLE artifacts
    ADD COLUMN canonical_snapshot JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD CONSTRAINT artifacts_canonical_snapshot_is_object
        CHECK (jsonb_typeof(canonical_snapshot) = 'object');
