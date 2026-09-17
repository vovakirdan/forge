# Forge: Technology Stack

**Статус:** принято для локального MVP
**Дата:** 4 сентября 2026
**Граница:** технологии реализации. Доменные правила остаются в PRD и
`*-domain-model.md`.

**Delivery update, 11 сентября 2026:** этот документ фиксирует backend baseline.
Локальный Control Room теперь планируется до installer M4 в
[UI0–UI4](UI_IMPLEMENTATION_PLAN.md). Импортированный frontend не утверждает
новый runtime stack сам по себе: hosting/toolchain решения фиксируются в UI0.

## 1. Контекст

Forge — Linux-first local control plane для AI-команды разработки. MVP запускает
длительные изолированные Runs, хранит durable историю Task и допускает несколько
параллельных Employee. CLI остаётся операторским интерфейсом backend baseline;
локальный web UI добавляется отдельным workstream UI0–UI4. Его hosting/toolchain
выбраны в UI0: static client и будущий Rust gateway; remote control остаётся
отложенным. Реализованный static proof отделён от будущего защищённого ingress.

Ключевые ограничения:

- Core — единственный источник canonical state и единственный применяющий
  доменные переходы;
- Employee получает shell в своей изолированной рабочей среде, но не доступ к
  host home, чужим поверхностям работы, секретам и неразрешённой сети;
- локальная установка должна поднимать продукт одной командой и wizard, без
  ручной сборки набора сервисов;
- хранилища разделены по назначению: транзакционное состояние, durable events,
  кэш, бинарные объекты и retrieval index не подменяют друг друга.

## 2. Состав системы

| Часть | Реализация | Роль |
|---|---|---|
| Forge Core | Rust, Tokio, Axum, SQLx | команды, canonical state, scheduler, outbox, local API |
| Execution Supervisor | Rust, Tokio, Tonic/Protobuf | provision, stop и наблюдение за RunEnvironment |
| Provider adapter | Rust adapter boundary | перевод provider protocol в provider-neutral RunEnvelope и typed commands |
| Tool Gateway | Rust component Core plane | capability-checked доступ к MCP, памяти, интеграциям и секретам |
| API Provider Gateway | LiteLLM Proxy | upstream API keys, per-Run virtual keys, API-lane limits и observed usage/cost |
| Observability instrumentation | Rust `tracing` + OpenTelemetry API | structured process diagnostics, correlation and W3C trace context |
| RunEnvironment | rootless Podman OCI container | изолированное выполнение одного Run |
| CLI | Rust binary | основной local client Core API |
| Data services | rootless Podman containers | PostgreSQL, NATS JetStream, Redis, MinIO, AgentMemory, LiteLLM и Prometheus |

Core и Supervisor работают как нативные host services. Их нельзя запускать в
контейнере с проброшенным container socket: такой подход превращает изоляцию
RunEnvironment в иллюзию. Контейнеры используются для data services и самих
изолированных Runs.

## 3. Язык и application runtime

| Слой | Выбор | Причина |
|---|---|---|
| Язык | Rust | один язык для control plane, Supervisor, CLI и будущего Tauri/TUI; безопасная конкурентность и низкие накладные расходы на долгоживущий runtime |
| Async runtime | Tokio | зрелая основа для network I/O, процессов, таймеров и конкурентных Runs |
| Local HTTP API | Axum | естественно сочетается с Tokio, Tower и SSE; подходит для typed command API |
| Internal control protocol | Tonic + Protobuf over gRPC | строгие streaming-контракты между Core и Supervisor, генерация типов и forward-compatible schema evolution |
| PostgreSQL access | SQLx | явный SQL для locks, outbox, idempotency и audit-critical транзакций без ORM, скрывающей запросы |
| Serialization | Serde + Protobuf | JSON на внешней границе, Protobuf на внутреннем control channel |

Для Rust workspace Cargo — единственный package manager и build tool. Базовые проверки: `cargo fmt`,
`cargo clippy` и `cargo test`; integration tests запускают реальные сервисы в
изолированном test environment.

**Frontend developer baseline, 17 сентября 2026:** Linux, Bun `1.3.11` как
package manager/script runner и Node `24.14.0` для существующего Vite toolchain.
Единственный frontend lockfile — `bun.lock`, установка — frozen. React/TanStack/
Vite и Lovable wrapper импортированного прототипа сохранены без dependency upgrade.
Команды находятся в [frontend README](../frontend/README.md). Это решение для
локального demo; production hosting/browser auth остаются в UI0.2, а component
harness и CI — в оставшейся части UI0.3.

FRONTEND-002 добавляет только read-contract tests: установленный Zod 3 и
встроенный `node:test` в закреплённом Node, без нового test framework/dependencies.
Это отдельный `just ui-test-contracts`, не component/browser harness UI0.3.

**Browser smoke, FRONTEND-004:** `@playwright/test` строго `1.63.0` и его
Chromium. Это единственная новая прямая dev dependency; существующие runtime
dependencies не обновляются. Установка browser binaries — отдельная команда
`just ui-browser-install`, тестирование — `just ui-test-browser`. Playwright
управляет отдельным loopback Vite на `4173`, использует один worker и изолирует
browser contexts. Ни системный Chrome, ни запущенный вручную сервер не служат
fallback. Это mock-only smoke, не live Core acceptance; отдельный component
runner, CI и production hosting этим выбором не вводятся.

**Static hosting decision, FRONTEND-006:** установленный Forge будет отдавать
React/TanStack SPA assets через отдельный тонкий Rust/Axum gateway, без Node
runtime. `just ui-build-static` собирает `frontend/dist/client`; server bundle
нужен toolchain только для build-time shell prerender и не поставляется.
Lovable wrapper сохранён, но в отдельном static target выключены Nitro и
автоматический environment export. Прежний demo/build workflow не меняется.
`just ui-test-static` проверяет client assets через test-only Node file server
на 4174, без Core или SSR runtime. Зависимости и lockfile не меняются.

Gateway/auth — пока принятый target, не реализация. Local-owner bootstrap
использует terminal code; browser session — explicit bearer header, не cookies.
Причины, сроки жизни, security gates и границы будущего Tauri описаны в
[browser boundary ADR](UI_BROWSER_BOUNDARY.md). UI0.1/UI0.3 остаются открытыми;
static proof — согласованный ранний preparatory-срез UI0.2.

## 4. Execution isolation

### Primary backend: rootless Podman

Каждый `sandboxed_local` Run запускается отдельным rootless OCI container через
Podman. Supervisor создаёт RunEnvironment с выделенной `TaskWorkSurface`,
cgroup resource limits, отдельным filesystem view, deny-by-default network и
только scoped credentials. Агент сохраняет нормальный shell внутри своей среды;
изоляция ограничивает host boundary, а не команды в его собственной работе.

Podman выбран как практический default для Linux MVP: он даёт OCI images,
rootless user namespaces, cgroups и понятный operational path без отдельного
daemon, сохраняя возможность запустить dependencies тем же механизмом.

`host_process` существует только как явно выбранный trusted execution profile.
Если Podman или требуемые kernel features недоступны, Core отказывает в запуске
`sandboxed_local`; он не заменяет его незаметно host process.

### Deliberately excluded from MVP default

| Вариант | Причина |
|---|---|
| Bubblewrap | лёгкая process sandbox, но потребует самостоятельно собрать OCI image lifecycle, cgroup accounting, network и diagnostics вокруг него |
| gVisor | усиливает syscall boundary, но добавляет runtime compatibility и debugging cost до появления доказанного требования |
| Firecracker | даёт microVM boundary, но требует отдельного image, networking и host-operations контура; это профиль для будущих high-assurance/remote runners |

`ExecutionBackend` остаётся интерфейсом Core/Supervisor. Это позволяет добавить
другой backend без изменения Task, Pipeline, RunSpec или Tool Gateway contract.

## 5. Interfaces and compatibility

| Boundary | Protocol | Rule |
|---|---|---|
| CLI ↔ Core | HTTP/JSON commands | каждая mutation — именованная команда с idempotency key |
| Core → CLI | Server-Sent Events | поток доменных/операционных updates; bidirectional UI protocol в MVP не нужен |
| Core ↔ Supervisor | gRPC + Protobuf over authenticated local Unix socket | Supervisor сообщает observed state; Core остаётся authority для Task и Pipeline |
| Adapter ↔ Core | provider-neutral typed command messages | provider-specific protocol не становится частью доменной модели |
| RunEnvironment ↔ external world | Tool Gateway | MCP, память, integrations, secrets и разрешённая сеть проверяются capability policy и аудитируются |

External API по умолчанию локален. Текущий operator API доступен через owner-only
UDS; target local browser boundary и session contract приняты в
[ADR UI0.2](UI_BROWSER_BOUNDARY.md), реализация ещё впереди. Remote control и
public ingress остаются вне local MVP; capability checks остаются
в Core, а не в транспорте, UI или CLI.

## 6. Data and event layer

| Назначение | Выбор | Canonical rule |
|---|---|---|
| Transactional state | PostgreSQL | единственный source of truth для Project, Task, Pipeline, Employee, QueueEntry, Lease, Run, Artifact, events и outbox |
| Durable operational delivery | NATS JetStream | доставляет outbox events Scheduler, Summarizer и projections; consumer обязан быть idempotent |
| Cache and ephemeral coordination | Redis | cache, rate/lease helpers и short-lived coordination; никогда не authority |
| Artifacts and large logs | MinIO through S3 API | binary bodies, chunked technical logs и archives; PostgreSQL хранит metadata, hash, ACL и retention |
| Retrieval | AgentMemory adapter | сменная projection/search service; canonical scope, visibility, revisions и source links хранятся в Forge |
| API LLM gateway | LiteLLM Proxy | держит upstream API credentials host-side, выпускает virtual keys для Run и сообщает observed usage/cost; его schema не является Forge canonical state |
| Secret storage | Forge Secret Store | AEAD-encrypted blobs и metadata в PostgreSQL; master key из Linux keyring или headless file fallback |
| Operational metrics | Prometheus | scrapes local `/metrics`, хранит bounded history; не хранит canonical Task state или raw logs |

PostgreSQL transaction сначала фиксирует state, immutable event и outbox record.
Только затем NATS доставляет работу асинхронным consumers. Поэтому потеря Redis,
NATS или AgentMemory не может откатить принятую команду и не создаёт второй
доменный переход.

Redpanda/Kafka не входят в MVP. Их добавление оправдано только при доказанной
потребности в partitioned replay большой телеметрии или множестве независимых
долгоживущих consumers; JetStream покрывает начальную operational delivery.

## 7. Memory and artifacts

Forge хранит canonical `EmployeeMemoryEntry` и `ProjectKnowledgeEntry` в своей
модели. AgentMemory получает retrieval projection через outbox и возвращает
кандидаты поиска. Перед выдачей в `ContextSnapshot` Core повторно проверяет
scope, visibility и revision canonical record.

Employee не записывает persistent memory напрямую: он прикладывает Artifact или
структурированное observation к Task. Summarizer формирует derived summaries и
memory entries с source links; human или уполномоченный Manager принимает
authoritative knowledge. Артефакты Task и raw technical logs — разные storage
классы с независимыми access и retention policy.

## 8. Local installation and operations

Установка Linux-first состоит из одного Forge installer/wizard:

1. проверяет kernel, user namespace, cgroup v2 и rootless Podman prerequisites;
2. устанавливает или проверяет нативные Forge Core, Supervisor и CLI binaries;
3. создаёт project-local конфигурацию и persistent volumes;
4. поднимает PostgreSQL, NATS JetStream, Redis, MinIO, AgentMemory, LiteLLM и
   Prometheus rootless containers;
5. проверяет local Core API, Supervisor control socket и sandboxed test Run.

Installer создаёт user-level `systemd` units для Forge Core и Execution
Supervisor и включает linger, поэтому local Forge продолжает работать после
logout и стартует после reboot. Units используют `Restart=on-failure` с
bounded backoff и systemd start-limit, а не бесконечный hot loop. Foreground
`forge daemon` остаётся отдельным режимом разработки.

Restart unit не является командой продолжить работу: Core применяет durable
Project `BootRecoveryPolicy` и recovery rules к незавершённым Runs. Data
containers переживают рестарты через persistent volumes. Внешний S3 совместим с
MinIO contract и может заменить local object store без смены модели Artifacts.

## 9. Compatibility rules

- SQLx migrations создают и изменяют только PostgreSQL canonical schema.
- Все contracts между Rust components versioned: Protobuf для internal control,
  OpenAPI-described JSON для local commands и stable event schema для SSE.
- Podman получает RunSpec только от Supervisor; adapter не запускает контейнеры
  самостоятельно.
- Никакой adapter, broker, cache или retrieval service не имеет database write
  authority над доменными таблицами Core.
- LiteLLM использует изолированную service schema и не пишет в Forge canonical
  tables. Core сопоставляет его virtual key и observed usage с конкретным Run.
- `/metrics` содержит только bounded-cardinality operational metrics. Task/Run
  identifiers, prompts, secrets и raw URL не становятся Prometheus labels.
- OpenTelemetry-compatible spans передают W3C trace context по Core/Supervisor
  gRPC и разрешённым HTTP boundaries. Collector/Tempo не входят в default MVP
  deployment, поэтому application не зависит от их availability.
- Raw Run data никогда не подмешиваются автоматически в Artifact, memory или
  prompt; потребление всегда идёт через явно выбранный projection/excerpt.

## 10. Rejected alternatives

| Слой | Альтернатива | Почему не выбираем для MVP |
|---|---|---|
| HTTP API | Actix Web | сильный framework, но Axum проще совмещается с выбранными Tower/Tonic patterns |
| PostgreSQL access | Diesel | compile-time guarantees ORM не окупают менее прямой контроль над сложными transactional queries и outbox |
| PostgreSQL access | SeaORM | active-record abstraction скрывает важные SQL details control plane |
| Durable broker | Redpanda/Kafka | operational overhead не оправдан для одного local Forge host на старте |
| Realtime | WebSocket | MVP передаёт server-to-client updates; SSE проще и достаточен |
| Sandbox default | host process | не даёт требуемой boundary между параллельными Runs и host |
| Log platform | Loki/Tempo/Grafana stack | raw Run logs уже имеют отдельное хранилище; full trace backend добавляется только с multi-host или доказанной потребностью в trace query UI |

## 11. Deferred product surfaces

Local web UI развивается отдельным [workstream](UI_IMPLEMENTATION_PLAN.md).
Tauri desktop shell, полноценный remote control, distributed runners и
high-assurance microVM profile остаются отложенными. Выбор не мешает
им: Rust shared domain/API client подходит Tauri и TUI, S3 API допускает
внешнее object storage, а `ExecutionBackend` допускает будущие runner profiles.
