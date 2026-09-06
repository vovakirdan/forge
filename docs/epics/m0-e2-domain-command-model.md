# Epic M0.2 — Canonical domain и command model

**Milestone:** M0 — Deterministic Core Simulator
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-03–05

**Сверка, 6 сентября 2026:** TASK-03–05 реализованы. Общий application engine
исполняет 14 команд M0 через PostgreSQL и in-memory adapters; conformance-suite
проверяет общие handlers с `ManualClock` и fault injection. Reference backend
добавлен после PostgreSQL; это изменение порядка реализации, а не исключение
из scope. Границы и команды проверки — в
[COMMAND_CONFORMANCE.md](../COMMAND_CONFORMANCE.md).

## Цель

Сделать утверждённые Task, Pipeline, dependency и escalation contracts
исполняемыми pure rules, а все мутации провести через один именованный command
boundary независимо от SQL и HTTP.

## В границах

- Task lifecycle, delivery/analysis, typed properties, terminal data, Artifacts,
  Events и minimal Employee eligibility;
- immutable PipelineVersion, transition matrix, dependency gates, wait condition
  и resolver-safe outcomes;
- application ports, actor/capability/revision checks, idempotency и in-memory
  reference repositories.

## Не в границах

SQLx, Axum, NATS, provider payloads, board columns, semantic LLM verdict и
direct repository mutation как альтернативный путь.

## Состав Task

| Task | Результат |
|---|---|
| TASK-03 | Task, Employee, Artifact и Event domain kernel |
| TASK-04 | Pipeline, dependency и escalation engine |
| TASK-05 | application commands и in-memory reference engine |

## Exit gate

Tests отклоняют invalid lifecycle transition, cancellation без reason, dependency
cycle и resolver outcome вне Pipeline. Идентичная повторная команда возвращает
прежний receipt; другой запрос с тем же ключом отклоняется. Domain crate не
импортирует storage, HTTP, Podman или provider SDK.

## Риски

Неоднозначность возвращается в domain-model proposal; её нельзя разрешить
случайным полем таблицы или веткой handler.
