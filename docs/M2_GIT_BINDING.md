# M2: local Git source and Task binding

Status: source registration and initial writer dispatch implemented. Candidate verification,
private snapshots and merge-commit integration exist as reusable Supervisor backend operations.
Writer proposal and quiescent candidate acceptance are described in
[M2 Git delivery](M2_GIT_DELIVERY.md). Read-only pinned snapshot dispatch is implemented;
Review verdict policy and [Core Integration](M2_GIT_INTEGRATION.md) have their own
acceptance slices.

## Operator-owned source allowlist

`register_project_repository` registers one immutable Project-owned local source. Its payload:

```json
{
  "name": "Mock application",
  "source": "/absolute/path/to/explicit/mock-repo",
  "target_ref": "refs/heads/main"
}
```

The common command envelope supplies `project_id`, `expected_revision` and the idempotency
header. The receipt returns `resource.kind = project_repository` and a generated UUIDv7.
Names are unique within a Project. Registration is the management source allowlist: worker
commands cannot supply or replace host source paths. There is no implicit Forge checkout,
current working directory or default repository. Changing a source requires a new entry;
registered entries cannot be updated or deleted.

Registration validates the path/ref shape, not filesystem existence, repository contents or
whether a branch is checked out. Paths are local absolute paths; remote URLs are not accepted.
Canonical path and actual Git object verification remain mandatory execution preflight.

## Task owns the binding

Before approval, `bind_task_git_repository` pins the source exactly once:

```json
{
  "task_id": "TASK_UUIDV7",
  "expected_task_revision": 1,
  "repository_id": "REGISTERED_REPOSITORY_UUIDV7",
  "initial_base": "FULL_40_OR_64_HEX_COMMIT_SHA"
}
```

The Task retains the repository ID, source and target-ref snapshot, full initial SHA and
generated persistent `surface_id`. The SHA is explicit operator input, not a verified claim;
Supervisor provisioning must resolve it to a commit in the selected repository before a Run
starts. A missing source or commit is an execution failure, never permission to fall back to
an Employee source or another repository. The Task cannot already have a retained execution
surface/attempt. Task and Project revisions, projections, Event, outbox and receipt commit
together. The Task snapshot and normalized surface/base/source projections must agree.

The Employee runtime still chooses provider, credentials, resources and access defaults.
For a Git-bound Task, dispatch overrides only the source/base from the Task and checks the
registered source and pinned Pipeline workspace requirement before provisioning. Two Tasks
may use the same repository/base, but never the same Task surface. Changing Employee does
not change this source contract. Existing unbound M0/M1 Task serialization remains `"none"`.

New Git-bound Tasks cannot dispatch via the fake runtime. A ReadOnly assignment requires an
accepted candidate for the same Project, Task and surface; a verified `needs_attention`
proposal is insufficient. Existing M1 unbound behavior is unchanged. A private snapshot alone
does not establish a review verdict or close the M2 handoff/acceptance feature.

## Pinned read-only snapshots

Core derives `surface.mode = git_candidate_snapshot` from the Task binding and latest accepted
candidate. It freezes the original `repository` and `base_ref`, plus exact `candidate.commit`
and `candidate.tree`, in the RunSpec. Employee configuration cannot select this derived mode.
The mode requires `access = read_only`; it cannot grant writer access or introduce a foreign
Task source. The Pipeline's Git workspace requirement applies to both writer and snapshot modes.

Supervisor checks the retained host-owned manifest against the Task, surface and original
source/base, then checks out the accepted commit into a new Run/fence/epoch-private directory.
The complete private Git root is mounted read-only at `/workspace`, with work at
`/workspace/worktree`. Later HEAD changes, tracked edits, untracked files and ignored leftovers
in the writer directory are not included. The writer directory and manifest remain unchanged.
Read-only snapshot Runs do not become the latest writer for later candidate inspection.

A missing/mismatched manifest, unavailable commit, different tree or preexisting destination
fails closed. Forge neither repairs the source, overwrites a retained snapshot nor silently
falls back to copying the mutable worktree. Git preparation has a bounded 60-second budget.
Writable test development remains a writer assignment, not this read-only lane.

## Post-stop inspection protocol

`InspectGitCandidate` uses envelope tag 5 and names only an exact Run/fence/epoch,
Project/surface, expected full commit, host and boot. Supervisor resolves the source through
its retained Task-writer registration and host-owned manifest. It accepts no filesystem path.
The latest writer owns this check; an earlier writer cannot be inspected after a newer writer
took the surface. ReadOnly Runs and taskless assignments do not replace that identity.

`GitCandidateInspectionResult` is a separate durable receipt, not another sequence in the
stopped Run stream. Its commit/tree fields are populated only for `verified`. An exact request
ID and payload replay the same receipt after ACK/reconnect/restart; reusing an ID with a different
payload is a conflict. Busy/unknown results are also immutable; retry after a state change uses
a new command ID. Journal capacity failures publish nothing. Minimal source identity survives
operational prompt/spec compaction; old tombstones without it fail closed.

Supervisor requires durable quiescence and checks actual container state, immutable environment
ID and host/boot/Run/fence/surface labels. A missing container alone is not proof: an already
recorded exact environment plus accepted durable Stopped acknowledgement is required for a
subsequently removed container. Current Run containers are normally retained. Candidate
inspection is asynchronous, has a 30-second overall budget, and never holds the journal while
calling Git/Podman. Provisioning contention returns busy without blocking stop controls.

## Reusable Git boundary

`forge_supervisor::git::GitBackend` verifies final candidates only after caller-confirmed
writer quiescence, exact HEAD and clean tracked/untracked state. Core remains responsible for
fencing and quiescence evidence. Private snapshots pin the exact candidate commit. Host Git
commands are time/output bounded and disable implicit hooks, fsmonitor and global config.
M2 fails closed on effective `filter.*` configuration (including includes/worktree scopes),
submodule/gitlink candidates, hidden index changes and redirected worktree metadata. Git
config is not silently stripped and project filters are not run outside the worker sandbox.

Integration is local merge-commit only. For current target `T` and approved candidate `H`,
`T` must be an ancestor of `H`; prepared merge `M` has parents `[T,H]` and exactly `tree(H)`.
No controller semantic merge, rebase or squash occurs after approval. Stale base needs an
explicit same-Task Pipeline return and fresh configured gates. No changes is an explicit
outcome. Preparation leaves the target untouched; Core must persist the exact intent before
apply. Apply uses a physical-repository/ref lock and compare-and-swap against `T`.

Bare targets are supported. Non-bare targets are supported only if the target branch is not
checked out, including all linked worktrees. Concurrent operator checkout/worktree operations
during integration are unsupported; Forge cannot lock the operator's shell. Reconciliation
uses actual target identity/ancestry and validates the exact prepared merge; a receipt ref
alone never proves the change was applied.
