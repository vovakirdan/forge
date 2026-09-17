# Forge: Task index

## Текущая очередь

| Task | Epic | Приоритет | Статус | Зависимости | Результат |
|---|---|---|---|---|---|
| [FRONTEND-001](frontend/frontend-001-local-demo-baseline.md) | UI0.3 | P0 | done | Импортированный frontend; Linux, Bun 1.3.11, Node 24.14.0 | Воспроизводимый локальный demo; evidence в Task |
| [FRONTEND-002](frontend/frontend-002-task-pipeline-contracts.md) | UI0.1 | P0 | done | FRONTEND-001; действующие Core read DTO | Изолированные Task/Pipeline contracts, 29 tests; без изменения экранов |
| [FRONTEND-003](frontend/frontend-003-run-read-contracts.md) | UI0.1 | P0 | done | FRONTEND-002; Run API и diagnostics projection | Run ownership/states и diagnostics; все 47 contract tests проходят, экраны без изменений |
| [FRONTEND-004](frontend/frontend-004-browser-smoke.md) | UI0.3 | P0 | done | FRONTEND-001; Linux, Bun/Node и Chromium | 7 browser tests, два root прогона; error/timeout/occupied-port probes; mock-only UI |
| [FRONTEND-005](frontend/frontend-005-task-run-presentation.md) | UI0.1 | P0 | done | FRONTEND-002/003 | Pure Task/Run presentation; 18 tests, source DTO и независимые состояния, без подключения экранов |
| [FRONTEND-006](frontend/frontend-006-static-hosting-boundary.md) | UI0.2 | P0 | done | FRONTEND-001–005; ранний preparatory-срез согласован | Static proof: 2 Node + 13 browser tests; принятый ADR, без gateway/auth/Core connection |
| [FRONTEND-007](frontend/frontend-007-live-owner-gateway.md) | UI0.2 | P0 | done | FRONTEND-001–006; ранний live-read срез согласован | Owner gateway/session, настоящий Core Project read; 7 live browser tests, sandbox/CSP/secret/cleanup gates, evidence в Task |
| [FRONTEND-008](frontend/frontend-008-live-task-reads.md) | UI1.3 / UI0.2 | P0 | done | FRONTEND-002/005/007; ранний read-only срез согласован | Реальный список/карточка Task и pinned Pipeline stage; 22 live browser tests, 29 live unit tests, evidence в Task |
| [FRONTEND-009](frontend/frontend-009-live-run-reads.md) | UI2.1 / UI0.2 | P0 | done | FRONTEND-003/005/008; ранний read-only срез согласован | Project Runs: список/карточка, независимые состояния и diagnostic availability; 35 live browser / 36 live unit tests, evidence в Task |
| [FRONTEND-010](frontend/frontend-010-live-pipeline-reads.md) | UI1.2 / UI0.2 | P0 | done | FRONTEND-002/008/009; ранний read-only срез согласован | Версии Pipeline и typed stage inspector; 51 live browser / 42 live unit tests, evidence в Task |

## Порядок и параллельность

Baseline локального запуска FRONTEND-001 проверен. FRONTEND-002 завершила первый
read-contract срез [UI0.1](../docs/epics/ui0-e1-domain-contracts.md), не весь epic.
FRONTEND-003 добавила контракты Run и внешней структуры diagnostics.
FRONTEND-004 добавляет browser smoke [UI0.3](../docs/epics/ui0-e3-frontend-tooling.md).
FRONTEND-005 добавляет presentation поверх проверенных Task/Run DTO.
Далее отдельно нарезаются оставшиеся contracts/fixtures UI0.1,
component harness, CI и оставшиеся состояния UI0.3.
FRONTEND-006 начинает [UI0.2](../docs/epics/ui0-e2-browser-api.md) с ADR/static
proof до полного закрытия обоих эпиков по явному согласованию пользователя.
FRONTEND-007 реализует gateway/session и первое настоящее Project read в отдельном
небольшом экране. Ранний live-read срез также согласован; полные epic gates
сохраняются, demo Board/Team ещё не подключены к Core.
FRONTEND-008 расширяет отдельный live экран списком/карточкой Task. Это ранний
read-only срез UI1.3, не подключение mock Board и не закрытие всего UI0/UI1.
FRONTEND-009 добавляет Project-wide Runs и ограниченную диагностическую сводку.
Это ранний срез UI2.1 без Activity/SSE, evidence viewer и управляющих команд.
FRONTEND-010 добавляет просмотр версий Pipeline и stage contracts из UI1.2;
без редактора, publication/default/delete и запуска hooks.
Это порядок эпиков, не заранее созданные дополнительные Task.

У FRONTEND-001 нет Task dependencies. Проверки install/build/dev выполняются
последовательно: общий `node_modules`, generated routes и порт 5173 исключают
параллельную валидацию в той же копии. То же ограничение действует для
FRONTEND-002/003/004/005/006/007/008/009/010: install, browser/static/live/contract/presentation tests,
typecheck, lint и build запускаются по очереди. Browser harness владеет
портом 4173; static harness — портом 4174;
ручной demo на 5173 не используется тестами.
Документацию можно готовить отдельно,
не меняя файлы исполнителя. EXT milestones не блокируют эту задачу.
