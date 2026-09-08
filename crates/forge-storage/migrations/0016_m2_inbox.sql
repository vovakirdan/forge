-- A conversation is canonical Project/Employee state, not a fabricated Task.
CREATE TABLE employee_threads (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id),
    employee_id UUID NOT NULL,
    task_id UUID,
    revision BIGINT NOT NULL CHECK (revision > 0),
    last_sequence BIGINT NOT NULL DEFAULT 0 CHECK (last_sequence >= 0),
    canonical_snapshot JSONB NOT NULL CHECK (jsonb_typeof(canonical_snapshot) = 'object'),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (project_id, id),
    UNIQUE (project_id, employee_id, id),
    CHECK (revision = last_sequence + 1),
    FOREIGN KEY (project_id, employee_id) REFERENCES employees(project_id, id),
    FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id)
);

CREATE TABLE employee_messages (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    employee_id UUID NOT NULL,
    thread_id UUID NOT NULL,
    sequence BIGINT NOT NULL CHECK (sequence > 0),
    target_task_id UUID,
    target_run_id UUID,
    requirement TEXT NOT NULL CHECK (requirement IN ('informational', 'acknowledged', 'answered')),
    reply_to UUID,
    canonical_snapshot JSONB NOT NULL CHECK (jsonb_typeof(canonical_snapshot) = 'object'),
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (project_id, id),
    UNIQUE (thread_id, id),
    UNIQUE (thread_id, sequence),
    CHECK (reply_to IS NULL OR reply_to <> id),
    FOREIGN KEY (project_id, employee_id, thread_id)
        REFERENCES employee_threads(project_id, employee_id, id),
    FOREIGN KEY (project_id, target_task_id) REFERENCES tasks(project_id, id),
    FOREIGN KEY (project_id, target_run_id) REFERENCES runs(project_id, id),
    FOREIGN KEY (thread_id, reply_to) REFERENCES employee_messages(thread_id, id)
);

-- Delivery receipts never rewrite message content. Runtime acceptance alone
-- does not satisfy an Employee acknowledgement/answer requirement.
CREATE TABLE employee_message_receipts (
    message_id UUID NOT NULL,
    project_id UUID NOT NULL,
    run_id UUID NOT NULL,
    fencing_token BIGINT NOT NULL CHECK (fencing_token > 0),
    environment_epoch BIGINT NOT NULL CHECK (environment_epoch > 0),
    kind TEXT NOT NULL CHECK (kind IN ('runtime_accepted', 'acknowledged', 'answered')),
    reply_id UUID,
    canonical_snapshot JSONB NOT NULL CHECK (jsonb_typeof(canonical_snapshot) = 'object'),
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (message_id, run_id, fencing_token, environment_epoch, kind),
    CHECK ((kind = 'answered') = (reply_id IS NOT NULL)),
    FOREIGN KEY (project_id, message_id) REFERENCES employee_messages(project_id, id),
    FOREIGN KEY (project_id, run_id) REFERENCES runs(project_id, id),
    FOREIGN KEY (project_id, reply_id) REFERENCES employee_messages(project_id, id)
);

CREATE INDEX employee_threads_inbox_idx ON employee_threads(project_id, employee_id, updated_at, id);
CREATE INDEX employee_messages_task_control_idx ON employee_messages(project_id, target_task_id, created_at)
    WHERE target_task_id IS NOT NULL AND requirement <> 'informational';
