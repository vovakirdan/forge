# Findings in M2

A Finding records an observation from a Task. Reporting it does not create,
approve, queue or change a Task. Its description, project-defined severity,
evidence references, author and source are immutable. Evidence must belong to
the source Task in the same Project; Forge does not verify its semantic truth.

`finding.report` / `forge_report_finding` is a scoped Task Gateway tool. Core
derives the source Task, Run, Employee and fence. Employees cannot submit
management fields or promote/triage Findings through this tool. Identical
`message_id` retries return the original receipt.

Local management commands:

- `report_finding`: report against an existing Task, with authenticated author.
- `triage_finding`: attach to an existing same-Project Task or ignore with a reason.
- `promote_finding`: select a Pipeline and draft Task intent explicitly; create
  that Task with source `promoted_finding` and retain its link in the Finding.

Triage requires `expected_finding_revision` in addition to the command envelope's
Project revision. An open or attached Finding can be triaged; promoted and
ignored results are final in M2. The original report remains unchanged and every
triage decision is retained in the event log. Attach creates a relation, not an
Artifact copy or additional execution assignment.

Promotion uses the ordinary draft-creation rules, including Pipeline default
selection, soft-deletion checks, kind compatibility and Project priority. Task
creation, report triage, events, outbox and command receipt commit together.
Replaying the command cannot create another Task. Promotion does not imply
approval, a repository binding, or permission to run.

The local Human control boundary grants management commands. Trusted embedding
code may delegate individual named capabilities; ordinary Employee Gateway
grants do not include triage or promotion. No autonomous Lead is introduced.

`GET /v1/projects/{project_id}/findings` reads ascending ID pages. Optional
`task_id` filters the source Task; `after` is a UUIDv7 cursor and `limit` is
1..100 (default 50). Reads never triage. `reported_at` and triage `at` use RFC3339;
ID ordering does not replace authoritative event time.

The schema retains Finding history; there is no delete or automatic backlog
promotion operation. M2 currently reports from Task execution, not from taskless
Communication or Resolution Runs.
