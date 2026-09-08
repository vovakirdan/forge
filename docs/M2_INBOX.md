# M2: Employee Inbox and Task instructions

This document describes canonical conversations and Task-directed Gateway tools.
Taskless processing is described in [Communication Runs](M2_COMMUNICATION_RUNS.md),
and transport receipts in [native input delivery](M2_NATIVE_INPUT.md). A persisted
general question does not start a Task or interrupt an arbitrary Task Run.

## Operator commands

All commands use the existing `forge command` transport, Project revision,
trusted actor and idempotency key. Payloads are defined in `openapi/v1.yaml`.

- `open_employee_thread`: Employee plus optional Task context; new revision is 1.
- `send_employee_message`: expected thread revision, target, kind, requirement,
  body and optional reply reference. Each accepted message advances sequence
  and thread revision once. Content is immutable and bounded to 32 KiB UTF-8.
- `waive_message_requirement`: explicitly remove a requirement with a reason;
  stores a separate immutable operator decision, not an Employee receipt.

Targets are distinct:

| Target | Meaning |
| --- | --- |
| `inbox` | Employee conversation for a separate Communication assignment. |
| `task_execution` | Employee plus exact Task, PipelineVersion, stage and visit. |
| `exact_run` | The same Task context plus Run, fencing token and environment epoch. |

A mention is text, not an assignment. Task context grants no capability.
Cancelled/done Tasks reject new execution-directed instructions. Existing
messages and threads remain retained. Disabled Employees can receive messages;
retired Employees reject new conversations and messages.

Execution-directed targets must name an Employee-executed stage. A conversation
about a Human/External stage can use `inbox` with optional Task thread context.

An `exact_run` requirement stays bound to its original fence after a restart or
stop; a replacement cannot acknowledge it. Management sends any replacement
instruction and explicitly waives the obsolete requirement, with a retained
reason. This also handles a retired recipient without pretending they answered.

Reads are scoped to Project and bounded to 1–100 items:

- `GET /v1/projects/{project}/employees/{employee}/threads?after={uuid}&limit=50`
- `GET /v1/projects/{project}/threads/{thread}/messages?after={sequence}&limit=50`

Responses contain `items` and `next_cursor`; pass the cursor as `after`. Thread
ordering uses immutable creation UUIDs, not mutable activity timestamps.

## Task Run tools

New Run contexts pin stage visit and `forge_task_tools_v2`. Historical contexts
without a visit remain valid for M0–M1, but cannot gain Inbox authority.

| Logical tool | MCP tool | Effect |
| --- | --- | --- |
| `inbox.list` | `forge_list_instructions` | Read messages for this exact Task visit. No ACK is recorded. |
| `inbox.acknowledge` | `forge_acknowledge_instruction` | Record the Employee's explicit acknowledgement. |
| `inbox.reply` | `forge_reply_instruction` | Append an attributed canonical reply and record an answer. |

Mutations require a stable UUIDv7 `message_id` for retries and a
`target_message_id`; reply additionally requires `body`. Identity, Project,
Employee, Task and fence come from the authenticated Gateway, never arguments.
General Inbox messages are excluded from Task Run reads and receipts.

Persistence, runtime acceptance, Employee acknowledgement and Employee answer
are different facts. Runtime acceptance alone satisfies neither acknowledgement
nor answer. Reading alone satisfies nothing. An acknowledgement does not answer
a question. These are attributed Employee claims, not semantic validation.

## Acceptance boundary

Required messages for the current Task visit are checked under the same Project
lock as outcome acceptance. An unresolved requirement rejects the outcome with
`instruction_pending`; no transition, handoff or completed queue is committed.
The Employee reads the instructions, acknowledges/answers and retries.

An answer, its message, receipt and Event/outbox are committed together.
Repeated identical Gateway calls return the recorded reply without appending
again. Events contain identities and authoritative timestamps, not message text.
Project stop revokes these tools immediately, independently of model attention.

Git-writing proposals reuse this barrier at the later quiescent candidate
acceptance boundary. A check at proposal time alone is insufficient: a required
instruction can arrive before the writer actually stops. In that case acceptance
holds the Task instead of silently advancing it. Taskless execution and native
driver proof remain separately tracked in the M2 execution ledger.

## Evidence

- Domain tests distinguish recipient/visit/fence, runtime ACK and Employee answer.
- Shared command scenarios run on both the reference adapter and PostgreSQL,
  including rollback, replay, stopped-Project persistence and cross-Run refusal.
- `inbox_acceptance` uses PostgreSQL, NATS, a manual Supervisor and a real UDS
  Gateway: required messages block outcome; ACK does not substitute for answer;
  reply retry deduplicates; unrelated general Inbox remains unconsumed.

These fixtures make no provider calls and do not establish live model behavior.
