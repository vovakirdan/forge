# FRONTEND-003 — Контракты Run и диагностики исполнения

**Epic:** [UI0.1](../../docs/epics/ui0-e1-domain-contracts.md)
**Статус:** done — read-contract срез; UI0.1 остаётся in_progress
**Приоритет:** P0
**Дата:** 17 сентября 2026
**Зависимости:** [FRONTEND-002](frontend-002-task-pipeline-contracts.md) — done

## Цель и контекст

Продолжить изолированный frontend read layer из baseline `a1c62e6`:
описать назначение Run, его владельца, desired/observed states и диагностику.
Task lifecycle, Run activity и Employee identity остаются разными понятиями.
Экраны продолжают работать на прежнем demo; live-интеграция сюда не входит.

## Источники

- [UI plan](../../docs/UI_IMPLEMENTATION_PLAN.md),
  [field/gap map](../../docs/UI_BACKEND_ALIGNMENT.md#9-run-read-contracts-frontend-003).
- [HTTP views](../../crates/forge-core/src/http/views.rs),
  [routes](../../crates/forge-core/src/http/handlers.rs),
  [assignment](../../crates/forge-domain/src/assignment.rs),
  [SystemJob](../../crates/forge-domain/src/system_job.rs).
- [Run states](../../crates/forge-storage/src/model.rs),
  [diagnostics projection](../../crates/forge-storage/src/run_evidence.rs),
  [canonical SQL constraints](../../crates/forge-storage/migrations/0001_m0_canonical_schema.sql).
- [OpenAPI](../../openapi/v1.yaml),
  [Rules](../../docs/PROJECT_RULES.md), [STACK](../../docs/STACK.md).

## Scope

- Zod schemas и inferred types для `ExecutionAssignment`, desired/observed
  states, Run list/detail и внешней структуры diagnostics.
- Пять assignments: TaskStage, Communication, Resolution, Hook, SystemJob;
  SystemJob kinds — summarization/onboarding. Поля owner повторяют serializer.
- Только TaskStage имеет верхние `task_id`/`stage_id`; Task внутри Hook owner
  остаётся контекстом. Hook/SystemJob могут не иметь Employee. Несколько Run
  одного Employee не сводятся к одному currentRun/currentTask.
- `attempt` — положительный u32; RunSpec version — положительный u16, не enum.
  Fencing/epoch/generations/stage visits — положительные safe integers;
  `last_observed_sequence` — неотрицательный safe integer.
- Detail содержит обязательный diagnostics object. `runtime_report`, `handoff`,
  `proxy_usage`, `git_source` — обязательные nullable JSON objects;
  incidents/evidence — массивы JSON objects; streams — `{stream,incomplete}[]`.
  Содержимое отчётов не интерпретируется и не доказывает acceptance/успех.
- UUIDv7, stable keys, JSON и `preserveWireValue` переиспользуются. Additional
  fields сохраняются; нет coercion, defaults, normalization или state machine.
- Synthetic fixtures и colocated `node:test`; production barrel не экспортирует
  fixtures/tests. Используется существующий `just ui-test-contracts`.

## Non-goals

Employee/Surface generic DTO для отсутствующих endpoints; детальные schemas
RuntimeReport/Evidence/Handoff/GitSource; HTTP client, SSE, экраны и demo types;
backend/OpenAPI/PRD изменения; новые dependencies; services/provider Runs;
Rust validation; commit и push. JSON validation не разрешает HTML rendering,
доступ к host paths или загрузку по object key.

## Acceptance и тестовые сценарии

1. Все пять purposes, обе разновидности SystemJob; неправильные owner shapes
   и неизвестные purpose/kind/state отклоняются.
2. Taskless Communication/Resolution, Hook с контекстной Task при null верхних
   Task/stage IDs, SystemJob без фиктивного Employee.
3. Несколько Run одного Employee; waiting Task при ещё running/stopping Run.
4. Все literal desired/observed states, в том числе `stop_requested + running`
   и `force_stop_requested + unknown`, без автоматической смены Task lifecycle.
5. Числовые границы u16/u32/safe integer и нулевой sequence; отрицательные,
   дробные, бесконечные и unsafe значения отклоняются.
6. Обязательный null отличается от omission; конечный cursor отсутствует,
   пагинация не добавляет null/defaults.
7. Пустая, частичная и заполненная диагностика; incomplete streams; unknown
   measurements остаются null. Nested JSON и специальные ключи не теряются.
8. Parsing не меняет входы, не создаёт missing metadata и не выдаёт JSON за
   разрешение открыть файл, источник или доверенную конфигурацию.
9. Прежние 29 tests сохраняются, новые проходят; typecheck/lint/build без
   новых проблем; spec и quality reviews завершены без открытых findings.

## Порядок и проверка

Один implementer владеет contracts/tests; root — документацией и финальными
проверками. Сначала негативные тесты, затем schemas; spec и quality reviews
проводят независимые исполнители. Findings исправляются до завершения Task.

Из корня последовательно запустить `just ui-test-contracts`, `just ui-typecheck`,
`just ui-lint`, `just ui-build`. Каждую проверку ограничить:

```sh
systemd-run --user --scope --quiet \
  -p MemoryMax=6G -p MemorySwapMax=1G -p CPUQuota=200% \
  timeout 300s just ui-test-contracts
```

Менять только recipe для остальных проверок. Дополнительно: `git diff --check`,
`just --unstable --fmt --check`, local Markdown links и diff/ignore audit.
Не запускать одновременно проверки в общей копии или Rust builds.

## Evidence

Проверено 17 сентября 2026 на Linux, Bun `1.3.11`, Node `24.14.0`.
Implementer сначала зафиксировал red phase: прежние 29 tests проходят, три новых
test modules падают на отсутствующих exports. Затем добавлены три schema-модуля,
три test-модуля и один fixture-модуль; production barrel получил три exports.

Root повторил все проверки после реализации, последовательно в scopes с
лимитами выше. Browser/React harness не добавлялся.

| Проверка | Результат |
|---|---|
| `just ui-test-contracts` | PASS: 47 tests (29 прежних + 18 новых), 0 failures/skips; около 1.267 s |
| `just ui-typecheck` | PASS, exit 0 |
| `just ui-lint` | PASS, 0 errors; те же 10 прежних react-refresh warnings |
| `just ui-build` | PASS, client/SSR/Nitro; существующий `cloudflare-module` preset |
| `git diff --check`, `just --unstable --fmt --check` | PASS |
| Local Markdown paths | PASS: 127 ссылок в изменённых документах; проверены файлы назначения |
| Scope/dependencies/lockfile audit | PASS: 15 файлов, из них 7 новых contract files; dependencies и lockfile без изменений |
| Quality review | APPROVE; открытых findings нет |
| Spec review | APPROVE; открытых findings нет |

TaskStage fixture использует текущую версию RunSpec 6, остальные purpose cases —
3/4/5/7. Это реалистичные synthetic примеры, а не новое ограничение схемы:
положительный u16 вне текущего набора версий тоже принимается.

SHA-256 `bun.lock` до/после:
`fc00072f27cf820a6b27ab8c9de583f0c508ff16aaa40ddb403271cc65991935`.
Backend/OpenAPI/PRD, Justfile/package manifest и прежние source files не менялись,
кроме добавления exports в contracts barrel. Generated output остаётся ignored;
route manifest не изменился. Прежнее build-предупреждение `vite-tsconfig-paths`
сохраняется; новый слой не требует изменения wrapper или dependency graph.

Unit tests используют synthetic fixtures, а не ответы запущенного Core; они не
доказывают live integration, server-side redaction или безопасный browser access.
Экраны остаются на demo. Browser smoke, Core/services/provider Runs и Rust checks
не запускались по границам задачи. Commit/push не выполнялись. UI0.1 остаётся
открытым; read gaps и дальнейшие работы перечислены ниже и в field/gap map.

## Оставшиеся границы

UI0.2 владеет синхронизацией OpenAPI и browser/API boundary. Employee catalog,
runtime profile, generic WorkSurface и Run metadata остаются named read gaps.
Детализация diagnostics/evidence относится к UI2.1/UI1.4. Полные каталоги Project,
presentation adapters и подключение экранов остаются следующими Task UI0/UI1.
