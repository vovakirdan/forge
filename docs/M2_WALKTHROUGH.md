# M2 local acceptance and CLI walkthrough

Use a disposable project and a separate repository. These are developer
acceptance tools, not an installer or an automatic test runner for owner projects.
No command below discovers credentials or starts paid inference implicitly.

For the real end-to-end operator exercise in a separate empty repository, use
the [M2 launcher and Russian scenario](M2_OPERATOR_SCENARIO.md). It asks for
explicit provider/auth approval and drives work, review, QA and integration.
The keyless checks below remain separate proof.

## Keyless checks

With the existing local PostgreSQL/NATS development services prepared:

```bash
just test-m2
```

This runs real Git fixture checks, CLI template tests and the selected M2
canonical/transport scenarios. Synthetic Supervisor and provider observations
prove those boundaries, not live LLM behavior. Keep the full `just check`,
`just test-unit` and `just test-integration` regression gates separate.

## Fresh repository for manual work

```bash
just prepare-m2-repository
```

The output is a JSON manifest. It names a newly created private directory under
`/tmp/forge-m2-repository.*`, a bare `source.git`, `refs/heads/main` and an exact
initial commit. The manifest is also saved as `fixture.json` inside that
directory. Nothing is registered with Core yet. No existing target path is
accepted and no repository, worktree or log is deleted afterward.

The toy project has two files: a README and an executable line normalizer with
an intentional duplicate-removal omission. It needs only a POSIX shell and
`sort`. The bare target avoids a checked-out integration branch; it has no remote.
The owner may keep the fixture elsewhere later, but must explicitly register
the resulting source instead of relying on Forge's working directory.

## Named-command sequence

All mutations use the same CLI boundary:

```bash
target/debug/forge-cli --socket "$M2_SOCKET" command COMMAND_NAME \
  --project-id "$M2_PROJECT_ID" --expected-revision "$M2_PROJECT_REVISION" \
  --idempotency-key "$M2_COMMAND_ID" --payload "$M2_PAYLOAD"
```

Read the current Project revision before a new command. Reuse an idempotency
key only for an identical retry; a transport timeout is not proof of rollback.
The sequence is:

1. Create a new stopped Project and its immutable Pipeline. Configure the stages
   you actually want: work, optional independent review, report-only QA or
   writable test development, then optional local Integration. There is no
   implicit `run all tests` step.
2. Create Employees, explicitly enroll each credential from an owner-only file,
   and configure its runtime using a [four-lane profile template](M2_PROVIDER_PROFILES.md).
   Keep the Project stopped while reviewing configuration.
3. Call `register_project_repository` with the manifest's `registration` object.
   Keep the returned repository ID. Create two draft Tasks: normalize duplicate
   lines and improve the README. Each Task has its own ID and surface.
4. For each draft call `bind_task_git_repository` with its current revision, the
   returned repository ID and manifest `initial_base`, then `approve_task`.
   [Binding payloads](M2_GIT_BINDING.md) show the exact fields.
5. For a concurrency exercise, explicitly amend one eligible Employee's capacity
   to three. Start the Project only after approving the possible provider cost.
   Two Task Runs may then overlap. Open a general Employee thread and send an
   `inbox` question to exercise the separate third Communication Run.
6. Send a required instruction to an exact Task visit or fenced Run to exercise
   [Inbox receipts](M2_INBOX.md). A general question never grants Task ownership.
   Inspect the same Task through review/rework; do not create a new Task just to
   move it to a different stage.
7. Read candidate review records, artifacts, integration history and Run
   diagnostics. An Employee must commit explicitly. A dirty worktree, an old
   review or an unresolved required instruction cannot be accepted as a clean
   final candidate. Stale target handling returns the same Task to configured
   rework rather than silently merging after review.
8. Use `stop_project_execution` to stop the exercise. Wait for positive physical
   quiescence; a Task's `waiting` status alone is not proof that its process has
   stopped. Keep the private session, repository and evidence for inspection.

For independent review, use another eligible Employee identity: the same
Employee cannot review any candidate to which it contributed. Capacity three
does not grant self-review permission or merge three Task surfaces.
The default shared host/Project/account limits allow this three-Run exercise;
an operator-selected lower [admission limit](M2_ADMISSION_LIMITS.md) can keep a
Run queued even when the Employee has free capacity.

## Useful reads

Use `forge-cli --socket "$M2_SOCKET" get PATH` with the actual IDs:

- `/v1/projects/PROJECT/tasks`
- `/v1/projects/PROJECT/tasks/TASK`
- `/v1/projects/PROJECT/runs`
- `/v1/projects/PROJECT/runs/RUN`
- `/v1/projects/PROJECT/tasks/TASK/reviews`
- `/v1/projects/PROJECT/tasks/TASK/integrations`
- `/v1/projects/PROJECT/employees/EMPLOYEE/threads`
- `/v1/projects/PROJECT/threads/THREAD/messages`
- `/v1/projects/PROJECT/findings`

The list endpoints are scoped and paginated; use returned cursors. A Finding is
not a new Task until an authorized manager explicitly promotes it. Message
runtime acceptance is not Employee acknowledgement, and acknowledgement is not
an answer.
