# M2: Git inputs for future Runs

A Task's original Git binding identifies its retained work surface. A separate,
versioned source policy selects the repository input available to each new writer Run.
Changing this policy preserves the Task revision, current proposal, accepted candidates,
existing RunSpecs and retained files.

## Source policy

Every Git Task starts at revision 1 with `{"mode":"latest_target"}`. Before each new
writer Run, Core freezes this policy and its revision in TaskRunSpec v6. Supervisor
resolves the registered target once, records that selection durably, and exports a
read-only Git bundle. Replaying provisioning reuses that exact selection even if the
target has moved. The source is not refreshed inside an existing Run.

Human or System Manager can call `set_task_git_source_policy` using the usual command
envelope and idempotency key:

```json
{
  "task_id": "TASK_UUIDV7",
  "expected_policy_revision": 1,
  "policy": {"mode": "pinned_commit", "commit": "FULL_COMMIT_SHA"},
  "reason": "Use the agreed baseline for future attempts"
}
```

The result is revision 2. The pin applies to all future writer Runs until another
management command selects `{"mode":"latest_target"}` or another commit. Employees
cannot issue this management command. Policy history, audit, outbox and command receipt
commit atomically. The Task has no new revision solely because its policy changed.

`GET /v1/projects/{project_id}/tasks/{task_id}/git-source-policy` returns
`{"revision":2,"policy":{"mode":"pinned_commit","commit":"..."}}`.
Unbound and foreign-Project Tasks return 404. The command validates full commit format;
Supervisor verifies that the object exists in the registered source before launching.

## Run-local delivery

The writer reads `/run/forge-source/descriptor.json`. It contains repository ID,
target ref, policy revision, policy, selected revision, object format and bundle digest
and size. It contains no host path. For a committed selection, `source.bundle` is
mounted beside it read-only. The Employee imports the selected commit into its own refs
and explicitly merges/rebases as the Task requires. Forge never resets the retained
branch, discards files or silently performs a semantic merge.

Core validates the receipt against the frozen request, stores it immutably, and includes
it in the canonical `run_observed` event. Run detail exposes it as `diagnostics.git_source`,
including after later observations replace the current runtime state. A source selection
is evidence of input delivery, not proof that the Employee used it correctly.

## Empty repositories and publication

`bind_task_git_repository.initial_base` accepts
`{"kind":"unborn","object_format":"sha1"}` or `sha256`. No synthetic empty commit,
zero SHA or null is created. Committed bindings remain backward-readable as a full SHA
string; explicit `{"kind":"commit","commit":"..."}` input is also accepted and
stored in the compatible string form.

An unborn source receipt has no bundle. The first writer creates a real root commit.
After normal candidate/review gates, integration publishes that exact candidate using
create-only compare-and-swap against an absent target. Its intent explicitly carries
`expected_target:null`; omitting the field is invalid. A competing publication yields
`stale_base` rather than overwriting the winner. Once the target has existed, deleting it
does not restore permission for an initial publication.

For an existing target, the normal two-parent merge-commit integration contract applies.
A stale `latest_target` task follows the Pipeline's declared rework outcome. A stale
`pinned_commit` task instead enters an integration hold for management; Forge does not
discard the pin or repeatedly send the same stale work through automatic rework.

Legacy TaskRunSpec v2 remains readable with its original behavior. New real Task Runs
use v6; Communication, Resolution and Hook retain their separate versions. Supplemental
file snapshot inputs are independent of the Git candidate and source policy.
