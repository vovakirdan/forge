# Epic M2.3 — Project management и resolver queues

**Milestone:** M2 — Engineering Pipeline и provider matrix
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-26

## Цель

Сделать операционное управление и эскалации управляемыми named commands и
очередями, а не распределённой властью Employee.

## В границах

- Employee identity, prompt/runtime profile, enabled/retired state и scheduling
  eligibility;
- command registry System Manager: control message, stop, force stop, reassign,
  Project gate и recovery acceptance;
- resolver queues, ResolutionLease, human/employee resolution submission и
  escalation routing/failover.

## Не в границах

Manager LLM, строгая корпоративная иерархия, обязательные начальники или прямое
изменение lifecycle/stage Employee.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-26 | Employee, System Manager и resolver queues |

## Exit gate

Employee читает разрешённую projection и поднимает escalation, но не двигает Task
сам. Busy resolver не блокирует routing; Manager stop завершает Run по policy и
по умолчанию оставляет незавершённую Task в waiting.

## Риски

Control message не доказывает остановку process. Canonical outcome подтверждает
только Core/Supervisor evidence.
