# Forge: Task index

## Текущая очередь

| Task | Epic | Приоритет | Статус | Зависимости | Результат |
|---|---|---|---|---|---|
| [FRONTEND-001](frontend/frontend-001-local-demo-baseline.md) | UI0.3 | P0 | done | Импортированный frontend; Linux, Bun 1.3.11, Node 24.14.0 | Воспроизводимый локальный demo; evidence в Task |
| [FRONTEND-002](frontend/frontend-002-task-pipeline-contracts.md) | UI0.1 | P0 | done | FRONTEND-001; действующие Core read DTO | Изолированные Task/Pipeline contracts, 29 tests; без изменения экранов |
| [FRONTEND-003](frontend/frontend-003-run-read-contracts.md) | UI0.1 | P0 | done | FRONTEND-002; Run API и diagnostics projection | Run ownership/states и diagnostics; все 47 contract tests проходят, экраны без изменений |
| [FRONTEND-004](frontend/frontend-004-browser-smoke.md) | UI0.3 | P0 | done | FRONTEND-001; Linux, Bun/Node и Chromium | 7 browser tests, два root прогона; error/timeout/occupied-port probes; mock-only UI |

## Порядок и параллельность

Baseline локального запуска FRONTEND-001 проверен. FRONTEND-002 завершила первый
read-contract срез [UI0.1](../docs/epics/ui0-e1-domain-contracts.md), не весь epic.
FRONTEND-003 добавила контракты Run и внешней структуры diagnostics.
FRONTEND-004 добавляет browser smoke [UI0.3](../docs/epics/ui0-e3-frontend-tooling.md).
Далее отдельно нарезаются оставшиеся contracts/fixtures UI0.1,
component harness, CI и оставшиеся состояния UI0.3.
[UI0.2](../docs/epics/ui0-e2-browser-api.md) ждёт обоих эпиков.
Это порядок эпиков, не заранее созданные дополнительные Task.

У FRONTEND-001 нет Task dependencies. Проверки install/build/dev выполняются
последовательно: общий `node_modules`, generated routes и порт 5173 исключают
параллельную валидацию в той же копии. То же ограничение действует для
FRONTEND-002/003/004: install, browser/contract tests, typecheck, lint и build
запускаются по очереди. Browser harness владеет отдельным портом 4173;
ручной demo на 5173 не используется тестами.
Документацию можно готовить отдельно,
не меняя файлы исполнителя. EXT milestones не блокируют эту задачу.
