# Epic M2.2 — Git delivery, verification и independent review

**Milestone:** M2 — Engineering Pipeline и provider matrix
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-24–25

**Уточнение M2:** [specification](../2026-09-06-m2-specs.md) и
[execution ledger](../2026-09-06-m2-tasks.md) задают текущие границы и проверки.

## Цель

Дать delivery Task настоящий engineering loop, остающийся одной Task при
verification failure, review return и merge conflict.

## В границах

- Task-owned Git worktree, quiescent clean candidate и точный commit/tree;
- необязательные явно настроенные project hooks в отдельном provider-free Run;
- Integration lock, durable merge intent, target CAS и crash reconciliation,
  без скрытых commit/rebase/reset;
- review read-only snapshot, reviewer independence, typed verdict/comments и
  artifact acceptance.
- QA как деятельность Employee: отчёт либо явно назначенная разработка тестов;
  никакого автоматического обнаружения или обязательного «run all tests».

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
