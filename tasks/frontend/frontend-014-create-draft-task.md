# FRONTEND-014 — Создание черновика Task

**Статус:** done
**Epic:** UI1.3 / UI0.2; contracts UI0.1
**Приоритет:** P0
**Зависимости:** FRONTEND-010–013; ранний command-срез согласован отдельно.

## Результат

В live-разделе Tasks пользователь создаёт настоящую draft Task и открывает её
карточку. Core назначает ID/key и закрепляет явно выбранную PipelineVersion.
Approval, исполнение, property editor и настройка каталогов не входят.

## Контракт

- Узкий `POST /api/commands/create_task` передаёт существующий Core command.
  Envelope: project_id, expected_revision; payload: title, description,
  optional definition_of_done, kind, priority, pipeline_version_id, properties:{}.
  Idempotency-Key отдельно. Browser не задаёт actor, Task ID или lifecycle;
  альтернативный pipeline_id и непустые properties здесь не разрешены.
- Существующие owner/session/Host/Origin, strict JSON, UUID/revision, body/receipt
  limits и таймауты сохраняются. Receipt содержит новый UUIDv7 Task от Core;
  matching известного Task ID для amend/priority не ослабляется.
- Форма: title, description, optional DoD, явный delivery/analysis, active priority
  из схемы проекта и явная точная PipelineVersion. Допустимый default priority
  можно выбрать автоматически; Pipeline latest/default не подставляется.
- Pipeline выбирается из постраничного каталога с именем, версией и ID.
  Fresh detail проверяет совместимость kind и отсутствие удаления; project
  scope задаёт endpoint, окончательную принадлежность проверяет Core.
- Fresh PriorityScheme задаёт Project revision. Конфликт сохраняет ввод;
  refresh baseline не отправляет command. Недоступный выбранный ID сохраняется
  видимым и блокирует create до допустимого выбора.
- Unknown outcome фиксирует body/key; повтор возвращает ту же Task. Receipt
  подтверждает создание отдельно от последующих reads. Ошибка readback показывает
  созданный ID и предлагает повтор чтения, а не новое создание.
- Форма заменяет detail/editor в Tasks, соблюдает leave/session/scope guards.
  После fresh reads Project/list/new Task открывается карточка по Core ID.
  Отправка explicit onSubmit избегает автоматического сброса React form action.

Properties:{} и отсутствие DoD допустимы для draft; это не обещание успешного
approval. Новая Task не допускается к работе; общий post-commit dispatch Core
не изменяется. Приёмка выполняется на stopped Project без Employees/providers.

## Проверки и evidence

Покрытие: contracts/gateway strict payload и receipt; реальный Core create/reload/pin
для обоих kinds; exact retry с единственной записью; conflict и сохранение ввода;
чужой/удалённый/несовместимый Pipeline, retired priority; unavailable catalogs;
confirmed receipt с failed readback; scope/logout/late responses; keyboard/narrow.

Приёмка 2026-09-18:

- Rust unit: 266 passed (Core 80, Domain 117, UI 69); command conformance:
  52 passed, 61 integration tests явно ignored. Новый PostgreSQL
  `create_draft_receipt` — 1 passed: draft/no queue/no Runs, selected pin,
  audit, applied/replayed и конфликт ключа. После object-only hardening
  отдельно повторены 4 create gateway tests — PASS.
- Frontend: typecheck PASS; contracts 59, live unit 69, presentation 23 — PASS.
  Lint: 0 errors, 10 прежних react-refresh warnings. Scoped Prettier PASS.
- Финальный `just ui-test-live`: 104/104 browser tests PASS, включая 18 новых;
  Core egress 2 и настоящий rootless Podman sandbox test 1 — PASS.
  Secret scan: 290 issued values в основном наборе, 294 после intentional
  failure probe; утечек нет, probe — 1 expected failure / 0 unexpected.
- Live/demo/static builds PASS; static — 2 Node + 13 browser tests,
  demo — 7 browser tests. Провайдеры и реальные credentials не использовались.
- Workspace clippy all-targets/all-features с `-D warnings`, fmt, diff-check —
  PASS. Проверены локальные Markdown-ссылки; отсутствующих целей нет.
- Независимые spec и quality reviews — APPROVE, открытых findings нет.

Первый browser-прогон дал 101/103: тест ошибочно ожидал 422 для Pipeline
InvalidTransport (существующий Core contract — 400) и читал response body из
Chromium после закрытия формы. Helper теперь получает настоящий receipt до
передачи ответа браузеру; проверки canonical результата не ослаблены. Добавлен
conflict + synthetic deleted-detail case, полный повтор 104/104 прошёл.

Проверки выполнялись последовательно (6 GiB RAM, 1 GiB swap, CPU 200%, Cargo
jobs 2). Поднятые PostgreSQL/NATS остановлены; volumes сохранены.

FRONTEND-013 опубликована отдельно как `2a205d0`. Commit/push FRONTEND-014
не входят в этот этап. Полные UI epics остаются открытыми.
