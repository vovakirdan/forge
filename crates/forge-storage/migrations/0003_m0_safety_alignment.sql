-- Align normalized projections with the canonical domain model before Core
-- writes aggregates. Existing non-null values in the mistakenly named column
-- have ambiguous semantics, so do not silently reinterpret or discard them.

ALTER TABLE projects
    ALTER COLUMN execution_enabled SET DEFAULT FALSE;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM projects WHERE revision = 0)
        OR EXISTS (SELECT 1 FROM tasks WHERE revision = 0) THEN
        RAISE EXCEPTION
            'cannot migrate zero aggregate revisions: values require explicit operator remediation';
    END IF;
END;
$$;

ALTER TABLE projects
    ALTER COLUMN revision SET DEFAULT 1;

ALTER TABLE projects
    DROP CONSTRAINT projects_revision_non_negative;

ALTER TABLE projects
    ADD CONSTRAINT projects_revision_positive CHECK (revision >= 1);

ALTER TABLE tasks
    ALTER COLUMN revision SET DEFAULT 1;

ALTER TABLE tasks
    DROP CONSTRAINT tasks_revision_non_negative;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_revision_positive CHECK (revision >= 1);

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM tasks
        WHERE resume_to_stage_id IS NOT NULL
    ) THEN
        RAISE EXCEPTION
            'cannot migrate tasks.resume_to_stage_id: values require explicit operator remediation';
    END IF;
END;
$$;

ALTER TABLE tasks
    DROP CONSTRAINT tasks_resume_to_stage_not_blank;

ALTER TABLE tasks
    DROP COLUMN resume_to_stage_id;

ALTER TABLE tasks
    ADD COLUMN resume_to_lifecycle TEXT;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_resume_to_lifecycle_known CHECK (
        resume_to_lifecycle IS NULL
        OR resume_to_lifecycle IN ('ready', 'in_progress')
    );

ALTER TABLE event_log
    ADD COLUMN reason TEXT;
