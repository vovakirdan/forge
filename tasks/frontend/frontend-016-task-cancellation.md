# FRONTEND-016 — Отмена Task с причиной

**Статус:** done
**Epic:** UI1.3 / UI0.2; contracts UI0.1
**Приоритет:** P0
**Зависимости:** FRONTEND-008, FRONTEND-011–015; command-срез согласован.

## Результат и границы

Человек явно выбирает причину и подтверждает отмену нетерминальной Task.
Core применяет существующую команду, а карточка показывает сохранённые данные
отмены. Отмена запрашивает graceful stop активных Runs и блокирует поздний
outcome; принятый receipt не доказывает физическую остановку executor.

Настройка каталога, stop/resume/force-stop, physical reconciliation, property
editor, dependency editor и подключение mock Board не входят. Новых доменных
правил, provider integrations или миграций не требуется.

## Контракт

- `GET /v1/projects/{project_id}/cancellation-reasons` и browser route с `/api`
  возвращают `{project_id, project_revision, reasons: [{id, display_name, retired}]}`.
  IDs стабильны, причины принадлежат Project, retired сохраняются для истории.
- Task detail получает nullable `cancellation`: `reason_id`, `note`,
  `cancelled_by` (ActorReference), `cancelled_at` (RFC3339).
- `POST /api/commands/cancel_task`: envelope `project_id`, `expected_revision`,
  payload `task_id`, `expected_task_revision`, `cancellation_reason_key`, optional
  `note`; `Idempotency-Key` в заголовке. Wire key соответствует существующему
  Core command, а не переименовывает доменный reason ID.
- Owner/session, Host/Origin, строгий payload, bounds и receipt identity остаются
  обязательными. Только cancel receipt допускает Project revision больше
  исходной без требования ровно +1: Core может обновить зависимые ожидания.
- Форма читает свежие Project, Task и каталог, требует активную причину и
  отдельное подтверждение. Даже `unspecified` не выбирается автоматически.
  Комментарий необязателен; browser boundary ограничивает его 20 000 Unicode
  scalar values. Lifecycle/stage UI не вычисляет самостоятельно.
- Conflict refresh сохраняет ввод, заново проверяет доступность причины и
  сбрасывает consent. Unknown outcome разрешает только exact body/key retry.
  Принятый receipt отделён от readback: повтор обновления делает только GET.
- Карточка показывает причину, комментарий, автора и время; недоступный каталог
  не скрывает сохранённый reason ID. Scope/logout/leave guards и защита от
  поздних ответов сохраняются; одновременно открыт один command panel.

## Проверки

1. Rust read/command contracts: scope, каталог, nullable metadata, strict payload,
   limits, receipt identity и revision, active/retired/unknown причины.
2. Core conformance: replay без повторных эффектов, terminal/conflict refusals,
   активный Run и late-outcome fencing, зависимые ожидания.
3. Browser через настоящий Core: явный выбор, optional note, readback, conflict,
   unknown/malformed response, GET failure, scope/logout/late response,
   недоступный каталог, keyboard и narrow layout.
4. Последовательная ограниченная приёмка Rust/frontend/live/static/demo/security;
   независимые spec и quality reviews. Без платных провайдеров и credentials.

## Evidence

Приёмка 18 сентября 2026 завершена:

- Rust: Core 81, Domain 119, UI 78 unit tests — PASS (278).
- Workspace clippy (`--all-targets --all-features --locked`, `-D warnings`),
  rustfmt, scoped Prettier, frontend typecheck — PASS. Lint: 0 errors,
  10 прежних react-refresh warnings.
- Frontend contracts 63, live unit 79, presentation 23 — PASS. Demo/static
  builds — PASS; static Node 2, static browser 13, demo browser 7 — PASS.
- Command conformance: 53 passed, 63 integration tests явно ignored в общем
  прогоне. Отдельно PostgreSQL catalog/readback — PASS 1; active Run ownership,
  rollback неизвестной причины, audit/replay — PASS на memory и PostgreSQL (2).
- PostgreSQL/NATS + manual Supervisor gRPC: отменённая Task отклоняет late
  completed outcome, сохраняет accepted Artifact; desired `stop_requested`
  не подменяет observed `running` — PASS 1. Это keyless proof, не provider Run.
- Финальный `just ui-test-live`: 134/134 browser tests — PASS (13 новых),
  Core egress 2 и настоящий rootless Podman isolation test 1 — PASS.
  Secret scan: 454 issued values, утечек нет; после intentional failure probe —
  458, утечек нет. Probe дал 1 expected failure и 0 unexpected.
- Независимые spec и quality reviews — APPROVE; отдельное повторное ревью
  исправлений тестов также APPROVE. Реализацию каждый reviewer проверял в
  файлах другого исполнителя.

Выявленные при проверке случаи:

- Старый gateway denylist-test ожидал, что `cancel_task` недоступна. После
  открытия route проверка запрещённой команды использует `resume_task`.
- Late-outcome regression ошибочно ожидал пустой список Artifact после ранее
  принятого evidence. Проверка исправлена: immutable Artifact и Task/Run
  attachment сохраняются, новая работа после отмены не принимается.
- Первый browser-прогон остановился по общему лимиту: 14 passed, 5 failed,
  115 did not run. Exact label locator не находил вложенный select; он заменён
  scoped accessible-role locator. Domain отказ неизвестной причины/terminal
  Task имеет 422, не wire-shape 400; проверяется точный `validation_failed`.
- Второй прогон: 133/134. Аналогичный locator заполненного textarea после
  conflict не находил поле, хотя snapshot подтверждал сохранённый текст и
  сброшенный consent. Scoped textbox locator исправлен; финальный прогон
  подтвердил все сценарии без увеличения timeout и без изменения production UI.

Custom active/retired причины проверены изолированными domain tests. В browser
retired entry — явно synthetic read; реальный каталог fixture содержит только
`unspecified`, поскольку public настройки каталога пока нет. Прямых DB-правок
для обхода этого ограничения нет. Fixture Projects stopped, без Employees и
provider Runs; пользовательские credentials и платные провайдеры не использованы.

Тяжёлые проверки выполнялись последовательно с лимитами 6 GiB RAM, 1 GiB swap,
CPU 200% и двумя Cargo jobs. PostgreSQL/NATS, запущенные для приёмки, остановлены;
volumes сохранены, работающих session containers после проверки нет.

FRONTEND-015 опубликована как `5009c37`; FRONTEND-016 остаётся отдельным локальным
изменением до следующего запроса публикации. Полные UI epics остаются открытыми.
