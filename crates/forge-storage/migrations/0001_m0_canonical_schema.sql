-- Canonical M0 storage. Runtime processes only validate this schema; the
-- forge-migrate operator command is the sole migration entry point.

CREATE SEQUENCE lease_fencing_token_seq
    AS BIGINT
    START WITH 1
    INCREMENT BY 1
    MINVALUE 1
    NO MAXVALUE
    CACHE 1;

CREATE FUNCTION forge_set_updated_at()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END;
$$;

CREATE FUNCTION forge_reject_append_only_write()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION '% is append-only; % is forbidden', TG_TABLE_NAME, TG_OP
        USING ERRCODE = '55000';
    RETURN NULL;
END;
$$;

CREATE FUNCTION forge_reject_pipeline_delete()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'pipelines are soft-deleted; DELETE is forbidden'
        USING ERRCODE = '55000';
    RETURN NULL;
END;
$$;

CREATE FUNCTION forge_guard_lease_identity()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.project_id IS DISTINCT FROM NEW.project_id
        OR OLD.task_id IS DISTINCT FROM NEW.task_id
        OR OLD.queue_entry_id IS DISTINCT FROM NEW.queue_entry_id
        OR OLD.employee_id IS DISTINCT FROM NEW.employee_id
        OR OLD.fencing_token IS DISTINCT FROM NEW.fencing_token
        OR OLD.task_work_surface_id IS DISTINCT FROM NEW.task_work_surface_id
        OR OLD.lease_scope IS DISTINCT FROM NEW.lease_scope
        OR OLD.resource_reservation IS DISTINCT FROM NEW.resource_reservation THEN
        RAISE EXCEPTION 'lease identity and granted scope are immutable'
            USING ERRCODE = '55000';
    END IF;

    IF NEW.environment_epoch < OLD.environment_epoch THEN
        RAISE EXCEPTION 'lease environment epoch cannot decrease'
            USING ERRCODE = '22000';
    END IF;

    RETURN NEW;
END;
$$;

CREATE FUNCTION forge_guard_run_identity()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.project_id IS DISTINCT FROM NEW.project_id
        OR OLD.task_id IS DISTINCT FROM NEW.task_id
        OR OLD.queue_entry_id IS DISTINCT FROM NEW.queue_entry_id
        OR OLD.lease_id IS DISTINCT FROM NEW.lease_id
        OR OLD.employee_id IS DISTINCT FROM NEW.employee_id
        OR OLD.stage_id IS DISTINCT FROM NEW.stage_id
        OR OLD.attempt_number IS DISTINCT FROM NEW.attempt_number
        OR OLD.lease_fencing_token IS DISTINCT FROM NEW.lease_fencing_token
        OR OLD.run_spec_version IS DISTINCT FROM NEW.run_spec_version
        OR OLD.run_spec IS DISTINCT FROM NEW.run_spec
        OR OLD.context_manifest IS DISTINCT FROM NEW.context_manifest THEN
        RAISE EXCEPTION 'run identity, RunSpec, and context manifest are immutable'
            USING ERRCODE = '55000';
    END IF;

    IF NEW.environment_epoch < OLD.environment_epoch THEN
        RAISE EXCEPTION 'run environment epoch cannot decrease'
            USING ERRCODE = '22000';
    END IF;

    IF NEW.last_sequence < OLD.last_sequence THEN
        RAISE EXCEPTION 'run event sequence cannot decrease'
            USING ERRCODE = '22000';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TABLE projects (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    next_task_sequence BIGINT NOT NULL DEFAULT 1,
    execution_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    execution_stop_reason TEXT,
    priority_scheme JSONB NOT NULL DEFAULT
        '{"levels":[{"id":"low","rank":1},{"id":"normal","rank":2},{"id":"high","rank":3}]}'::JSONB,
    cancellation_reasons JSONB NOT NULL DEFAULT '["unspecified"]'::JSONB,
    properties JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT projects_name_not_blank CHECK (length(btrim(name)) > 0),
    CONSTRAINT projects_revision_non_negative CHECK (revision >= 0),
    CONSTRAINT projects_next_task_sequence_positive CHECK (next_task_sequence > 0),
    CONSTRAINT projects_priority_scheme_is_object CHECK (jsonb_typeof(priority_scheme) = 'object'),
    CONSTRAINT projects_cancellation_reasons_is_array CHECK (jsonb_typeof(cancellation_reasons) = 'array'),
    CONSTRAINT projects_properties_is_object CHECK (jsonb_typeof(properties) = 'object')
);

CREATE TABLE employees (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    employee_state TEXT NOT NULL DEFAULT 'active',
    roles JSONB NOT NULL DEFAULT '[]'::JSONB,
    skills JSONB NOT NULL DEFAULT '[]'::JSONB,
    stage_eligibility JSONB NOT NULL DEFAULT '{}'::JSONB,
    provider_preferences JSONB NOT NULL DEFAULT '{}'::JSONB,
    runtime_policy JSONB NOT NULL DEFAULT '{}'::JSONB,
    properties JSONB NOT NULL DEFAULT '{}'::JSONB,
    revision BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT employees_name_not_blank CHECK (length(btrim(name)) > 0),
    CONSTRAINT employees_state_known CHECK (employee_state IN ('active', 'disabled', 'retired')),
    CONSTRAINT employees_roles_is_array CHECK (jsonb_typeof(roles) = 'array'),
    CONSTRAINT employees_skills_is_array CHECK (jsonb_typeof(skills) = 'array'),
    CONSTRAINT employees_stage_eligibility_is_object CHECK (jsonb_typeof(stage_eligibility) = 'object'),
    CONSTRAINT employees_provider_preferences_is_object CHECK (jsonb_typeof(provider_preferences) = 'object'),
    CONSTRAINT employees_runtime_policy_is_object CHECK (jsonb_typeof(runtime_policy) = 'object'),
    CONSTRAINT employees_properties_is_object CHECK (jsonb_typeof(properties) = 'object'),
    CONSTRAINT employees_revision_non_negative CHECK (revision >= 0),
    CONSTRAINT employees_project_id_id_unique UNIQUE (project_id, id),
    CONSTRAINT employees_project_name_unique UNIQUE (project_id, name)
);

CREATE INDEX employees_project_state_idx
    ON employees (project_id, employee_state, name);

CREATE TABLE pipelines (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    default_version_id UUID,
    revision BIGINT NOT NULL DEFAULT 0,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT pipelines_name_not_blank CHECK (length(btrim(name)) > 0),
    CONSTRAINT pipelines_revision_non_negative CHECK (revision >= 0),
    CONSTRAINT pipelines_project_id_id_unique UNIQUE (project_id, id)
);

CREATE UNIQUE INDEX pipelines_active_project_name_unique
    ON pipelines (project_id, name)
    WHERE deleted_at IS NULL;

CREATE TABLE pipeline_versions (
    id UUID PRIMARY KEY,
    pipeline_id UUID NOT NULL REFERENCES pipelines (id) ON DELETE RESTRICT,
    version BIGINT NOT NULL,
    definition JSONB NOT NULL,
    definition_hash TEXT,
    created_by JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    published_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT pipeline_versions_version_positive CHECK (version > 0),
    CONSTRAINT pipeline_versions_definition_is_object CHECK (jsonb_typeof(definition) = 'object'),
    CONSTRAINT pipeline_versions_created_by_is_object CHECK (jsonb_typeof(created_by) = 'object'),
    CONSTRAINT pipeline_versions_hash_not_blank CHECK (
        definition_hash IS NULL OR length(btrim(definition_hash)) > 0
    ),
    CONSTRAINT pipeline_versions_pipeline_version_unique UNIQUE (pipeline_id, version),
    CONSTRAINT pipeline_versions_pipeline_id_id_unique UNIQUE (pipeline_id, id)
);

ALTER TABLE pipelines
    ADD CONSTRAINT pipelines_default_version_belongs_to_pipeline_fk
    FOREIGN KEY (id, default_version_id)
    REFERENCES pipeline_versions (pipeline_id, id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE tasks (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    task_sequence BIGINT NOT NULL,
    task_key TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    definition_of_done TEXT,
    task_kind TEXT NOT NULL,
    properties JSONB NOT NULL DEFAULT '{}'::JSONB,
    priority_level_id TEXT NOT NULL,
    priority_rank INTEGER NOT NULL,
    lifecycle TEXT NOT NULL DEFAULT 'draft',
    pipeline_id UUID,
    pipeline_version_id UUID,
    current_stage_id TEXT,
    wait_conditions JSONB NOT NULL DEFAULT '[]'::JSONB,
    resume_to_stage_id TEXT,
    cancellation_reason_id TEXT,
    cancellation_note TEXT,
    assignment_policy JSONB NOT NULL DEFAULT '{}'::JSONB,
    reviewer_policy JSONB NOT NULL DEFAULT '{}'::JSONB,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    base_sha TEXT,
    task_work_surface_id UUID,
    created_by JSONB NOT NULL DEFAULT '{}'::JSONB,
    revision BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT tasks_sequence_positive CHECK (task_sequence > 0),
    CONSTRAINT tasks_key_not_blank CHECK (length(btrim(task_key)) > 0),
    CONSTRAINT tasks_title_not_blank CHECK (length(btrim(title)) > 0),
    CONSTRAINT tasks_definition_of_done_not_blank CHECK (
        definition_of_done IS NULL OR length(btrim(definition_of_done)) > 0
    ),
    CONSTRAINT tasks_kind_known CHECK (task_kind IN ('delivery', 'analysis')),
    CONSTRAINT tasks_properties_is_object CHECK (jsonb_typeof(properties) = 'object'),
    CONSTRAINT tasks_priority_level_not_blank CHECK (length(btrim(priority_level_id)) > 0),
    CONSTRAINT tasks_priority_rank_non_negative CHECK (priority_rank >= 0),
    CONSTRAINT tasks_lifecycle_known CHECK (
        lifecycle IN ('draft', 'ready', 'in_progress', 'waiting', 'done', 'cancelled')
    ),
    CONSTRAINT tasks_pipeline_pair_is_consistent CHECK (
        (pipeline_id IS NULL AND pipeline_version_id IS NULL)
        OR (pipeline_id IS NOT NULL AND pipeline_version_id IS NOT NULL)
    ),
    CONSTRAINT tasks_active_lifecycle_has_stage CHECK (
        lifecycle IN ('draft', 'cancelled')
        OR (
            pipeline_id IS NOT NULL
            AND pipeline_version_id IS NOT NULL
            AND current_stage_id IS NOT NULL
            AND length(btrim(current_stage_id)) > 0
        )
    ),
    CONSTRAINT tasks_current_stage_not_blank CHECK (
        current_stage_id IS NULL OR length(btrim(current_stage_id)) > 0
    ),
    CONSTRAINT tasks_wait_conditions_is_array CHECK (jsonb_typeof(wait_conditions) = 'array'),
    CONSTRAINT tasks_resume_to_stage_not_blank CHECK (
        resume_to_stage_id IS NULL OR length(btrim(resume_to_stage_id)) > 0
    ),
    CONSTRAINT tasks_cancellation_data_matches_lifecycle CHECK (
        (lifecycle = 'cancelled'
            AND cancellation_reason_id IS NOT NULL
            AND length(btrim(cancellation_reason_id)) > 0)
        OR (lifecycle <> 'cancelled'
            AND cancellation_reason_id IS NULL
            AND cancellation_note IS NULL)
    ),
    CONSTRAINT tasks_assignment_policy_is_object CHECK (jsonb_typeof(assignment_policy) = 'object'),
    CONSTRAINT tasks_reviewer_policy_is_object CHECK (jsonb_typeof(reviewer_policy) = 'object'),
    CONSTRAINT tasks_attempt_count_non_negative CHECK (attempt_count >= 0),
    CONSTRAINT tasks_created_by_is_object CHECK (jsonb_typeof(created_by) = 'object'),
    CONSTRAINT tasks_revision_non_negative CHECK (revision >= 0),
    CONSTRAINT tasks_project_sequence_unique UNIQUE (project_id, task_sequence),
    CONSTRAINT tasks_project_key_unique UNIQUE (project_id, task_key),
    CONSTRAINT tasks_project_id_id_unique UNIQUE (project_id, id),
    CONSTRAINT tasks_pipeline_belongs_to_project_fk
        FOREIGN KEY (project_id, pipeline_id)
        REFERENCES pipelines (project_id, id)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT tasks_pipeline_version_matches_pipeline_fk
        FOREIGN KEY (pipeline_id, pipeline_version_id)
        REFERENCES pipeline_versions (pipeline_id, id)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX tasks_project_lifecycle_priority_idx
    ON tasks (project_id, lifecycle, priority_rank DESC, task_sequence ASC);

CREATE INDEX tasks_project_pipeline_stage_idx
    ON tasks (project_id, pipeline_id, pipeline_version_id, current_stage_id)
    WHERE pipeline_id IS NOT NULL;

CREATE TABLE task_dependencies (
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    blocker_task_id UUID NOT NULL,
    blocked_task_id UUID NOT NULL,
    required_blocker_lifecycle TEXT NOT NULL DEFAULT 'done',
    created_by JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT task_dependencies_not_self_referential CHECK (blocker_task_id <> blocked_task_id),
    CONSTRAINT task_dependencies_required_lifecycle_known CHECK (
        required_blocker_lifecycle IN (
            'draft', 'ready', 'in_progress', 'waiting', 'done', 'cancelled'
        )
    ),
    CONSTRAINT task_dependencies_created_by_is_object CHECK (jsonb_typeof(created_by) = 'object'),
    CONSTRAINT task_dependencies_primary_key PRIMARY KEY (blocker_task_id, blocked_task_id),
    CONSTRAINT task_dependencies_blocker_same_project_fk
        FOREIGN KEY (project_id, blocker_task_id)
        REFERENCES tasks (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT task_dependencies_blocked_same_project_fk
        FOREIGN KEY (project_id, blocked_task_id)
        REFERENCES tasks (project_id, id)
        ON DELETE RESTRICT
);

CREATE INDEX task_dependencies_blocked_idx
    ON task_dependencies (project_id, blocked_task_id, blocker_task_id);

CREATE TABLE event_log (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    project_sequence BIGINT NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id UUID NOT NULL,
    aggregate_revision BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    event_schema_version SMALLINT NOT NULL DEFAULT 1,
    actor JSONB NOT NULL DEFAULT '{}'::JSONB,
    command_id UUID,
    correlation_id UUID,
    causation_id UUID,
    payload JSONB NOT NULL DEFAULT '{}'::JSONB,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT event_log_project_sequence_positive CHECK (project_sequence > 0),
    CONSTRAINT event_log_aggregate_type_not_blank CHECK (length(btrim(aggregate_type)) > 0),
    CONSTRAINT event_log_aggregate_revision_non_negative CHECK (aggregate_revision >= 0),
    CONSTRAINT event_log_event_type_not_blank CHECK (length(btrim(event_type)) > 0),
    CONSTRAINT event_log_schema_version_positive CHECK (event_schema_version > 0),
    CONSTRAINT event_log_actor_is_object CHECK (jsonb_typeof(actor) = 'object'),
    CONSTRAINT event_log_payload_is_object CHECK (jsonb_typeof(payload) = 'object'),
    CONSTRAINT event_log_project_sequence_unique UNIQUE (project_id, project_sequence),
    CONSTRAINT event_log_project_id_id_unique UNIQUE (project_id, id)
);

CREATE INDEX event_log_project_occurred_idx
    ON event_log (project_id, occurred_at, project_sequence);

CREATE INDEX event_log_aggregate_idx
    ON event_log (project_id, aggregate_type, aggregate_id, project_sequence);

CREATE TABLE outbox (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    event_id UUID NOT NULL,
    subject TEXT NOT NULL,
    envelope JSONB NOT NULL,
    delivery_state TEXT NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    available_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    lock_owner TEXT,
    lock_expires_at TIMESTAMPTZ,
    last_error_code TEXT,
    last_error_at TIMESTAMPTZ,
    published_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT outbox_subject_not_blank CHECK (length(btrim(subject)) > 0),
    CONSTRAINT outbox_envelope_is_object CHECK (jsonb_typeof(envelope) = 'object'),
    CONSTRAINT outbox_delivery_state_known CHECK (
        delivery_state IN ('pending', 'leased', 'published', 'dead_letter')
    ),
    CONSTRAINT outbox_attempt_count_non_negative CHECK (attempt_count >= 0),
    CONSTRAINT outbox_lock_pair_is_consistent CHECK (
        (lock_owner IS NULL AND lock_expires_at IS NULL)
        OR (lock_owner IS NOT NULL AND lock_expires_at IS NOT NULL)
    ),
    CONSTRAINT outbox_published_at_matches_state CHECK (
        (delivery_state = 'published' AND published_at IS NOT NULL)
        OR (delivery_state <> 'published' AND published_at IS NULL)
    ),
    CONSTRAINT outbox_event_unique UNIQUE (event_id),
    CONSTRAINT outbox_event_same_project_fk
        FOREIGN KEY (project_id, event_id)
        REFERENCES event_log (project_id, id)
        ON DELETE RESTRICT
);

CREATE INDEX outbox_ready_idx
    ON outbox (available_at ASC, created_at ASC, id ASC)
    WHERE delivery_state = 'pending';

CREATE INDEX outbox_project_state_idx
    ON outbox (project_id, delivery_state, available_at);

CREATE TABLE idempotency_keys (
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    idempotency_key TEXT NOT NULL,
    command_id UUID NOT NULL,
    command_name TEXT NOT NULL,
    expected_revision BIGINT,
    request_hash TEXT NOT NULL,
    actor JSONB NOT NULL DEFAULT '{}'::JSONB,
    receipt JSONB NOT NULL,
    event_id UUID,
    response_revision BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ,
    CONSTRAINT idempotency_key_not_blank CHECK (length(btrim(idempotency_key)) > 0),
    CONSTRAINT idempotency_command_name_not_blank CHECK (length(btrim(command_name)) > 0),
    CONSTRAINT idempotency_expected_revision_non_negative CHECK (
        expected_revision IS NULL OR expected_revision >= 0
    ),
    CONSTRAINT idempotency_request_hash_not_blank CHECK (length(btrim(request_hash)) > 0),
    CONSTRAINT idempotency_actor_is_object CHECK (jsonb_typeof(actor) = 'object'),
    CONSTRAINT idempotency_receipt_is_object CHECK (jsonb_typeof(receipt) = 'object'),
    CONSTRAINT idempotency_response_revision_non_negative CHECK (
        response_revision IS NULL OR response_revision >= 0
    ),
    CONSTRAINT idempotency_expiry_after_creation CHECK (
        expires_at IS NULL OR expires_at > created_at
    ),
    CONSTRAINT idempotency_primary_key PRIMARY KEY (project_id, idempotency_key),
    CONSTRAINT idempotency_command_id_unique UNIQUE (command_id),
    CONSTRAINT idempotency_event_same_project_fk
        FOREIGN KEY (project_id, event_id)
        REFERENCES event_log (project_id, id)
        ON DELETE RESTRICT
);

CREATE INDEX idempotency_expiry_idx
    ON idempotency_keys (expires_at)
    WHERE expires_at IS NOT NULL;

CREATE TABLE queue_entries (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    task_id UUID NOT NULL,
    task_sequence BIGINT NOT NULL,
    pipeline_id UUID NOT NULL,
    pipeline_version_id UUID NOT NULL,
    stage_id TEXT NOT NULL,
    queue_kind TEXT NOT NULL DEFAULT 'stage',
    task_revision BIGINT NOT NULL,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    priority_level_id TEXT NOT NULL,
    priority_rank INTEGER NOT NULL,
    resource_profile JSONB NOT NULL DEFAULT '{}'::JSONB,
    queue_state TEXT NOT NULL DEFAULT 'queued',
    eligible_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    last_error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT queue_entries_task_sequence_positive CHECK (task_sequence > 0),
    CONSTRAINT queue_entries_stage_not_blank CHECK (length(btrim(stage_id)) > 0),
    CONSTRAINT queue_entries_kind_known CHECK (queue_kind IN ('stage', 'system')),
    CONSTRAINT queue_entries_task_revision_non_negative CHECK (task_revision >= 0),
    CONSTRAINT queue_entries_attempt_positive CHECK (attempt_number > 0),
    CONSTRAINT queue_entries_priority_level_not_blank CHECK (length(btrim(priority_level_id)) > 0),
    CONSTRAINT queue_entries_priority_rank_non_negative CHECK (priority_rank >= 0),
    CONSTRAINT queue_entries_resource_profile_is_object CHECK (jsonb_typeof(resource_profile) = 'object'),
    CONSTRAINT queue_entries_state_known CHECK (
        queue_state IN ('queued', 'leased', 'cancelled', 'completed', 'dead_letter')
    ),
    CONSTRAINT queue_entries_project_task_unique UNIQUE (project_id, task_id, stage_id, task_revision),
    CONSTRAINT queue_entries_project_id_id_unique UNIQUE (project_id, id),
    CONSTRAINT queue_entries_task_same_project_fk
        FOREIGN KEY (project_id, task_id)
        REFERENCES tasks (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT queue_entries_pipeline_same_project_fk
        FOREIGN KEY (project_id, pipeline_id)
        REFERENCES pipelines (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT queue_entries_pipeline_version_matches_pipeline_fk
        FOREIGN KEY (pipeline_id, pipeline_version_id)
        REFERENCES pipeline_versions (pipeline_id, id)
        ON DELETE RESTRICT
);

CREATE INDEX queue_entries_runnable_order_idx
    ON queue_entries (
        project_id,
        priority_rank DESC,
        eligible_at ASC,
        task_sequence ASC,
        id ASC
    )
    WHERE queue_state = 'queued';

CREATE UNIQUE INDEX queue_entries_one_active_per_task_idx
    ON queue_entries (task_id)
    WHERE queue_state IN ('queued', 'leased');

CREATE TABLE leases (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    task_id UUID NOT NULL,
    queue_entry_id UUID NOT NULL,
    employee_id UUID NOT NULL,
    fencing_token BIGINT NOT NULL DEFAULT nextval('lease_fencing_token_seq'),
    environment_epoch BIGINT NOT NULL DEFAULT 1,
    lease_state TEXT NOT NULL DEFAULT 'active',
    task_work_surface_id UUID,
    lease_scope JSONB NOT NULL DEFAULT '{}'::JSONB,
    resource_reservation JSONB NOT NULL DEFAULT '{}'::JSONB,
    revision BIGINT NOT NULL DEFAULT 0,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    released_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revocation_reason TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT leases_fencing_token_positive CHECK (fencing_token > 0),
    CONSTRAINT leases_environment_epoch_positive CHECK (environment_epoch > 0),
    CONSTRAINT leases_state_known CHECK (lease_state IN ('active', 'released', 'revoked', 'expired')),
    CONSTRAINT leases_scope_is_object CHECK (jsonb_typeof(lease_scope) = 'object'),
    CONSTRAINT leases_resource_reservation_is_object CHECK (jsonb_typeof(resource_reservation) = 'object'),
    CONSTRAINT leases_revision_non_negative CHECK (revision >= 0),
    CONSTRAINT leases_expiry_after_issue CHECK (expires_at > issued_at),
    CONSTRAINT leases_project_queue_unique UNIQUE (project_id, queue_entry_id),
    CONSTRAINT leases_project_id_id_unique UNIQUE (project_id, id),
    CONSTRAINT leases_fencing_token_unique UNIQUE (fencing_token),
    CONSTRAINT leases_task_same_project_fk
        FOREIGN KEY (project_id, task_id)
        REFERENCES tasks (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT leases_queue_same_project_fk
        FOREIGN KEY (project_id, queue_entry_id)
        REFERENCES queue_entries (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT leases_employee_same_project_fk
        FOREIGN KEY (project_id, employee_id)
        REFERENCES employees (project_id, id)
        ON DELETE RESTRICT
);

ALTER SEQUENCE lease_fencing_token_seq OWNED BY leases.fencing_token;

CREATE UNIQUE INDEX leases_active_task_idx
    ON leases (task_id)
    WHERE lease_state = 'active';

CREATE UNIQUE INDEX leases_active_work_surface_idx
    ON leases (task_work_surface_id)
    WHERE lease_state = 'active' AND task_work_surface_id IS NOT NULL;

CREATE INDEX leases_project_state_expiry_idx
    ON leases (project_id, lease_state, expires_at);

CREATE TABLE runs (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    task_id UUID NOT NULL,
    queue_entry_id UUID NOT NULL,
    lease_id UUID NOT NULL,
    employee_id UUID NOT NULL,
    stage_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    lease_fencing_token BIGINT NOT NULL,
    environment_epoch BIGINT NOT NULL DEFAULT 1,
    last_sequence BIGINT NOT NULL DEFAULT 0,
    desired_state TEXT NOT NULL DEFAULT 'provision_requested',
    observed_state TEXT NOT NULL DEFAULT 'unknown',
    run_spec_version SMALLINT NOT NULL DEFAULT 1,
    run_spec JSONB NOT NULL,
    context_manifest JSONB NOT NULL DEFAULT '{}'::JSONB,
    observed_details JSONB NOT NULL DEFAULT '{}'::JSONB,
    error_code TEXT,
    revision BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT runs_stage_not_blank CHECK (length(btrim(stage_id)) > 0),
    CONSTRAINT runs_attempt_positive CHECK (attempt_number > 0),
    CONSTRAINT runs_fencing_token_positive CHECK (lease_fencing_token > 0),
    CONSTRAINT runs_environment_epoch_positive CHECK (environment_epoch > 0),
    CONSTRAINT runs_last_sequence_non_negative CHECK (last_sequence >= 0),
    CONSTRAINT runs_desired_state_known CHECK (
        desired_state IN (
            'provision_requested',
            'running',
            'stop_requested',
            'force_stop_requested',
            'stopped',
            'failed'
        )
    ),
    CONSTRAINT runs_observed_state_known CHECK (
        observed_state IN (
            'unknown',
            'provisioning',
            'running',
            'stopping',
            'stopped',
            'failed',
            'lost'
        )
    ),
    CONSTRAINT runs_spec_version_positive CHECK (run_spec_version > 0),
    CONSTRAINT runs_spec_is_object CHECK (jsonb_typeof(run_spec) = 'object'),
    CONSTRAINT runs_context_manifest_is_object CHECK (jsonb_typeof(context_manifest) = 'object'),
    CONSTRAINT runs_observed_details_is_object CHECK (jsonb_typeof(observed_details) = 'object'),
    CONSTRAINT runs_revision_non_negative CHECK (revision >= 0),
    CONSTRAINT runs_finished_after_start CHECK (
        finished_at IS NULL OR started_at IS NULL OR finished_at >= started_at
    ),
    CONSTRAINT runs_queue_entry_unique UNIQUE (queue_entry_id),
    CONSTRAINT runs_project_id_id_unique UNIQUE (project_id, id),
    CONSTRAINT runs_task_stage_attempt_unique UNIQUE (task_id, stage_id, attempt_number),
    CONSTRAINT runs_task_same_project_fk
        FOREIGN KEY (project_id, task_id)
        REFERENCES tasks (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT runs_queue_same_project_fk
        FOREIGN KEY (project_id, queue_entry_id)
        REFERENCES queue_entries (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT runs_lease_same_project_fk
        FOREIGN KEY (project_id, lease_id)
        REFERENCES leases (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT runs_employee_same_project_fk
        FOREIGN KEY (project_id, employee_id)
        REFERENCES employees (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT runs_lease_fencing_token_fk
        FOREIGN KEY (lease_fencing_token)
        REFERENCES leases (fencing_token)
        ON DELETE RESTRICT
);

CREATE INDEX runs_project_task_created_idx
    ON runs (project_id, task_id, created_at DESC);

CREATE INDEX runs_observed_state_idx
    ON runs (project_id, observed_state, updated_at);

CREATE TABLE artifacts (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    task_id UUID,
    run_id UUID,
    stage_id TEXT,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    producer_type TEXT NOT NULL,
    producer_id UUID,
    producer JSONB NOT NULL DEFAULT '{}'::JSONB,
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    body_storage TEXT NOT NULL DEFAULT 'inline_json',
    body_json JSONB,
    object_ref JSONB,
    content_sha256 TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT artifacts_stage_not_blank CHECK (
        stage_id IS NULL OR length(btrim(stage_id)) > 0
    ),
    CONSTRAINT artifacts_kind_not_blank CHECK (length(btrim(kind)) > 0),
    CONSTRAINT artifacts_title_not_blank CHECK (length(btrim(title)) > 0),
    CONSTRAINT artifacts_producer_type_not_blank CHECK (length(btrim(producer_type)) > 0),
    CONSTRAINT artifacts_producer_is_object CHECK (jsonb_typeof(producer) = 'object'),
    CONSTRAINT artifacts_metadata_is_object CHECK (jsonb_typeof(metadata) = 'object'),
    CONSTRAINT artifacts_storage_known CHECK (body_storage IN ('inline_json', 'object_reference')),
    CONSTRAINT artifacts_body_matches_storage CHECK (
        (body_storage = 'inline_json' AND body_json IS NOT NULL AND object_ref IS NULL)
        OR (body_storage = 'object_reference' AND body_json IS NULL AND object_ref IS NOT NULL)
    ),
    CONSTRAINT artifacts_inline_body_bounded CHECK (
        body_json IS NULL OR octet_length(body_json::TEXT) <= 262144
    ),
    CONSTRAINT artifacts_object_ref_is_object CHECK (
        object_ref IS NULL OR jsonb_typeof(object_ref) = 'object'
    ),
    CONSTRAINT artifacts_content_hash_not_blank CHECK (
        content_sha256 IS NULL OR length(btrim(content_sha256)) > 0
    ),
    CONSTRAINT artifacts_task_same_project_fk
        FOREIGN KEY (project_id, task_id)
        REFERENCES tasks (project_id, id)
        ON DELETE RESTRICT,
    CONSTRAINT artifacts_run_same_project_fk
        FOREIGN KEY (project_id, run_id)
        REFERENCES runs (project_id, id)
        ON DELETE RESTRICT
);

CREATE INDEX artifacts_project_task_created_idx
    ON artifacts (project_id, task_id, created_at)
    WHERE task_id IS NOT NULL;

CREATE INDEX artifacts_project_run_created_idx
    ON artifacts (project_id, run_id, created_at)
    WHERE run_id IS NOT NULL;

CREATE TABLE consumer_dedupe (
    consumer_name TEXT NOT NULL,
    message_id UUID NOT NULL,
    source_event_id UUID,
    receipt JSONB NOT NULL DEFAULT '{}'::JSONB,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ,
    CONSTRAINT consumer_dedupe_name_not_blank CHECK (length(btrim(consumer_name)) > 0),
    CONSTRAINT consumer_dedupe_receipt_is_object CHECK (jsonb_typeof(receipt) = 'object'),
    CONSTRAINT consumer_dedupe_expiry_after_process CHECK (
        expires_at IS NULL OR expires_at > processed_at
    ),
    CONSTRAINT consumer_dedupe_primary_key PRIMARY KEY (consumer_name, message_id),
    CONSTRAINT consumer_dedupe_source_event_fk
        FOREIGN KEY (source_event_id)
        REFERENCES event_log (id)
        ON DELETE RESTRICT
);

CREATE INDEX consumer_dedupe_expiry_idx
    ON consumer_dedupe (expires_at)
    WHERE expires_at IS NOT NULL;

CREATE TRIGGER projects_set_updated_at
BEFORE UPDATE ON projects
FOR EACH ROW
EXECUTE FUNCTION forge_set_updated_at();

CREATE TRIGGER employees_set_updated_at
BEFORE UPDATE ON employees
FOR EACH ROW
EXECUTE FUNCTION forge_set_updated_at();

CREATE TRIGGER pipelines_set_updated_at
BEFORE UPDATE ON pipelines
FOR EACH ROW
EXECUTE FUNCTION forge_set_updated_at();

CREATE TRIGGER tasks_set_updated_at
BEFORE UPDATE ON tasks
FOR EACH ROW
EXECUTE FUNCTION forge_set_updated_at();

CREATE TRIGGER outbox_set_updated_at
BEFORE UPDATE ON outbox
FOR EACH ROW
EXECUTE FUNCTION forge_set_updated_at();

CREATE TRIGGER queue_entries_set_updated_at
BEFORE UPDATE ON queue_entries
FOR EACH ROW
EXECUTE FUNCTION forge_set_updated_at();

CREATE TRIGGER leases_set_updated_at
BEFORE UPDATE ON leases
FOR EACH ROW
EXECUTE FUNCTION forge_set_updated_at();

CREATE TRIGGER leases_identity_immutable
BEFORE UPDATE ON leases
FOR EACH ROW
EXECUTE FUNCTION forge_guard_lease_identity();

CREATE TRIGGER runs_set_updated_at
BEFORE UPDATE ON runs
FOR EACH ROW
EXECUTE FUNCTION forge_set_updated_at();

CREATE TRIGGER runs_identity_immutable
BEFORE UPDATE ON runs
FOR EACH ROW
EXECUTE FUNCTION forge_guard_run_identity();

CREATE TRIGGER pipeline_versions_append_only
BEFORE UPDATE OR DELETE ON pipeline_versions
FOR EACH ROW
EXECUTE FUNCTION forge_reject_append_only_write();

CREATE TRIGGER event_log_append_only
BEFORE UPDATE OR DELETE ON event_log
FOR EACH ROW
EXECUTE FUNCTION forge_reject_append_only_write();

CREATE TRIGGER artifacts_append_only
BEFORE UPDATE OR DELETE ON artifacts
FOR EACH ROW
EXECUTE FUNCTION forge_reject_append_only_write();

CREATE TRIGGER pipelines_no_physical_delete
BEFORE DELETE ON pipelines
FOR EACH ROW
EXECUTE FUNCTION forge_reject_pipeline_delete();
