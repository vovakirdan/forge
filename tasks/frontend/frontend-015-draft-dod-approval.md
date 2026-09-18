# FRONTEND-015 — DoD черновика и явный approval

**Статус:** done
**Epic:** UI1.3 / UI0.2; contracts UI0.1
**Приоритет:** P0
**Зависимости:** FRONTEND-011–014; отдельный command-срез согласован.

## Результат и границы

Пользователь сохраняет Definition of Done в существующем draft editor, затем
отдельно подтверждает допуск сохранённой Task к исполнению. Создание и сохранение
черновика не вызывают approval. Команда approval не открывает execution gate,
но при уже открытом Project может привести к запуску работы.

Property editor, смена pin, настройка схем/каталогов, отмена, stop/resume и
управление execution gate не входят. Domain policy остаётся в Core.

## Контракт

- `amend_draft.patch.definition_of_done`: отсутствие сохраняет значение,
  `null` очищает, непустая строка заменяет. Лимит — 20 000 Unicode scalar values.
  Пустое поле формы отправляет null; patch содержит только изменённые поля.
  Исправляется существующее несоответствие Serde parser обещанной семантике null.
- Conflict refresh сохраняет изменённый пользователем текст, обновляя остальные
  поля. Exact retry повторяет прежние bytes/key; save остаётся draft-only.
- Отдельный `POST /api/commands/approve_task`: project_id, expected_revision,
  payload {task_id, expected_task_revision}; Idempotency-Key — в заголовке.
  Сохраняются owner/session/Host/Origin, strict payload, лимиты и receipt validation.
- Перед подтверждением читаются свежие Project, Task и закреплённая версия
  Pipeline. Показываются сохранённые title/DoD, точный pin и execution gate.
  Отсутствующий DoD блокирует подтверждение и предлагает редактор; остальные
  условия готовности проверяет Core. Удалённый из каталога закреплённый Pipeline
  не блокируется на стороне UI.
- После конфликта нужны refresh и новое подтверждение. Unknown outcome разрешает
  только exact retry либо уход с предупреждением. Receipt подтверждает approval
  отдельно от readback; ошибка readback разрешает повтор GET, не новый command.
- UI читает итоговый lifecycle и revision, не вычисляет их: entry executor и
  зависимости могут привести к ready, waiting или in_progress.
- Одновременно открыт один editor/panel. Сохраняются leave/session/scope guards,
  защита от поздних ответов и keyboard/narrow layout.

## План проверки

1. Parser regression missing/string/null, clear-only patch, Unicode limits.
2. Gateway/contract/attempt tests: strict envelope, matching receipt, immutable
   retry и revision refusals; старые amend/priority/create не ослабляются.
3. Реальный Core: create без DoD → save DoD → approve, pin, replay и отсутствие
   повторных эффектов; employee/human/external/dependency ветки и Core refusals.
4. Browser: сохранение/очистка DoD, conflict, отдельное подтверждение, lost response,
   malformed receipt, failed readback, logout/scope/late responses, keyboard/narrow.
5. Последовательная ограниченная приёмка Rust/frontend/live/static/demo/security;
   независимые spec и quality reviews. Fixture Projects остановлены, Employees
   отсутствуют, платные провайдеры и пользовательские credentials не используются.

## Evidence

Приёмка 2026-09-18:

- Rust: Core 80 + Domain 117 + UI 73 = 270 unit tests; Application 20 unit и
  public-contract 7 — PASS. Command conformance: 53 passed, 62 integration
  tests явно ignored в общем прогоне. Новый approval scenario повторён на
  memory и PostgreSQL — PASS 2, с сохранением pin, audit/replay/queue invariants.
- Registered-system approval → in_progress: существующий PostgreSQL/NATS test
  `invalid_first_hook_policy_does_not_block_later_valid_task` — PASS 1.
- Frontend: typecheck PASS; contracts 61, live unit 74, presentation 23 — PASS.
  Lint: 0 errors, 10 прежних react-refresh warnings; scoped formatting PASS.
- Финальный `just ui-test-live` — 121/121 browser tests PASS (17 новых),
  Core egress 2 и настоящий rootless Podman isolation test 1 — PASS.
  Secret scan: 382 issued values в основном наборе, 386 после intentional
  failure probe, утечек нет. Probe: 1 expected failure, 0 unexpected.
- Live/demo/static builds PASS; static: 2 Node + 13 browser tests; demo:
  7 browser tests. Провайдеры и пользовательские credentials не использовались.
- Финальный workspace clippy all-targets/all-features с `-D warnings`, fmt и
  diff-check — PASS. Проверены 124 локальные Markdown-цели, отсутствующих нет.
- Независимые spec и quality reviews — APPROVE, замечания закрыты.

Проверки выполнялись последовательно под лимитами 6 GiB RAM / 1 GiB swap /
CPU 200%, Cargo jobs 2. Поднятые PostgreSQL/NATS остановлены, volumes сохранены.
FRONTEND-014 опубликована как `f201e00`; FRONTEND-015 пока без commit/push.
Полные UI epics остаются открытыми.

Найденные и исправленные случаи:

- Parser regression сначала подтвердил, что явный null теряется, затем прошёл
  после отдельного десериализатора present-nullable поля.
- Независимый review обнаружил устаревший DoD в «Current saved values» после
  conflict → refresh → save. Readback теперь обновляет локальный baseline;
  browser regression проверяет новый текст и отсутствие старого.
- Первый live-прогон дал 119/121. Pending approval мешал смене Project:
  синхронная навигация была помещена в React action и ожидала async transition.
  Project submit теперь обычный обработчик; test удерживает реальный ответ
  принятой команды и требует ухода/отмены ожидания до выдачи этого ответа.
- Второй отказ первого прогона — transport error старого boundary probe.
  Конкретный probe тогда не был установлен. Гонка раннего отказа с загрузкой
  большого body — гипотеза, не доказанная причина. Oversize probe теперь явно
  использует Expect: 100-continue и строго требует настоящий 413; реальные body
  limits также покрыты Rust-тестами. Добавлены только фиксированные безопасные
  error categories и подписи этапов; транспортная ошибка не считается успехом.

Required-property refusal проверяется на изолированной копии domain Task:
публичного command настройки property schema пока нет. Это не browser/schema
configuration proof и не прямое изменение canonical DB. Registered system entry
проверяется существующим keyless PostgreSQL/NATS hook-policy тестом, не запуском
платного провайдера. Open-gate warning в browser проверяется synthetic read;
реальные fixture Projects остаются stopped, без Employees и provider Runs.
