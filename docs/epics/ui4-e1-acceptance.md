# Epic UI4.1 — Сквозная Control Room acceptance

**Milestone:** UI4 — UI-приёмка и передача установщику
**Статус на 23 сентября 2026:** локальный gate пройден: 178/178 keyless browser scenarios, отдельный 1 000 Task / 20 Employee test и согласованный live scenario. Границы доказательства — в [отчёте](../UI4_LIVE_PROPOSAL.md).
**Тип / приоритет:** testing / P0
**Источники:** [UI-план](../UI_IMPLEMENTATION_PLAN.md),
[матрица соответствия](../UI_BACKEND_ALIGNMENT.md)

## Цель

Доказать, что Control Room показывает и управляет настоящим Forge, а не только
совпадает с макетом. Дать пользователю воспроизводимые сценарии для дальнейших
ручных проверок, сохранив результаты и ограничения каждого вида evidence.

## В границах

- Keyless end-to-end: Project → Pipeline → Employee/onboarding gate → Task →
  Run → review/QA по явному Pipeline → artifacts/handoff → summary/history.
- Отдельные сценарии non-Git/analysis без hooks и Git worktree с optional hook;
  retry/rework оставляет одну Task, candidate acceptance не переносится между SHA.
- Два Projects, несколько Task/Run, browser reconnect/reload, конфликт revisions,
  delayed response, offline Core, Project stop и physical uncertainty.
- Inbox/exact-Run message, pending human resolution, cancelled wait, durable
  resume/recovery; устаревший ответ не применяется к новому stage visit.
- Onboarding failure/explicit skip, Summarizer disabled/lag/error, index outage,
  withdrawn page/memory; UI сохраняет различие authority и derived history.
- Performance/UX: 1000 Task, 20 Employee, bounded lists/object previews,
  keyboard/focus, narrow layout, actionable failure states.
- Отдельный operator-approved live сценарий с точной моделью и ограниченным
  allowance; приватный test repo/session и отчёт о фактическом cleanup/quiescence.

## Не в границах

Clean-host installer/reboot proof M4, полное покрытие всех provider lanes,
новые EXT domains, production deployment или автоматические проверки чужого repo.
Provider quota не тратится самим открытием экрана или запуском keyless suite.

## Контракты и зависимости

**Зависимости:** UI1.1, UI1.2, UI1.3, UI1.4, UI2.1, UI2.2, UI2.3,
UI3.1, UI3.2, UI3.3. Shared harness — UI0.3; domain/security contracts — UI0.1/0.2.
Каждый feature epic уже имеет свои tests; здесь проверяются их совместные сценарии.

## Направления будущей нарезки

1. Изолированный keyless browser/Core scenario и durable evidence report.
2. Failure/reconnect/scope/concurrency matrix и regression coverage findings.
3. Performance/keyboard/readability review и операторский сценарий.
4. Согласованный live proof, независимое review и closure ledger.

## Exit gate и проверка

- Frozen install/build/typecheck/lint, feature tests и keyless suite проходят.
  Backend changes удовлетворяют PROJECT_RULES; результаты не выводятся из mocks.
- Live proof хранит IDs реальных Run, исходные artifacts, summary provenance и
  подтверждённую остановку; где проверено только retrieval/delivery, это названо
  именно так, без заявления о семантическом «обучении» агента.
- Без нового разрешения на live milestone остаётся pending_live, не closed.
  Прошлые M2/M3 Runs не засчитываются как новый browser acceptance.
- Оставшиеся gaps/deferred provider lanes перечислены; ни один enabled action
  не сообщает успешное изменение без accepted Core receipt.
- Пользователь может повторить описанный сценарий через UI без SQL repair.

## Риски

Недоступный provider не оправдывает изменение реальных статусов для зелёного
отчёта. Test data, secrets и логи сохраняются/очищаются по явной session policy,
а не удалением чужих volumes или рабочих репозиториев.
