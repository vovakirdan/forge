-- Support bounded Project Run and retained Pipeline version pages.
CREATE INDEX runs_project_created_page_idx ON runs (project_id, created_at DESC, id DESC);
CREATE INDEX pipelines_project_name_page_idx ON pipelines (project_id, name, id);
