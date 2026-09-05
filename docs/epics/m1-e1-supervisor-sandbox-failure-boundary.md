# Epic M1.1 — Supervisor, sandbox и failure boundary

**Milestone:** M1 — Первый реальный Employee
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-11–16

## Цель

Заменить fake runtime на наблюдаемый isolated execution boundary, сохранив Core
единственным владельцем canonical state и безопасное поведение при сбоях.

## В границах

- authenticated UDS gRPC/Protobuf между Core и Supervisor;
- rootless Podman provision, TaskWorkSurface modes и deny-by-default network;
- immutable RunSpec, ContextSnapshot, TaskHandoff и MinIO raw evidence;
- Tool Gateway, provider-neutral adapter contract и MCP façade;
- Watchdog, RunIncident, boot recovery, tracing, metrics и health.

## Не в границах

Real provider credential, Git engineering workflow, Grafana, Loki, Tempo, remote
runner и host-process fallback.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-11 | Core–Supervisor protocol/service skeleton |
| TASK-12 | rootless Podman и TaskWorkSurface manager |
| TASK-13 | RunSpec, handoff и raw-evidence retention |
| TASK-14 | Tool Gateway и adapter contract |
| TASK-15 | watchdog, incidents и recovery |
| TASK-16 | tracing, Prometheus и health baseline |

## Exit gate

Two-process test доказывает desired/observed boundary. Podman Run не видит host
home, container socket или сеть по умолчанию; force stop создаёт interrupted
handoff и waiting, а reboot отзывает старые lease без duplicate write-Run.

## Риски

Shell внутри собственной WorkSurface остаётся нормальным shell. Изоляция
ограничивает host boundary, а не работу агента внутри него.
