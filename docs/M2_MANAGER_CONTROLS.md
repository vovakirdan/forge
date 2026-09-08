# M2: named execution management

Status: individual stop/pause commands, one-shot Employee constraints and durable scheduled
resume implemented. No LLM manager or resolver runtime
is added by these commands.

Every command uses the common Project revision and idempotency envelope. Trusted command
authority determines the actor; payload JSON cannot grant management capabilities.

## Stop an Employee

`stop_employee` accepts `employee_id`, `expected_employee_revision`, explicit
`mode: graceful | force` and optional nonblank `reason`.

It disables future admission and stops every existing execution owned by this Employee,
regardless of execution purpose. Already retired Employees remain retired. This is distinct
from `disable_employee`, which still leaves existing executions alone. `enable_employee`
is an explicit later command, never an automatic consequence of a stopped Run.

Only a Run owning the Task's exact current stage visit can add its Employee-stop wait.
A communication Run's Task context grants no Task ownership. A finishing Run from a prior
stage or prior visit cannot pause the new stage, even when both stages have the same name.
Historical Runs without a frozen stage visit are stopped but do not infer a current Task
pause; use explicit `pause_task` when that management action is required.

## Pause one Task

`pause_task` accepts `task_id`, `expected_task_revision`, explicit `mode` and optional
nonblank `reason`. It stops this Task's stage executions, invalidates queued snapshots and
adds a `manual_pause` wait. Draft and terminal Tasks reject the operation. A repeated pause
does not duplicate the same pause condition. Other active wait conditions remain intact.

Task `waiting` records accepted management intent immediately. It does not claim physical
shutdown; Run observed state and reservation still report actual execution. A ready Task
resumes to `ready`, an already started Task to `in_progress`, only after its last wait clears.
The existing `resume_task` resolves one explicitly named wait and queues eligible work.
Project and physical reservation gates still control admission.

## Stop is not completion

Graceful stop requests bounded shutdown. Force stop also revokes the exact logical Lease.
Neither releases physical reservations, deletes workspace/history, invents accepted evidence,
cancels the Task, nor silently retries it. A stopped or revoked Run cannot submit a late
outcome. Retained physical executions remain visible to stop replay and reconciliation.

Employee state, Task wait, fenced Run intent, immutable Events, outbox and receipt commit
atomically. Transport delivery follows commit and is retryable. Old Project-stop requests
retain their existing graceful default; individual commands require an explicit mode.

Optional `reason` is at most 10,000 Unicode characters and contains no NUL. It is retained
in the canonical management Event, not interpreted as a status, shell command or prompt.

## Select the Employee for the next Run

`set_next_run_employee` accepts `task_id`, `expected_task_revision`, `employee_id` and
optional `reason`. The target must be enabled, in the same Project, and eligible for the
Task's current Employee stage. `clear_next_run_employee` accepts the same Task revision
and optional reason, without an Employee ID. Both require common management authority.

This is a one-shot admission constraint, not a transfer of the current Run. Its immutable
scope pins Task, PipelineVersion, stage and stage visit. It can be set while work is running
or waiting without stopping work, changing Task state or resolving any wait. It is consumed
only in the transaction issuing the next matching Lease. Failed provisioning after that
commit does not restore the consumed instruction or choose a different Employee implicitly.

| Result | Meaning |
| --- | --- |
| `pending` | Wait for the specified Employee's capacity; no fallback. Other Tasks remain schedulable. |
| `blocked` | Employee was disabled or lost stage eligibility before dispatch. Explicit replacement or clearing is required. |
| `consumed` | Exactly one matching Run was admitted; its ID is retained. |
| `cancelled` | Explicitly cleared/replaced, or found to belong to an earlier stage visit. History remains. |

An admission hold does not itself change the Task lifecycle or interrupt another active
Run. Enabling an Employee later does not silently clear a recorded hold. Old-visit intent
never applies to a new visit with the same stage name; dispatch retires it when encountered.
If no further dispatch occurs, the historical record remains visible but inapplicable and
can be cleared explicitly, including after Task closure. Replacement and clearing preserve
the old immutable scope and append canonical Events. Task revision guards the chosen scope;
Project revision and constraint-state CAS serialize changes without inventing Task edits.

## Schedule one explicit resume

`schedule_task_resume` accepts `task_id`, `expected_task_revision`, `wait_condition_id`,
future RFC3339 `not_before` and required nonblank `reason`. It returns a
`task_resume_schedule` identity. `cancel_task_resume` accepts `schedule_id` and optional
reason. One pending alarm is allowed per Task/wait; completed records are retained.

The alarm pins Task revision, PipelineVersion, stage and exact visit. Its wait must be
resolvable by ordinary `resume_task` on the current Employee stage: scheduling uses a
dry-run of that same domain operation. It cannot clear a stage-owned wait, undo exhausted
pipeline retries, or bypass a recovery assessment. Other waits remain active. Scheduling
does not itself change the Task or stop any Run; use the named stop/pause actions first.

At the deadline, Core locks Project, alarm and Task, rechecks scope and normal resume
eligibility, then checks Project execution, boot recovery hold, dependencies, logical
execution and retained physical/leased-proposal ownership. A failed precondition commits
`rejected` with a stable reason and leaves the Task untouched. It never silently retries;
the operator can create a fresh alarm after resolving the cause. A concurrent cancellation
or Task mutation is serialized by the Project gate.

If allowed, the ordinary `resume_task` command executes in the same transaction as the
alarm's `applied` result. Audit identifies the trusted SystemManager performing the action
and retains the original issuer and reason in the alarm. The resulting queue still obeys
capacity, Employee assignment, surface and provider gates; a timer is not a Run permission.

The watchdog scans at most 64 due records per pass. PostgreSQL is the durable wake source;
outbox notification is not required, and reconstructing Core needs no in-memory timer
registration. Storage failures roll back and remain pending. Logical rejections are terminal.
For real runtimes, unreconciled Supervisor inventory defers alarms until boot policy and
physical ownership are known; it does not consume the alarm as a guessed rejection.
Polling delay, Core downtime and queue capacity may delay work beyond `not_before`.
Deadline eligibility uses operational wall time; canonical Event times use the Project's
causal clock. Canonical history retains full timestamp precision while PostgreSQL scans
at microsecond precision and the locked intent rechecks the precise deadline.
