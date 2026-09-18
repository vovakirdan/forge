# FRONTEND-017 — Чтение зависимостей и переходы между Task

**Статус:** done
**Epic:** UI1.3 / UI0.2; contracts UI0.1
**Приоритет:** P0
**Зависимости:** FRONTEND-008, FRONTEND-016; read-only срез согласован.

## Результат и границы

Карточка показывает «Зависит от» (`blocked_by`) и «Блокирует» (`blocks`).
Человек открывает связанную Task, даже если её нет на текущей странице списка,
и возвращается к предыдущей карточке. Обе стороны читаются из одной canonical
связи, а не из текстов wait conditions.

Создание и удаление связей — отдельный следующий срез. Граф-визуализация,
mock Board, stage/artifact conditions, SSE, новые URL-маршруты и изменение
правил исполнения не входят. Публичная модель не обещает пока отсутствующие
dependency ID, audit/history или gate stage.

## Контракт

- `GET /v1/projects/{project_id}/tasks/{task_id}/dependencies/{direction}`;
  browser route с `/api`. Направление: `blocked_by` либо `blocks`.
- Страница: `items`, optional `next_cursor`; default limit 20, maximum 100.
  Направления имеют независимую пагинацию, порядок по ID связанной Task.
  Cursor привязан к Project, Task и направлению; удалённый cursor требует
  сброса страницы, не тихого показа первой страницы.
  Gateway ограничивает ответ 64 KiB; превышение даёт явную size error,
  а не усечённый или пустой список.
- Элемент: `blocker_task_id`, `blocked_task_id`, `required_condition: task_done`,
  `related_task: {id, key, title, lifecycle}` и `condition_state`.
- Core вычисляет `condition_state`: `pending`, `satisfied` или
  `blocker_cancelled`. `done` удовлетворяет условию, `cancelled` — нет.
  Выполненное условие не доказывает готовность зависимой Task к запуску.
- Read использует согласованный snapshot связи и Task, проверяет обе Project
  boundaries и не меняет revisions, Events, waits или очередь.
  PostgreSQL использует read-only repeatable-read transaction и SQL limit + 1:
  всё дерево зависимостей и отдельные Task через N+1 запросы не загружаются.

## Поведение интерфейса

Два read panel показывают ключ, название, lifecycle связанной Task и оценку
условия. Выполненные связи остаются видимыми, пока они существуют в Core.
Loading, empty, error/retry и stale различаются; сбой dependencies не скрывает
основную карточку. Обновление явное, без polling.

Переходы используют существующий Task detail и leave guard, сохраняют исходную
страницу списка. Локальная история даёт возврат к предыдущей Task; закрытие
возвращает фокус к исходному opener, если он ещё доступен. Project/session change
очищает историю и изолирует запросы. После accepted cancellation dependency reads
текущего Project инвалидируются, даже если readback карточки завершился ошибкой.

## Приёмка

1. Rust/storage/HTTP: обе стороны, несколько blockers, empty и pagination,
   scoped/removed cursor, отсутствующая или чужая Task, три condition states.
2. GET не меняет Project/Task revisions или Events. Fixture создаётся commands,
   без прямых DB writes, Employees, provider Runs или открытия execution gate.
3. Browser через настоящий Core: переход/возврат, target вне страницы списка,
   focus, независимые страницы, error/retry/stale/refresh, отменённый blocker,
   scope/logout/late responses, leave guard, keyboard/narrow layout.
4. Последовательные Rust/frontend/live/static/demo/security проверки с лимитами
   памяти; независимые spec/quality reviews и устранение findings.

## Evidence

Приёмка 18 сентября 2026:

- Frontend typecheck, contracts и live unit — PASS (65 и 84 tests).
- Core/storage/UI unit run и финальный повтор — PASS (Core 81, UI 79).
  Workspace Clippy all targets/all features с `-D warnings` — PASS.
- Реальный PostgreSQL HTTP contract — PASS 1: обе стороны и 23 blockers,
  три condition states, scope/удалённый cursor; raw snapshots, revisions и
  Events неизменны после GET. Отдельная команда удаления нужна только для
  проверки устаревшего cursor, browser mutation route не открыт.
- Независимое quality review backend/browser и frontend — APPROVE.
- Полный browser/security повтор — PASS: 142/142 browser tests, включая
  восемь новых; egress 2 и реальный Podman sandbox 1. Secret scan: 476 значений
  в основном прогоне, 480 после intentional failure probe — без утечек.
  Probe: одна ожидаемая ошибка, ноль неожиданных.
- Typecheck, lint (0 errors, 10 прежних warnings), presentation 23 и обе
  demo/static сборки — PASS; static Node 2, static browser 13 и demo browser 7 — PASS.
- `cargo fmt`, `git diff --check`, 136 локальных Markdown targets и лимит
  500 effective LOC для изменённых/новых исходников — PASS.

При разработке typecheck обнаружил несовместимость optional callback с
`exactOptionalPropertyTypes`; тип исправлен, повтор прошёл. Первый PG-запуск
остановил integration guard: не был задан `FORGE_INTEGRATION=1`. Повтор с явным
флагом прошёл; также исправлен тестовый payload удаления связи, не принимающий
поле condition. Неоднозначное «missing cursor» в OpenAPI заменено на «removed»:
отсутствующий optional cursor означает первую страницу.

Первый полный browser run остановился на startup fixture: readiness JSON вырос
до 12 934 bytes при неизменном лимите 8 KiB. Ответ сокращён до используемых
координат (7 810 bytes), все 23 + 23 связи сохранены. Producer теперь проверяет
лимит до отправки; лимиты и таймауты browser runner не повышались. Независимое
ревью этого изменения — APPROVE; повтор полного прогона прошёл.

FRONTEND-016 опубликована отдельным коммитом `eb750e0`; этот срез остаётся
локальным до следующего запроса публикации. Полные UI epics остаются открытыми.
