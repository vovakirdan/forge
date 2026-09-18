# FRONTEND-013 — Изменение приоритета Task

**Статус:** done
**Epic:** UI1.3 / UI0.2; contracts UI0.1
**Приоритет:** P0
**Зависимости:** FRONTEND-011/012; command-срез согласован отдельно.

## Результат

Live-карточка меняет приоритет существующей non-terminal Task через
`set_task_priority`. Каталог проекта задаёт уровни; браузер не создаёт свои
priority enum, сортировку очереди или изменения lifecycle.

## Контракт и поведение

- Allowlisted `POST /api/commands/set_task_priority` передаёт существующую
  команду Core `/v1/commands/set_task_priority`. Envelope: `project_id`,
  `expected_revision`; payload: `task_id`, `expected_task_revision`, `priority`;
  `Idempotency-Key` отдельно. Actor браузер не задаёт.
- Сохраняются owner/session/Host/Origin, strict JSON, UUIDv7, safe revisions,
  bounded body/receipt и timeout. Общие helpers обслуживают только две явно
  разрешённые команды, не произвольный Core proxy.
- Редактор получает свежие Task и PriorityScheme. Project revision берётся
  из каталога; Task revision — из Task. Одновременно открыт только редактор
  текста draft или приоритета.
- Назначать можно только active known level для draft/ready/in_progress/waiting.
  Terminal Task read-only. Retired/unknown текущий ID виден, default не подставляется.
  Неизменённое значение не отправляется; недоступный каталог блокирует save,
  но не чтение Task.
- Conflict сохраняет выбор, требует явного refresh baseline и отдельного Save.
  Исчезнувший/retired выбор остаётся видимым до нового допустимого выбора.
- Неопределённый исход сохраняет исходные body/key для exact retry. Успех
  подтверждается receipt, не совпадением прочитанного значения. Ошибка refresh
  после receipt не отменяет запись и не вызывает повтор команды.
- После receipt обновляются Project, Task, список и каталог. Scope/session
  cleanup, late responses и предупреждение о несохранённом вводе сохраняются.

Core не прерывает существующий Run/lease, но обновляет подходящую ожидающую
работу. Общий post-commit dispatch остаётся; нельзя обещать отсутствие новых
Runs на открытом исполняющем проекте. Приёмка использует отдельный stopped
Project без Employees и платных providers.

Не входят: настройка схемы, создание/approval/cancel Task, property editor,
Board/SSE, preemption, новые доменные правила, миграции и зависимости.

## Приёмка

- Gateway/contracts: allowlist, shapes, auth/origin/scope, bounds, receipt,
  explicit refusal versus unknown outcome; regression amend_draft.
- Core: lifecycle/active levels, revisions/event, queue projection и сохранение
  активного Run/lease. Никаких прямых SQL mutations для browser fixtures.
- Browser real Core: save/reload, unchanged, two tabs/draft conflict, frozen replay,
  terminal/cross-project refusals, failed refresh, navigation/logout и keyboard.
- Synthetic cases явно отделены: custom/retired catalogs, malformed receipts,
  unavailable and late responses.
- Rust fmt/clippy/tests, frontend types/lint/unit/build и browser/security gates;
  тяжёлые проверки последовательно, 6 GiB RAM / 1 GiB swap / CPU 200%, Cargo jobs 2.
- Независимые spec/security и quality reviews; findings исправляются до done.

## Evidence

Приёмка 2026-09-18 завершена; commit/push не входят в этот этап.

- Rust: `cargo test --locked -p forge-ui -p forge-domain -p forge-core --lib` —
  262 passed. Command conformance — 51 passed, 60 integration tests явно ignored;
  отдельный PostgreSQL priority-прогон — 3 passed, включая lifecycle и active Run.
- `cargo fmt --all -- --check`, workspace clippy со всеми targets/features и
  `-D warnings`, `git diff --check` — PASS.
- Frontend: typecheck PASS; lint — 0 errors, 10 прежних react-refresh warnings;
  contracts 57, live unit 62, presentation 23 — PASS.
- Финальный `just ui-test-live` — 86/86 browser tests PASS, включая 13 новых;
  2 Core egress tests и 1 настоящий rootless Podman sandbox test — PASS.
  Secret scan: 232 issued values в основном наборе и 236 после intentional
  failure probe, утечек нет; probe — 1 expected failure, 0 unexpected.
- Live/demo/static builds — PASS; static — 2 Node + 13 browser tests,
  demo — 7 browser tests. Paid providers не запускались.
- Независимые spec и quality reviews — APPROVE после исправлений.

При проверке исправлены устаревшая подпись сохранённого приоритета и сброс
выбранного Low после 409 в двух вкладках. Первые два browser-прогона дали
85/86: отмена native reset через React `onReset` оказалась недостаточной.
Финальный явный `onSubmit` с `preventDefault` и `startTransition(save)`
сохраняет `useActionState`, но не запускает автоматический host-form reset.
Regression assertion на сохранение Low не ослаблена; повторный полный прогон
86/86 подтвердил исправление. Unknown/refused состояния показывают последнее
загруженное значение, receipt и ошибки последующего чтения разделены.

UI1.3/UI0.2 целиком не закрыты: этот Task реализует только согласованный
priority command-срез, без создания Task и настройки схемы.
