# FRONTEND-002 — Контракты Task и Pipeline

**Epic:** [UI0.1](../../docs/epics/ui0-e1-domain-contracts.md)
**Статус:** done — read-contract срез; UI0.1 остаётся in_progress
**Приоритет:** P0
**Дата:** 17 сентября 2026
**Зависимости:** [FRONTEND-001](frontend-001-local-demo-baseline.md) — done

## Цель и контекст

Подготовить проверяемые frontend read contracts для будущего подключения Core,
не принимая упрощённую mock-модель за доменную истину. Baseline `3854acf` уже
имеет воспроизводимый local demo, но его экраны смешивают lifecycle и стадии.
Эта задача оставляет экраны без изменений и добавляет изолированный новый слой.

## Источники

- [UI plan](../../docs/UI_IMPLEMENTATION_PLAN.md),
  [field/gap map](../../docs/UI_BACKEND_ALIGNMENT.md#8-read-contracts-frontend-002).
- [Task domain](../../docs/2026-09-03-task-domain-model.md),
  [Pipeline domain](../../docs/2026-09-03-pipeline-domain-model.md),
  [M2 Pipeline management](../../docs/M2_PIPELINE_MANAGEMENT.md),
  [M2 Integration](../../docs/M2_GIT_INTEGRATION.md).
- [Current HTTP serializers](../../crates/forge-core/src/http/views.rs),
  [OpenAPI](../../openapi/v1.yaml), [PRD](../../forge-prd-v0.1.md).
- [Rules](../../docs/PROJECT_RULES.md), [STACK](../../docs/STACK.md),
  [ARCHITECTURE](../../docs/ARCHITECTURE.md).

## Scope и реализация

- `frontend/src/contracts/`: установленные Zod 3 schemas и inferred TypeScript
  types для Project detail, Task summary/detail/list и PipelineVersion detail/list.
- Вложенные properties, waits, artifacts, stages, transitions; additive fields
  сохраняются. DTO не объявляют отсутствующие catalogs, cancellation reason,
  project scope, Employee assignment, Run activity или обязательный Git.
- Различать обязательный null и omission; priority/wait/stage IDs открыты,
  lifecycle/kind закрыты. Revisions — safe integers без преобразования wire type.
- `resolveTaskStage(task, pipeline)` возвращает `resolved`, `no_stage` либо
  `unavailable` с причиной `version_not_loaded`, `version_mismatch` или
  `stage_not_found`. Не выбирает default/entry stage, не исполняет transitions.
- Synthetic fixtures и colocated `*.test.ts`; Node `node:test` с concurrency 1,
  относительными `.ts` imports, без JSX/TS enums/Vite aliases.
- `test:contracts` и root `just ui-test-contracts`; без новых dependencies.
- Карта происхождения полей/gaps; исправление только устаревших PRD-фраз о выборе
  версии при approval и скрытом rebase в Integration. Core поведение не меняется.

## Non-goals

HTTP-клиент и browser boundary, изменения API/OpenAPI/backend, миграция demo
types/services/screens, Employee/Run/Surface contracts, Project schema catalogs,
новый scheduler/переходы, UI0.3 component/browser harness/CI, Rust validation,
provider Runs, commit и push.

## Acceptance и тестовые сценарии

1. Оба TaskKind и все шесть lifecycle; waiting на произвольной стадии с двумя waits.
2. Analysis без Git, draft и отменённый draft с null DoD/stage.
3. Два разных Pipeline graph, включая граф без review/QA; terminal targets не
   превращаются в фиктивные стадии. Array order не определяет flow.
4. Pin v1 при default v2; soft-deleted Pipeline по-прежнему читается.
5. Все tagged properties, особенно `[year, ordinal_day]` и typed reference;
   arbitrary JSON body, object-root metadata и дополнительные поля не теряются.
6. Открытые priority/wait keys и `TASK-001` принимаются; неверные типы,
   `research` kind, `review` lifecycle и unsafe revisions отклоняются.
7. Обязательный null, omission artifact requirements и пагинация проверены.
8. Все результаты resolver; нет fallback к default/entry и мутации входов.
9. Contract tests, typecheck, lint без новых warnings и build проходят;
   lockfile и legacy source не меняются; независимое review не оставляет findings.

## Проверка и evidence

Команды из корня: `just ui-test-contracts`, `just ui-typecheck`, `just ui-lint`,
`just ui-build`, `git diff --check`, `just --unstable --fmt --check`.
Тяжёлые проверки выполняются по очереди через `systemd-run --user --scope --quiet
-p MemoryMax=6G -p MemorySwapMax=1G -p CPUQuota=200% timeout 300s …`.
Build может регенерировать tracked route manifest; после него проверить diff.

Проверено 17 сентября 2026 на Linux, Bun `1.3.11`, Node `24.14.0`.
Root повторил проверки после implementer; tests/typecheck/lint/build выполнялись
последовательно в scopes с указанными выше лимитами.

| Проверка | Результат |
|---|---|
| `just ui-test-contracts` | PASS: 29 tests, 0 failures/skips; Node duration около 744 ms |
| `just ui-typecheck` | PASS, exit 0; includes новые schemas, fixtures и tests |
| `just ui-lint` | PASS, 0 errors; те же 10 прежних react-refresh warnings |
| `just ui-build` | PASS, client/SSR/Nitro; существующий Cloudflare-module target |
| `git diff --check`, `just --unstable --fmt --check` | PASS |
| Local Markdown links | PASS, 119 ссылок при финальной сверке после evidence |
| Dependencies/lockfile | declarations и `bun.lock` без изменений |
| Legacy source/backend/OpenAPI | Без изменений, включая tracked route manifest |
| Spec review / quality review | APPROVE; открытых findings нет |

В read layer добавлено 15 файлов; tests/fixtures не экспортируются production
entrypoint. `preserveWireValue` проверяет значения, затем возвращает исходные
данные: так Zod не теряет дополнительные `__proto__` JSON keys. Это validation-only
helper, не место для coercion/defaults/нормализации. Возвращённые данные по-прежнему
нельзя считать безопасным HTML, URL или доверенной конфигурацией.

В ходе review исправлены три findings и добавлены регрессии: bounded cycle в
analysis fixture, сохранение special JSON/additive keys, object-root metadata
при произвольном JSON body. Проверки special keys охватывают 18 публичных object
schemas и вложенные данные, включая сохранение путей ошибок валидации.

SHA-256 `bun.lock` до/после:
`fc00072f27cf820a6b27ab8c9de583f0c508ff16aaa40ddb403271cc65991935`.
Generated node_modules/.output/.wrangler остаются ignored. Существующее build
предупреждение о `vite-tsconfig-paths` не устранялось; wrapper не менялся.

Synthetic unit tests доказывают поведение read-contract слоя, не live API
integration. Browser smoke не повторялся: экраны не менялись и новый слой ими
не используется. Core/services/provider Runs/Rust validation не запускались.
Прежние 10 react-refresh warnings, API gaps и полные gates UI0.1/UI0.3 остаются
видимыми; они не объявлены исправленными в этой Task. Commit/push не выполнялись.

## Порядок, делегирование и риски

Один implementer владеет `src/contracts`; root — командами/docs/tasks. Spec и
quality reviews выполняются независимо от автора. Нельзя запускать одновременно
build, полный lint/typecheck и Rust checks в общей копии после прежнего OOM.

Следующие шаги UI0.1 — оставшиеся сущности/каталоги и адаптация потребителей;
UI0.2 ждёт полного UI0.1 и UI0.3. Эта задача их gates не закрывает.
Существующий OpenAPI drift явно перечислен в gap map; API не исправляется молча
вместе с frontend. Fixtures не импортируются production contract entrypoint.
