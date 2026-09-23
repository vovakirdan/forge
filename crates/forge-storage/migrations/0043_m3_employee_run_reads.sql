-- Keep Employee Run history scoped and ordered without scanning every Project Run.
CREATE INDEX runs_project_employee_created_idx
    ON runs (project_id, employee_id, created_at DESC, id DESC);
