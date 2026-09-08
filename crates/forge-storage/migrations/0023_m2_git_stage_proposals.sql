-- No successful Pipeline outcome is implied by capturing a writer proposal.
CREATE TABLE git_stage_proposals (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    task_id UUID NOT NULL,
    run_id UUID NOT NULL UNIQUE,
    surface_id UUID NOT NULL,
    canonical_snapshot JSONB NOT NULL CHECK(jsonb_typeof(canonical_snapshot)='object'),
    proposal_state TEXT NOT NULL DEFAULT 'awaiting_quiescence'
        CHECK(proposal_state IN ('awaiting_quiescence','inspecting','needs_attention','accepted','superseded')),
    inspection_command_id UUID UNIQUE,
    inspection_host_id TEXT,
    inspection_boot_id TEXT,
    inspection_attempts INTEGER NOT NULL DEFAULT 0 CHECK(inspection_attempts>=0),
    inspection_requested_at TIMESTAMPTZ,
    inspection_result JSONB,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE(project_id,id),
    FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id),
    FOREIGN KEY(project_id,run_id) REFERENCES runs(project_id,id)
);
CREATE UNIQUE INDEX git_stage_proposals_pending_task_idx ON git_stage_proposals(task_id)
    WHERE proposal_state IN ('awaiting_quiescence','inspecting');
CREATE INDEX git_stage_proposals_scan_idx ON git_stage_proposals(proposal_state,updated_at);

-- Each inspection reply survives retries; a new request never rewrites old evidence.
CREATE TABLE git_inspection_receipts (
    command_id UUID PRIMARY KEY,
    proposal_id UUID NOT NULL REFERENCES git_stage_proposals(id),
    result JSONB NOT NULL CHECK(jsonb_typeof(result)='object'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- Verified candidates and writer provenance are retained independently of acceptance.
CREATE TABLE task_git_candidates (
    proposal_id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    task_id UUID NOT NULL,
    canonical_snapshot JSONB NOT NULL CHECK(jsonb_typeof(canonical_snapshot)='object'),
    verified_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY(project_id,proposal_id) REFERENCES git_stage_proposals(project_id,id),
    FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id)
);
CREATE INDEX task_git_candidates_history_idx ON task_git_candidates(project_id,task_id,verified_at);
