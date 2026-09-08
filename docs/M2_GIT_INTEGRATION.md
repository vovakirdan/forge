# M2: local Git Integration

Integration is an explicit deterministic Pipeline action. It does not create an
Employee, Task execution lease or synthetic Run. It uses the Task's registered
local repository and target branch, never the Forge checkout by default.

## Configure a System stage

The owner chooses stage and outcome names. For example:

```json
{
  "id": "publish_here",
  "name": "Local integration",
  "executor_kind": "system",
  "outcomes": ["landed", "nothing", "refresh"],
  "system_action": {
    "kind": "git_integration",
    "outcomes": {
      "applied": "landed",
      "no_changes": "nothing",
      "stale_base": "refresh"
    },
    "required_review_stages": []
  }
}
```

The Pipeline declares transitions for these outcomes. `stale_base` must return
the same Task to an Employee stage. A worker merges the current target into the
Task surface and submits a new committed candidate. That revision follows the
configured Pipeline again. Forge does not silently merge, rebase, squash,
auto-add or commit uncommitted worker changes.

`required_review_stages` defaults to empty. Each configured ID must name a stage
with a candidate-review policy in this immutable Pipeline version. Integration
requires the latest record for each stage and exact candidate proposal to be
`accepted`; a later rejected or inconclusive assessment supersedes acceptance.
No review stage, test command or hook is inferred from repository files or stage
names. Optional repository hooks are a separate executor feature.

## Prepare, persist, apply

Core pins the accepted candidate, writer provenance, Task surface, Pipeline
version and stage visit in a fenced Integration operation. All Task environments
must be physically quiescent, the Project must allow execution, and current
candidate, Inbox and configured-review gates must pass.

Supervisor prepares in a private retained directory. For current target `T` and
accepted candidate `H`, `T` must be an ancestor of `H`. Merge `M` preserves the
worker commits, has parents `[T,H]` and exactly `tree(H)`. A stale base returns its
configured outcome; no changes is also explicit. Preparation does not update the
target branch.

Supervisor journals the prepared intent; Core persists the exact `T/H/M` before
sending Apply. Apply rechecks physical ownership and performs a compare-and-swap
under the physical repository/ref lock. Reconciliation reads the actual target:
`M` or a descendant proves Applied, unchanged `T` with consistent evidence is
Retryable, and incompatible movement remains Unknown. A receipt ref alone never
proves publication.

Core records request and reply receipts independently of the stopped Run stream.
Replay preserves the exact command payload, result and merge identity. A normal
accepted result attaches a System `integration_result` artifact and applies the
mapped Pipeline outcome in the same canonical transaction. A stale Task revision,
changed stage, stopped Project or restarted Core preserves the physical result
without applying an obsolete Task transition.

The same transaction records a canonical `SystemAction` handoff, with the
integration artifact and the actual next stage. Returning to work therefore gives
the next Employee the integration result, not a handoff addressed to the previous
System stage. Handoffs have independent IDs; historical Run handoffs keep their
Run identity and immutable body. A System handoff has no Run ID.

## Stop and recovery

`GET /v1/projects/{project_id}/tasks/{task_id}/integrations` reads retained
operations, including held and retired attempts. It accepts `after` (UUIDv7) and
`limit` (1–100, default 50). The view includes candidate and merge SHAs, result,
state and wait ID; it excludes host paths and control-message payloads.

Project stop prevents new Prepare/Apply delivery. An already delivered filesystem
effect may finish; stopping cannot undo a completed Git ref update. The result
remains in the Integration ledger. Timeouts, ambiguous state and failed gates
place the operation on hold and add an explicit Task wait when it still owns the
current stage visit.

After Core restart, retained operations perform read-only reconciliation and
require explicit management. Each Core process has a fresh instance identity;
clones inside the same process share it. Ordinary `resume_task` removes its
specified wait but cannot create a replacement operation or authorize Apply.

After repairing the cause and resuming the same Task visit, use a named command
with the **current** Task revision, the operation UUID and a nonempty reason:

```json
{
  "operation_id": "OPERATION_UUIDV7",
  "expected_task_revision": 12,
  "reason": "Target configuration repaired; retry the retained intent"
}
```

- `retry_git_integration` first requests a fresh read-only observation. Retryable
  authorizes Apply of the same retained intent. It does not regenerate `M`.
- If no intent exists, Retry requires Supervisor's explicit `no_preparation`
  proof: correct host and operation, retained exact writer provenance, no
  preparation and no Apply receipt. Core also verifies that it never authorized
  Apply. Only then does it retain the old attempt as `retired` and create a new
  fenced operation. Offline, missing ownership or generic Unknown is not proof.
- `accept_git_integration_result` applies only a previously observed Applied
  result. It requests fresh reconciliation and rechecks current Task/candidate
  gates, then applies the normal mapped outcome. It never sends Apply/CAS.

If reconciliation recovers a prepared intent whose reply was lost, that exact
intent is retained on hold. A subsequent explicit Retry obtains the fresh target
observation before any Apply. Conflicting or irrecoverably missing evidence stays
held for investigation; history and staging directories are not deleted.

## Boundaries and verification

Bare local targets are supported. A non-bare target is supported only while its
target branch is not checked out in any worktree. Concurrent operator checkout or
worktree administration during integration is unsupported; Forge does not lock
the owner's shell. Git filters, submodules and unsafe metadata fail closed at
the host boundary described in [M2 Git binding](M2_GIT_BINDING.md).

Supervisor tests use actual temporary Git repositories and a synthetic container
inspection fixture. Core acceptance tests use PostgreSQL, Gateway and the local
Supervisor transport with synthetic Git observations. Neither constitutes a live
provider run or changes an operator's repository. Both boundaries must pass their
own tests before this slice is considered verified.
