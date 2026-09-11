-- Semantic processing is durable work with its own owner, not an Employee alias.
CREATE TABLE project_system_job_settings (
 project_id UUID PRIMARY KEY REFERENCES projects(id),
 revision BIGINT NOT NULL CHECK(revision>0),
 policy JSONB NOT NULL CHECK(jsonb_typeof(policy)='object'),
 binding JSONB NOT NULL CHECK(jsonb_typeof(binding)='object'),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE system_jobs (
 id UUID PRIMARY KEY,
 project_id UUID NOT NULL REFERENCES projects(id),
 kind TEXT NOT NULL CHECK(kind IN ('summarization','onboarding')),
 source_task_id UUID,
 target_employee_id UUID,
 generation BIGINT NOT NULL DEFAULT 1 CHECK(generation>0),
 covered_sequence BIGINT NOT NULL DEFAULT 0 CHECK(covered_sequence>=0),
 state TEXT NOT NULL CHECK(state IN ('pending','running','completed','held','cancelled')),
 input JSONB NOT NULL CHECK(jsonb_typeof(input)='object'),
 eligible_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 reason_code TEXT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(project_id,id),
 FOREIGN KEY(project_id,source_task_id) REFERENCES tasks(project_id,id),
 FOREIGN KEY(project_id,target_employee_id) REFERENCES employees(project_id,id),
 CHECK((kind='summarization' AND source_task_id IS NOT NULL)
    OR (kind='onboarding' AND source_task_id IS NULL AND target_employee_id IS NOT NULL))
);
CREATE TABLE system_job_event_cursors (
 project_id UUID PRIMARY KEY REFERENCES projects(id),
 last_sequence BIGINT NOT NULL CHECK(last_sequence>=0)
);
CREATE UNIQUE INDEX system_job_task_identity ON system_jobs(project_id,source_task_id) WHERE kind='summarization';
CREATE UNIQUE INDEX system_job_onboarding_identity ON system_jobs(project_id,target_employee_id) WHERE kind='onboarding';
CREATE INDEX system_jobs_pending ON system_jobs(project_id,eligible_at,id) WHERE state='pending';
CREATE TABLE system_job_memory_heads (
 job_id UUID NOT NULL REFERENCES system_jobs(id),
 subject_key TEXT NOT NULL CHECK(length(subject_key)<=2048),
 entry_id UUID NOT NULL UNIQUE,
 revision BIGINT NOT NULL CHECK(revision>0),
 PRIMARY KEY(job_id,subject_key),
 FOREIGN KEY(entry_id) REFERENCES derived_memory_entries(id) DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE system_job_attempts (
 id UUID PRIMARY KEY,
 project_id UUID NOT NULL,
 job_id UUID NOT NULL,
 generation BIGINT NOT NULL CHECK(generation>0),
 run_id UUID NOT NULL UNIQUE,
 core_instance UUID NOT NULL,
 state TEXT NOT NULL DEFAULT 'running' CHECK(state IN ('running','completed','stopped','failed','superseded')),
 spec JSONB NOT NULL,
 result JSONB,
 result_message_id UUID UNIQUE,
 result_hash TEXT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 completed_at TIMESTAMPTZ,
 UNIQUE(project_id,id),
 FOREIGN KEY(project_id,job_id) REFERENCES system_jobs(project_id,id),
 FOREIGN KEY(project_id,run_id) REFERENCES runs(project_id,id) DEFERRABLE INITIALLY DEFERRED,
 CHECK((spec->>'schema_version'='7' AND spec->>'project_id'=project_id::text
   AND spec->>'run_id'=run_id::text AND spec->'assignment'->>'job_id'=job_id::text
   AND spec->'assignment'->>'attempt_id'=id::text
   AND (spec->'assignment'->>'generation')::bigint=generation) IS TRUE),
 CHECK((result IS NULL)=(result_message_id IS NULL)),
 CHECK((result IS NULL)=(result_hash IS NULL))
);
CREATE UNIQUE INDEX system_job_one_running_attempt ON system_job_attempts(job_id) WHERE state='running';
CREATE INDEX system_job_attempt_quota ON system_job_attempts(project_id,created_at);

CREATE TABLE employee_onboarding (
 project_id UUID NOT NULL,
 employee_id UUID PRIMARY KEY,
 state TEXT NOT NULL CHECK(state IN ('pending','completed','skipped','legacy_bypass')),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
 job_id UUID,
 receipt JSONB,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(project_id,employee_id) REFERENCES employees(project_id,id),
 FOREIGN KEY(project_id,job_id) REFERENCES system_jobs(project_id,id)
);
INSERT INTO employee_onboarding(project_id,employee_id,state)
 SELECT project_id,id,'legacy_bypass' FROM employees;
CREATE FUNCTION forge_create_employee_onboarding() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 INSERT INTO employee_onboarding(project_id,employee_id,state) VALUES(NEW.project_id,NEW.id,'pending');
 RETURN NEW;
END $$;
CREATE TRIGGER employee_onboarding_initial AFTER INSERT ON employees FOR EACH ROW EXECUTE FUNCTION forge_create_employee_onboarding();

ALTER TABLE leases ADD COLUMN system_job_attempt_id UUID;
ALTER TABLE runs ADD COLUMN system_job_attempt_id UUID;
ALTER TABLE run_environment_reservations ADD COLUMN system_job_attempt_id UUID;
ALTER TABLE leases DROP CONSTRAINT leases_employee_purpose_exact;
ALTER TABLE runs DROP CONSTRAINT runs_employee_purpose_exact;
ALTER TABLE run_environment_reservations DROP CONSTRAINT environment_employee_purpose_exact;
ALTER TABLE leases ADD CONSTRAINT leases_employee_purpose_exact CHECK ((purpose IN ('hook','system_job'))=(employee_id IS NULL));
ALTER TABLE runs ADD CONSTRAINT runs_employee_purpose_exact CHECK ((purpose IN ('hook','system_job'))=(employee_id IS NULL));
ALTER TABLE run_environment_reservations ADD CONSTRAINT environment_employee_purpose_exact CHECK ((purpose IN ('hook','system_job'))=(employee_id IS NULL));

-- Preserve all prior purpose predicates, including V6 Task source/snapshot rules.
DO $$ DECLARE item record; prior_expr text; BEGIN
 FOR item IN SELECT * FROM (VALUES
 ('leases','leases_owner_exact','task_id IS NULL AND queue_entry_id IS NULL AND task_work_surface_id IS NULL'),
 ('runs','runs_owner_exact','task_id IS NULL AND queue_entry_id IS NULL AND stage_id IS NULL AND run_spec_version=7'),
 ('run_environment_reservations','environment_owner_exact','task_id IS NULL AND surface_id IS NULL')) AS x(tbl,cname,extra)
 LOOP
  SELECT pg_get_expr(conbin,conrelid) INTO STRICT prior_expr FROM pg_constraint
   WHERE conrelid=item.tbl::regclass AND conname=item.cname;
  EXECUTE format('ALTER TABLE %I DROP CONSTRAINT %I',item.tbl,item.cname);
  EXECUTE format('ALTER TABLE %I ADD CONSTRAINT %I CHECK ((system_job_attempt_id IS NULL AND (%s)) OR
    (purpose=''system_job'' AND system_job_attempt_id IS NOT NULL AND employee_id IS NULL
     AND communication_assignment_id IS NULL AND resolution_assignment_id IS NULL AND hook_invocation_id IS NULL AND %s))',
     item.tbl,item.cname,prior_expr,item.extra);
 END LOOP;
END $$;
ALTER TABLE leases ADD FOREIGN KEY(project_id,system_job_attempt_id) REFERENCES system_job_attempts(project_id,id);
ALTER TABLE leases ADD CONSTRAINT leases_system_job_scope UNIQUE(project_id,id,system_job_attempt_id);
ALTER TABLE runs ADD FOREIGN KEY(project_id,lease_id,system_job_attempt_id) REFERENCES leases(project_id,id,system_job_attempt_id);
ALTER TABLE runs ADD CONSTRAINT runs_system_job_scope UNIQUE(project_id,id,system_job_attempt_id);
CREATE UNIQUE INDEX system_job_attempt_run ON runs(system_job_attempt_id) WHERE purpose='system_job';
ALTER TABLE run_environment_reservations ADD FOREIGN KEY(project_id,run_id,system_job_attempt_id) REFERENCES runs(project_id,id,system_job_attempt_id);
CREATE UNIQUE INDEX system_job_physical_attempt ON run_environment_reservations(system_job_attempt_id) WHERE released_at IS NULL AND purpose='system_job';

CREATE OR REPLACE FUNCTION forge_lease_owns_run(l leases,r runs) RETURNS BOOLEAN LANGUAGE SQL IMMUTABLE AS $$
 SELECT l.id=r.lease_id AND l.project_id=r.project_id AND l.employee_id IS NOT DISTINCT FROM r.employee_id
  AND l.fencing_token=r.lease_fencing_token AND l.environment_epoch=r.environment_epoch
  AND l.purpose=r.purpose AND CASE r.purpose
   WHEN 'task_stage' THEN l.task_id=r.task_id AND l.queue_entry_id=r.queue_entry_id
   WHEN 'communication' THEN l.communication_assignment_id=r.communication_assignment_id
   WHEN 'resolution' THEN l.resolution_assignment_id=r.resolution_assignment_id
   WHEN 'hook' THEN l.hook_invocation_id=r.hook_invocation_id
   WHEN 'system_job' THEN l.system_job_attempt_id=r.system_job_attempt_id
   ELSE FALSE END;
$$;
CREATE OR REPLACE FUNCTION forge_guard_assignment_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (OLD.purpose,OLD.communication_assignment_id,OLD.resolution_assignment_id,OLD.hook_invocation_id,OLD.system_job_attempt_id)
  IS DISTINCT FROM (NEW.purpose,NEW.communication_assignment_id,NEW.resolution_assignment_id,NEW.hook_invocation_id,NEW.system_job_attempt_id) THEN
  RAISE EXCEPTION 'execution assignment is immutable';
 END IF;
 RETURN NEW;
END $$;
CREATE FUNCTION forge_guard_system_job_run() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.purpose='system_job' AND NOT EXISTS(SELECT 1 FROM system_job_attempts a
   WHERE a.id=NEW.system_job_attempt_id AND a.project_id=NEW.project_id AND a.run_id=NEW.id AND a.spec=NEW.run_spec) THEN
  RAISE EXCEPTION 'SystemJob RunSpec differs from its canonical attempt';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER system_job_run_guard BEFORE INSERT OR UPDATE ON runs FOR EACH ROW EXECUTE FUNCTION forge_guard_system_job_run();
CREATE FUNCTION forge_guard_system_job_attempt() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'SystemJob attempt history is retained'; END IF;
 IF (OLD.id,OLD.project_id,OLD.job_id,OLD.generation,OLD.run_id,OLD.core_instance,OLD.spec,OLD.created_at)
  IS DISTINCT FROM (NEW.id,NEW.project_id,NEW.job_id,NEW.generation,NEW.run_id,NEW.core_instance,NEW.spec,NEW.created_at)
  OR (OLD.result IS NOT NULL AND (NEW.result,NEW.result_hash,NEW.result_message_id) IS DISTINCT FROM (OLD.result,OLD.result_hash,OLD.result_message_id))
  OR (OLD.state<>'running' AND NEW.state<>OLD.state) THEN
  RAISE EXCEPTION 'SystemJob identity and completed result are immutable';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER system_job_attempt_guard BEFORE UPDATE OR DELETE ON system_job_attempts FOR EACH ROW EXECUTE FUNCTION forge_guard_system_job_attempt();
