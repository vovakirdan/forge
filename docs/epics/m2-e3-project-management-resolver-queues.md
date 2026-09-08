# Epic M2.3 — Project management и resolver queues

**Milestone:** M2 — Engineering Pipeline и provider matrix
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-26

**Уточнение M2:** [specification](../2026-09-06-m2-specs.md) и
[execution ledger](../2026-09-06-m2-tasks.md) задают текущие границы и проверки.

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
- Employee capacity, taskless Communication, адресный Inbox/native input,
  точная next-Run constraint и durable resume alarm.

Resolution Run не получает права исполнять контекстную Task. Ответ продолжает
только источник вопроса; изменение Pipeline outcome или cancellation требует
своего разрешённого действия. Task и Communication имеют разные typed sources.

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
