# Epic M0.1 — Workspace и local test topology

**Milestone:** M0 — Deterministic Core Simulator
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-01–02

## Цель

Подготовить воспроизводимую Rust-среду и local dependencies, чтобы дальнейшие
изменения одинаково проверялись на чистом Linux host и у разработчика.

## В границах

- Cargo workspace, toolchain, shared lints, Cargo.lock и quality commands;
- empty crate boundaries из ARCHITECTURE.md без бизнес-логики;
- rootless Podman topology для PostgreSQL, NATS JetStream, Redis и MinIO;
- readiness, isolated test namespaces и synthetic configuration;
- короткая документация development mode.

## Не в границах

Migrations, domain types, API, Supervisor, AgentMemory, LiteLLM, Prometheus,
реальные provider credentials и production installer.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-01 | Cargo workspace и repeatable quality workflow |
| TASK-02 | local data-service и integration-test topology |

## Exit gate

Cargo format, Clippy и tests пустого workspace проходят; rootless services
поднимаются, а disposable test создаёт и очищает PostgreSQL schema, NATS stream
и MinIO bucket без секретов в repository.

## Риски

Dev topology не становится production installer и не даёт data containers
container socket или доступ к RunEnvironment.
