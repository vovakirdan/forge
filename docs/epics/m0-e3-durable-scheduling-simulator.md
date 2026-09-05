# Epic M0.3 — Durable scheduling и simulator acceptance

**Milestone:** M0 — Deterministic Core Simulator
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-06–10

## Цель

Соединить command model с canonical PostgreSQL, transactional outbox, NATS и
deterministic scheduler так, чтобы Forge доказал основную ценность без реального
исполнителя.

## В границах

- migrations/repositories для canonical state, Event, outbox, QueueEntry, Lease,
  Run, Artifact и TaskDependency;
- at-least-once outbox delivery и consumer deduplication;
- QueueEntry claim, capacity, priority/age, dependency gates, Project stop,
  Lease/fencing и fake Supervisor;
- local HTTP/JSON commands, CLI, SSE projections и M0 acceptance harness.

## Не в границах

Podman, Git, LLM, provider keys, real process lifecycle и UI.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-06 | PostgreSQL transaction/store/migrations |
| TASK-07 | outbox и NATS JetStream delivery |
| TASK-08 | scheduler, Lease и fake Supervisor |
| TASK-09 | local API, CLI и SSE baseline |
| TASK-10 | deterministic end-to-end acceptance |

## Exit gate

При capacity = 1 две Task дают один active fake Run и одну queued Task. Первая
проходит implementation, retry, verification, review и integration как одна
карточка. Stale event, replayed outbox, unsatisfied dependency и stop дают
известный audit/state outcome.

### M0 reconciliation terminal Run

`Stopped`, `Failed` или `Lost`, пришедший до принятого `StageOutcome`, не
считается успешным завершением stage. Core в одной canonical transaction
отменяет ещё leased QueueEntry, освобождает Lease и переводит Task в
`waiting` с `Interrupted` condition. Следующий ожидающий Task может занять
освободившуюся capacity; прерванная Task не ретраится молча и ждёт явного
manager/human resolution. Если QueueEntry уже `completed` после принятого
outcome, terminal observation только освобождает Lease и не создаёт лишний
interruption wait.

Stop intent является monotonic до terminal observation: поздний Supervisor
`Running` не может переписать durable `stop_requested` или
`force_stop_requested`. `retry_exhausted` также оставляет пару audit facts:
`task_retry_exhausted` и `task_waiting`.

Priority change заменяет queued snapshot только когда Task ещё не leased. Для
active Run Core сохраняет новую priority в Task, но не создаёт второй active
QueueEntry: следующая stage/retry унаследует значение после terminal fence.

Stale fence или environment epoch не меняет Run, Task либо QueueEntry. Core
сохраняет отдельный `run_observation_ignored` audit Event с безопасными
ожидаемыми и полученными fence/epoch полями и помещает его в transactional
outbox; обычный duplicate уже принятого сообщения не создаёт новый transition.

## Риски

NATS не становится source of truth, а fake Supervisor не обходит fencing и не
создаёт вторую модель переходов.
