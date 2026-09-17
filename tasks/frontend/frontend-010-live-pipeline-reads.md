# FRONTEND-010 — Просмотр версий Pipeline

**Epic:** [UI1.2](../../docs/epics/ui1-e2-pipeline-versions-hooks.md), ранний read-only срез;
[UI0.2](../../docs/epics/ui0-e2-browser-api.md), расширение gateway.
**Статус:** done
**Приоритет:** P0
**Дата:** 17 сентября 2026
**Baseline:** `3285f54` — FRONTEND-009 закоммичена отдельно, без push.
**Зависимости:** FRONTEND-002/008/009; ранний срез согласован пользователем.

**Goal:** видеть реальные версии Pipeline, стадии и переходы, не меняя процесс.
**Approach:** существующие Core list/detail и Zod contracts; отдельный раздел
Pipeline versions в live UI, typed inspector и keyless browser acceptance.
**Skills:** coding, rust-best-practices, react-dev, react-best-practices,
typescript-best-practices, playwright-best-practices, subagent-task-execution.

## Контракт

1. Новый gateway GET `/api/projects/{project_id}/pipelines?limit&cursor`.
   Detail `/pipelines/{pipeline_version_id}` уже существует. UUIDv7, строгие
   limit/cursor, owner auth, Host/Origin, UDS, deadlines и concurrency guards
   сохраняются. Default limit 20, диапазон 1–100, cursor до 1024 UTF-8 bytes.
2. List содержит полные definitions: его cap — 1 MiB, как у detail. Task/Run
   list caps остаются 64 KiB. Oversize даёт явный 502/response_too_large, без
   усечения, partial success или скрытого уменьшения страницы. Cursor-invalid
   остаётся безопасным 409. Нет новых Core endpoints, миграций или dependencies.
3. Третий раздел Pipeline versions рядом с Tasks/Runs; Tasks остаётся default.
   Список по 20, серверный порядок, Previous/Next/Refresh. Это версии, не полный
   каталог уникальных Pipeline; total/pinned-Task usage не выдумываются.
4. Список показывает name, Pipeline ID, version ID/номер, default/latest и
   soft-delete state. Карточка делает отдельное свежее чтение. Name, revision,
   default/latest и deleted_at относятся к изменяемому каталогу; immutable —
   definition версии. Default не обязательно latest; soft delete не удаляет
   историю. Весь DTO не кэшируется навсегда как immutable.
5. Карточка показывает Task kinds, entry stage, max_stage_visits, таблицы stages
   и transitions. Стадии произвольны; переход содержит from/outcome/target
   (stage/done/cancelled), artifact kind/minimum_count/scope. У выбранной стадии
   видны executor/outcomes/instructions и typed workspace/acceptance/system action.
   По умолчанию выбрана entry stage. Неизвестная stage reference сохраняет ID
   и явно не разрешается, без поиска по имени или подстановки другой стадии.
6. Null означает «не задано»: workspace не обещает отсутствие WorkSurface,
   acceptance не означает автоматическое принятие, max visits не гарантирует
   безграничное исполнение. System action описывается, но не запускается.
   Instructions — обычный текст, без HTML/Markdown исполнения. Неизвестные
   passthrough-поля, raw JSON и приватные hook/runtime данные не выводятся.
7. Cache изолирован session/Project/page/version; Task-specific pipeline key
   не переиспользуется. Смена раздела/Project отменяет reads и очищает старые
   queries после размонтирования, selection и pagination сбрасываются. Reload
   очищает Project selection; logout/401 — session cache. Late replies не
   восстанавливают старую область. Loading/empty/error/not-found/stale различны;
   failed refresh сохраняет помеченные stale данные. Обновление ручное.

## Порядок и проверки

- [x] Коммит FRONTEND-009: `3285f54`, без push; fmt, 43 gateway / 36 live unit PASS.
- [x] Gateway list allowlist, cap/query/security/error regressions.
- [x] Live API, presentation, список/карточка/stage inspector, scoped cache tests.
- [x] Отдельные real Core fixture Projects: 23 + 1 versions и пустой Project;
  named commands, default older than latest, soft delete; без provider Runs.
  Readiness metadata компактны, без definitions; текущие startup bounds сохранены.
- [x] Real browser pagination/scope/detail, список >64 KiB и ≤1 MiB;
  отдельно synthetic policies/executors/terminal targets и transport faults.
- [x] Oversize >1 MiB, cursor recovery, delayed reads, stale/retry, logout/401,
  keyboard/narrow viewport и safe text; регрессии Task/Run/auth/secret/sandbox.
- [x] Последовательные Rust/frontend/browser проверки: MemoryMax=6G,
  MemorySwapMax=1G, CPUQuota=200%, Cargo jobs ≤2.
- [x] Независимые spec/security/quality reviews и исправление findings.
- [x] Документация, index, runbook и фактическое evidence.

## Evidence

Проверено 17 сентября 2026, Linux/rootless Podman, Bun 1.3.11, Node 24.14.0.
Сборки и тесты выполнялись последовательно с указанными выше resource limits.
Provider credentials не читались; платные модели и пользовательский Core
не использовались. FRONTEND-009 закоммичена отдельно как `3285f54`, без push.

| Gate | Результат |
|---|---|
| `cargo fmt --all --check` | PASS |
| `cargo test --locked -p forge-ui -p forge-cli -p forge-protocol --all-targets` | 83 PASS: 46 gateway, 12 CLI, 25 protocol |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | PASS |
| `cargo build --locked -p forge-testkit --example ui_core_fixture` | PASS |
| `bun run typecheck` / `bun run lint` | PASS; 0 errors, прежние 10 fast-refresh warnings |
| Live unit / contracts / presentation | 42 / 47 / 20 PASS |
| `just ui-test-live` | PASS: live build, gateway/CLI/fixture, 2 egress tests, 1 physical sandbox test, browser и secret scan |
| Live browser | Два успешных прогона по 51 PASS: прежние 35 и 16 новых Pipeline scenarios |
| Intentional authenticated failure | В каждом прогоне ровно 1 ожидаемая ошибка, 0 дополнительных; scan 116 issued code/token по stdout/artifacts/dist-live — PASS |
| Demo browser / static Node + browser | 7 / 2 + 13 PASS |
| Ordinary / static / live builds | PASS; отдельные entry сохранены |
| Hygiene | `git diff --check`, generated-output ignore и локальные Markdown-ссылки — PASS; все изменённые code files ≤458 физических строк |

### Границы доказательства

Новая fixture через named commands создаёт 23 + 1 версии в закрытых отдельных
Projects; не создаёт в них Tasks, Employees или Runs. Используется уже имеющийся
пустой Project. В данных есть default v1 при latest v2, сохранённая после soft
delete версия, произвольные стадии и оба artifact scopes. Полная страница Core
проверена как больше 64 KiB и не больше 1 MiB. Readiness по-прежнему передаёт
только компактные metadata: исходные лимиты 40 s и 8192 bytes не увеличивались.

Настоящие Core/UDS/HTTP reads доказывают порядок и страницы 20 + 3, отдельный
fresh detail, foreign-Project 404 и cursor-invalid 409. Реальные задержанные
ответы проверяют смену версии, страницы, раздела, Project, logout и серверный
401. Проверены stale/retry, keyboard и отсутствие горизонтального overflow
на 375px. Это не запуск Employee по новому Pipeline.

Отдельные synthetic cases проверяют все четыре executor kinds, workspace
kind/access, acceptance verdicts и independence, обе system actions, mutable
catalog refresh и неизвестные ссылки на стадии. Instructions с HTML остаются
текстом; неизвестные поля, private canary и URL не отображаются и не порождают
дополнительных запросов. Policies только показываются: исполнения hooks или
проверки их acceptance semantics этот срез не обещает.

Synthetic browser faults проверяют recovery для 404/409/502/malformed response.
Фактические byte bounds отдельно проверены Rust UDS-тестами, включая ровно
1 MiB, declared/chunked oversize и error responses. Регрессия доказывает, что
тот же body больше 64 KiB допускается для Pipeline, но не Task/Run lists.

Gateway test сначала воспроизвёл RED (`NotFound`), затем GREEN после добавления
маршрута. Frontend unit tests были написаны до компонентов, но отдельный RED
прогон не выполнялся. Первые проверки нашли две ошибки новых тестов: вывод
tuple type и несовпадение ожидаемого текста malformed-response ошибки. Обе
исправлены; исходный browser прогон дал 50/51, затем два полных — 51/51.
Независимые gateway/fixture и frontend spec/security/quality reviews приняты,
оставшихся P0/P1/P2 findings в проверенном scope нет.

После проверок остановлены только поднятые для них PostgreSQL/NATS; volumes,
private schemas и безопасные diagnostics сохранены. `podman ps` пуст;
Core fixture, gateway и тестовый Chromium завершены. Ручной запуск описан в
[live runbook](../../frontend/README.md#live-owner-ui-frontend-007).
На момент завершения приёмки FRONTEND-010 оставалась локальной, без commit/push.
Отдельный запрос на commit/push получен 17 сентября 2026; вместе с ней
публикуется ранее подготовленный commit FRONTEND-009 (`3285f54`).

## Ограничения

Core загружает все версии до pagination и дополнительно читает каталог для каждой.
Новый browser cap не устраняет этот серверный gap. Допустимая полная страница
может превысить 1 MiB: это ограничение интерфейса, не признак повреждённой версии.
Полный UI1.2 gate не закрывается. Demo, редактор/publication/default/delete,
hook execution, SSE и Task→Pipeline navigation не входят в Task.
Публикация этого среза не закрывает полный UI1.2 gate.
