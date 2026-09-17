# Forge: Task index

## Текущая очередь

| Task | Epic | Приоритет | Статус | Зависимости | Результат |
|---|---|---|---|---|---|
| [FRONTEND-001](frontend/frontend-001-local-demo-baseline.md) | UI0.3 | P0 | done | Импортированный frontend; Linux, Bun 1.3.11, Node 24.14.0 | Воспроизводимый локальный demo; evidence в Task |
| [FRONTEND-002](frontend/frontend-002-task-pipeline-contracts.md) | UI0.1 | P0 | done | FRONTEND-001; действующие Core read DTO | Изолированные Task/Pipeline contracts, 29 tests; без изменения экранов |

## Порядок и параллельность

Baseline локального запуска FRONTEND-001 проверен. FRONTEND-002 завершила первый
read-contract срез [UI0.1](../docs/epics/ui0-e1-domain-contracts.md), не весь epic.
Далее отдельно нарезаются
оставшиеся contracts/fixtures UI0.1 и test harness
[UI0.3](../docs/epics/ui0-e3-frontend-tooling.md).
[UI0.2](../docs/epics/ui0-e2-browser-api.md) ждёт обоих эпиков.
Это порядок эпиков, не заранее созданные дополнительные Task.

У FRONTEND-001 нет Task dependencies. Проверки install/build/dev выполняются
последовательно: общий `node_modules`, generated routes и порт 5173 исключают
параллельную валидацию в той же копии. То же ограничение действует для
FRONTEND-002: contract tests, typecheck, lint и build запускаются по очереди.
Документацию можно готовить отдельно,
не меняя файлы исполнителя. EXT milestones не блокируют эту задачу.
