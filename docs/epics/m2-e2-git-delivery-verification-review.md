# Epic M2.2 — Git delivery, verification и independent review

**Milestone:** M2 — Engineering Pipeline и provider matrix
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-24–25

## Цель

Дать delivery Task настоящий engineering loop, остающийся одной Task при
verification failure, review return и merge conflict.

## В границах

- Task-owned Git worktree, base/final SHA и versioned verification profiles;
- deterministic command runner, Integration lock, rebase/merge/final checks;
- review read-only snapshot, reviewer independence, typed verdict/comments и
  artifact acceptance.

## Не в границах

External CI import, deploy stage, UI review editor, отдельная FixTask и
automatic semantic approval текста review.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-24 | Git worktree, verification и Integration Controller |
| TASK-25 | independent review и artifact acceptance |

## Exit gate

Fixture repository проходит failed verification, retry, review changes, retry и
integration merge. Только Integration изменяет protected main; один Task хранит
attempts, Artifacts и final SHA. Reviewer не получает write mount и не approve
свою работу при включённой policy.

## Риски

Git surface не становится ownership Employee. Lost/force-stopped Run сначала
проходит handoff/recovery, а не автоматический re-execute.
