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
| [FRONTEND-011](frontend/frontend-011-draft-task-edit.md) | UI0.2 / UI1.3 | P0 | done | FRONTEND-007–010; первый command-срез согласован | Название/описание draft через Core, конфликты и безопасный retry; 61 live browser / 51 live unit tests, evidence в Task |
| [FRONTEND-012](frontend/frontend-012-project-priorities.md) | UI0.1 / UI1.1 / UI1.3 | P0 | done | FRONTEND-008/011; read-срез согласован | Canonical PriorityScheme read и названия приоритетов в Task; 73 live browser / 55 live unit tests, evidence в Task |
| [FRONTEND-013](frontend/frontend-013-task-priority-edit.md) | UI1.3 / UI0.2 | P0 | done | FRONTEND-011/012; command-срез согласован | Изменение приоритета через Core; conflict/exact retry, 86 live browser tests; evidence в Task |
| [FRONTEND-014](frontend/frontend-014-create-draft-task.md) | UI1.3 / UI0.2 | P0 | done | FRONTEND-010–013; command-срез согласован | Создание draft с точным Pipeline pin и безопасным replay; 104 live browser tests, evidence в Task |
| [FRONTEND-015](frontend/frontend-015-draft-dod-approval.md) | UI1.3 / UI0.2 | P0 | done | FRONTEND-011–014; command-срез согласован | DoD edit и явный approval с consent/retry/readback; 121 live browser tests, evidence в Task |
| [FRONTEND-016](frontend/frontend-016-task-cancellation.md) | UI1.3 / UI0.2 | P0 | done | FRONTEND-008, FRONTEND-011–015; command-срез согласован | Каталог причин, явная отмена и readback; 134 live browser tests PASS |

## Порядок и параллельность

FRONTEND-001–016 завершены в своих согласованных границах. FRONTEND-012 добавила
чтение приоритетов проекта перед будущим созданием Task; существующие
milestone/epic gates сохраняются. FRONTEND-013 добавляет изменение приоритета
существующей Task через `set_task_priority`. FRONTEND-014 добавляет создание
черновика без approval/исполнения и без property editor. FRONTEND-015 добавляет
DoD edit и отдельный approval сохранённого черновика; property editor не входит.
FRONTEND-016 закрыта: отмена Task с каталогом причин и authoritative readback.
Настройка каталога и execution controls остаются вне этого среза.

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
FRONTEND-011 добавляет первую command из UI0.2/UI1.3: редактирование
названия/описания существующей draft Task с receipt, conflict и manual retry.
CreateTask, прочие изменения Task и подключение mock Board сюда не входят.
FRONTEND-012 закрывает priority-часть gap `project-task-catalogs`: отдельный
read и названия в Task. Настройка схемы, properties и cancellation catalogs
остаются следующими шагами; synthetic custom схемы не означают их реализацию.
FRONTEND-013 использует этот каталог для выбора active priority и добавляет
вторую browser command. Создание Task, настройка схемы и preemption не входят.
Это порядок эпиков, не заранее созданные дополнительные Task.

У FRONTEND-001 нет Task dependencies. Проверки install/build/dev выполняются
последовательно: общий `node_modules`, generated routes и порт 5173 исключают
параллельную валидацию в той же копии. То же ограничение действует для
FRONTEND-002/003/004/005/006/007/008/009/010/011/012/013/014/015/016: install, browser/static/live/contract/presentation tests,
typecheck, lint и build запускаются по очереди. Browser harness владеет
портом 4173; static harness — портом 4174;
ручной demo на 5173 не используется тестами.
Документацию можно готовить отдельно,
не меняя файлы исполнителя. EXT milestones не блокируют эту задачу.
