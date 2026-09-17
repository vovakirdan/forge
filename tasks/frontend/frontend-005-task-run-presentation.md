# FRONTEND-005 — Pure Task/Run presentation

**Epic:** [UI0.1](../../docs/epics/ui0-e1-domain-contracts.md)
**Статус:** done — presentation-срез; UI0.1 остаётся in_progress
**Приоритет:** P0
**Дата:** 17 сентября 2026
**Зависимости:** [FRONTEND-002](frontend-002-task-pipeline-contracts.md),
[FRONTEND-003](frontend-003-run-read-contracts.md) — done
**Baseline:** `5587f08`; FRONTEND-004 опубликована отдельно.

## Цель и границы

Добавить в `frontend/src/presentation/` небольшие чистые TypeScript-функции
для отображения уже проверенных Core DTO. Результат каждой функции содержит
`source` — исходный DTO — и `presentation` — подписи и производные факты.
Это не новый wire contract, HTTP client или второй источник domain state.

Источники: [карта полей](../../docs/UI_BACKEND_ALIGNMENT.md),
[UI plan](../../docs/UI_IMPLEMENTATION_PLAN.md),
[правила](../../docs/PROJECT_RULES.md), [frontend README](../../frontend/README.md).

## Согласованный контракт

| Функция | Производные данные |
|---|---|
| `presentTaskSummary(task, pipeline?)` | Подписи kind/lifecycle; стадия через существующий `resolveTaskStage` по pinned version |
| `presentTaskDetail(task, pipeline?)` | Представление summary и количество загруженных waits/artifacts |
| `presentRun(run)` | Независимые подписи purpose, desired state, observed state; kind для SystemJob |
| `presentRunDiagnostics(diagnostics)` | Наличие nullable reports, `loadedIncidentCount`, `loadedEvidenceCount`, факты неполных загруженных streams |

- Английские подписи соответствуют языку текущего UI. Закрытые enum требуют
  exhaustive handling. Цвета, React, i18n и зависимости не добавляются.
- `source` сохраняет исходные данные и дополнительные JSON fields без мутации.
  Owner остаётся типизированным `source.assignment`, без копии discriminated union.
- `no_stage` отличается от `unavailable`; причины недоступности сохраняются.
  Новая default version и soft-delete не меняют закреплённую стадию Task.
- Priority остаётся stable ID в `source`: нет выдуманного rank, цвета или каталога.
  Причина отмены не извлекается из произвольного property или wait.
- Task lifecycle не выводится из Run. Запрос остановки не означает физическую
  остановку. Нет joins, выбора current Run, Employee aggregation или постоянного
  Task assignee; nullable Employee ID не превращается в профиль или диагноз.
- Счётчики отражают только возвращённые массивы, не полный объём хранения.
  Task summary без detail не получает нулевые counts.
- `null` report отсутствует, `{}` присутствует; наличие не доказывает качество,
  acceptance или успех. JSON не интерпретируется, ссылки не открываются,
  команды и сетевые вызовы не выполняются.

## Acceptance и проверки

1. Отдельная команда `just ui-test-presentation` использует встроенный `node:test`,
   relative `.ts` imports и последовательное выполнение без нового runner.
2. Тесты покрывают оба Task kind, все lifecycle, обе оси Run state, пять purposes
   и два SystemJob kind; проверяют подписи, а не повторяют schema validation.
3. Waiting Task остаётся waiting при running/stopping Run. Несколько Runs
   одного Employee представлены отдельно без invented current Run.
4. Покрыты все stage resolution outcomes, старый pin при смене default и
   soft-delete, analysis без Git, произвольный priority, multiple waits,
   отсутствие cancellation reason и detail-only counts в summary.
5. Проверены null/empty/populated diagnostics, неполные streams, additive JSON,
   отсутствие мутаций и побочных действий.
6. Последовательно проходят presentation tests, прежние contract tests,
   browser smoke, typecheck, lint, build и `git diff --check`.
7. Независимые spec и quality reviews не оставляют открытых findings.

Ресурсный предел каждой проверки: `systemd-run --user --scope --quiet`
с `MemoryMax=6G`, `MemorySwapMax=1G`, `CPUQuota=200%` и `timeout 300s`.
Не запускать сборки или browser suite одновременно; Core/services/provider
Runs для этой задачи не нужны.

## Evidence

Проверено 17 сентября 2026; все команды выполнялись по очереди с указанными
выше лимитами. Synthetic unit tests доказывают поведение presentation-функций,
не live Core integration.

| Проверка | Результат |
|---|---|
| `just ui-test-presentation` | PASS: 18/18, итоговый root run около 468 ms |
| `just ui-test-contracts` | PASS: прежние 47/47, около 1.188 s |
| `just ui-typecheck` | PASS после исправлений strict typing в тестах |
| `just ui-lint` | PASS: 0 errors, прежние 10 react-refresh warnings |
| `just ui-test-browser` | PASS: 7/7, 35.4 s, один Chromium worker, без retries |
| `just ui-build` | PASS: client/SSR/Nitro, прежний hosting target |
| Diff/Justfile formatting/ignore audit | PASS; build/browser output не попадает в Git |
| Local Markdown links | PASS: 131 локальная ссылка в изменённых документах |
| Независимые spec/quality reviews | APPROVE, открытых findings нет |

Тесты сначала завершились RED из-за отсутствующих implementation modules,
затем прошли после реализации. Первый root typecheck обнаружил четыре ошибки
только в тестах: доступ к additive fields через index signature и возможное
отсутствие первого artifact. Исправлены bracket access и явная assertion;
настройки TypeScript не ослаблялись. После этого typecheck и все 18 тестов
прошли повторно. Production-функции не потребовали исправлений по review.

Generic signatures сохраняют тип конкретного DTO, включая diagnostics у
RunDetailView. Frozen fixtures проверяют отсутствие мутации, отдельный fetch
guard — отсутствие загрузки opaque diagnostics. Misleading property с именем
`cancellation_reason` остаётся данными проекта, не причиной отмены в presentation.

Dependencies и lockfile не менялись. SHA-256 `bun.lock` до/после проверок:
`8b6a18718bad0635936d65b8ce39ba06a211be2595d3e7323cfb74a982aa0b6d`.
Contracts, demo services, components/routes, API, backend и PRD не изменены.
Core services, Rust validation и provider Runs не запускались: этот срез меняет
только frontend presentation и документацию. Известные browser/build warnings
и ранний SSR-click risk остаются из FRONTEND-004; они не исправлялись этой задачей.

## Вне границ и дальнейшая работа

Экраны, mock services, legacy types, API/OpenAPI/backend/PRD не меняются.
Нет Employee/Surface reads, подробных diagnostics schemas, каталога приоритетов,
команд, component runner, CI или исправления раннего SSR-взаимодействия.
UI0.1 и UI0.3 остаются открытыми. FRONTEND-005 не включается в коммит FRONTEND-004;
публикация новой задачи требует отдельного запроса.
