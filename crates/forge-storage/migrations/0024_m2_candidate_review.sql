CREATE TABLE candidate_reviews (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    task_id UUID NOT NULL,
    run_id UUID NOT NULL UNIQUE,
    candidate_proposal_id UUID NOT NULL,
    canonical_snapshot JSONB NOT NULL CHECK(jsonb_typeof(canonical_snapshot)='object'),
    recorded_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id),
    FOREIGN KEY(project_id,run_id) REFERENCES runs(project_id,id),
    FOREIGN KEY(project_id,candidate_proposal_id) REFERENCES git_stage_proposals(project_id,id)
);
CREATE INDEX candidate_reviews_history_idx ON candidate_reviews(project_id,task_id,recorded_at);
