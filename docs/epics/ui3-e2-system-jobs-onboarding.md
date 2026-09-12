# Epic UI3.2 — SystemJobs и Onboarding

**Milestone:** UI3 — Knowledge loop и операционная конфигурация
**Статус:** planned; реализация не начата
**Источник:** [UI plan](../UI_IMPLEMENTATION_PLAN.md),
[backend alignment](../UI_BACKEND_ALIGNMENT.md)
**База backend:** `1439211`; semantic work выполняется ownerless Runs.
**Зависимости:** UI0.1–UI0.3, UI3.1, UI2.1, UI2.3.

## Цель

Показать настройку и ход summarization/onboarding, их принятые результаты и
причины ожидания. Оператор должен различать job, generation, attempt, provider
Run, materialized memory и допуск нового Employee к работе.

## В границах

- Project SystemJob policy, pending/running/completed/held state и причины отказа.
- Связи job → generation/attempt → Run → source-linked derived result.
- Configure, request Task summary, request onboarding, explicit retry и skip.
- Onboarding pending/completed/skipped/legacy_bypass и familiarity receipt.
- Source revisions/digest/coverage, attempt/time/output quotas и usage uncertainty.
- Предупреждение о возможном provider вызове перед явным запуском или retry.

## Что уже есть и каких чтений нет

`GET /v1/projects/{project_id}/system-jobs` возвращает policy, settings revision,
jobs, aggregate attempt usage и onboarding receipts. Отдельный Employee onboarding
read уже есть. Named commands: `configure_system_jobs`, `request_task_summary`,
`request_employee_onboarding`, `retry_system_job`, `skip_employee_onboarding`.

Status не содержит credential binding и не заменяет полную bounded attempt
history. Для model/profile display и привязки результата нужны согласованные
безопасные projections существующих settings/attempts; секреты в DTO не включаются.
Опорный код: `crates/forge-core/src/http/system_jobs.rs`,
`crates/forge-storage/src/system_jobs.rs`, `crates/forge-core/src/system_jobs/`.

## Контракты и зависимости

- Job attempt владеет Run; target Employee и source Task являются контекстом.
  UI не создаёт фиктивных Task/Employee для фона и не считает его Employee slot.
- Onboarding receipt фиксирует знакомство с версиями, а не понимание или
  сохранённую provider session. Legacy bypass не отображается как completed.
- Result acceptance, physical stop и canonical materialization — разные факты.
  После stop pending jobs сохраняются; новый dispatch ждёт открытого gate.
- Retry ограничен current generation, policy и физической quiescence; UI
  не запускает повторную inference для уже принятого immutable receipt.
- Attempt limit относится к generation, отдельно действует rolling day cap.
  Missing CLI cost/usage остаётся unknown; успешный exit не заменяет valid result.
- Изменение Policy не означает автоматический onboarding всех Employees.

## Не в границах

Новые kinds SystemJob, массовый неограниченный backfill, background auto-retry
из браузера, manager planning LLM или расширение summarizer authority.
UI не требует создавать три derived outputs после каждой Task: сейчас личные
и проектные observations допустимы, но обязательный результат — TaskSummary.

## Направления будущей декомпозиции

1. Описать M3 settings/status/attempt/result DTO и дополнить OpenAPI.
2. Подключить job list/detail, Run links и memory materialization states.
3. Добавить guarded configure/request/retry/skip формы существующих команд.
4. Проверить onboarding gate, unknown usage, stop и retained pending generation.

Это направления работ; отдельные TASK-ID и файлы задач создаются позднее.

## Exit gate и проверки

- Новый Employee остаётся gated до completed receipt либо explicit audited skip;
  failed job и legacy bypass не дают ложного completed состояния.
- Valid result, late output и malformed output отображаются по Core evidence;
  late/stale attempt не заменяет текущую память.
- Stop сохраняет pending job и физический Run state, retry не обходит ownership.
- Повтор command receipt не порождает новую inference; ошибки revision и quota
  видны оператору с сохранённым введённым значением и актуальным state.
- Keyless fixtures/API/browser tests покрывают оба job kinds. Live provider
  приёмка остаётся отдельным явно разрешённым ограниченным сценарием.

## Риски

Смешение job completion и process exit скрывает незавершённую materialization.
Незаметный retry расходует подписку; stale UI не должен становиться ещё одним
источником автоматических SystemJob requests.
