# TASK-05: общий command engine

M0-команды исполняются в `forge-application` через один `CommandTransaction`.
Production использует PostgreSQL adapter из `forge-storage`; тесты — также
in-memory adapter из `forge-testkit`. Это reference persistence для проверки
команд, не память Employee и не новый режим запуска Forge без PostgreSQL.

## Границы

Общий engine обрабатывает 14 команд: создание Project, Pipeline, Employee и
Task; изменение draft, approve/cancel/resume/priority; создание и удаление
dependency; start/stop проекта; внешний stage outcome.

Четыре команды M1 — настройка runtime, enrollment credentials, boot policy и
recovery assessment — остаются расширениями Core. Они используют общие
prepare, fingerprint, replay и finalization. Supervisor, dispatch, Lease
reconciliation и provider I/O не входят в reference backend.

Core открывает и завершает production transaction. Команда атомарно сохраняет
state, Event, outbox и idempotency receipt. Только после commit Core доставляет
stop и запускает dispatch. Ошибка этой доставки не отменяет сохранённый receipt;
повтор команды возвращает прежние IDs и повторяет post-commit доставку.

`CommandContext` задаёт доверенные actor, Project scope и capabilities. HTTP
payload не может присвоить себе полномочия. Проверка доступа предшествует
replay; canonical JSON и typed payload должны совпадать. Fingerprint сохраняет
прежний формат, поэтому старые receipts продолжают работать после refactoring.

## Reference backend и время

Memory transaction удерживает writer lock и работает с отдельным снимком.
Commit публикует снимок целиком; rollback или drop его отбрасывает. Adapter
реализует persistence constraints, но не повторяет обработчики команд.

`ManualClock` позволяет менять время без sleeps. После Project lock engine
выбирает canonical timestamp не раньше сохранённого времени Project. Queue
eligibility использует raw wall clock; runtime timers и физические Lease
deadlines остаются отдельной ответственностью Core/storage.

## Проверки

Без PostgreSQL, NATS, Podman и credentials:

```sh
just test-command-conformance
```

Этот target запускает memory-сценарии, проверки fingerprint и fake clock.
PostgreSQL-тесты отмечены `ignored` и здесь не считаются пройденными.
`just test-unit` и полный workspace test также включают services-free suite.

С локальными сервисами и synthetic runtime fixture:

```sh
just dev-up
just build-runtime-fixture
just test-integration
```

Общие сценарии исполняют тот же engine на memory и на отдельных PostgreSQL
schemas. Проверяются:

- все 14 команд, invalid transitions, scopes и явная делегация capabilities;
- optimistic revisions, конкурентные команды, replay и конфликты fingerprint;
- rollback после aggregate write, Event/outbox append, receipt insert и перед commit;
- согласованность queue, waits, artifacts и активных Run stop intents;
- сохранение Lease/reservation до физического выхода, stale fence и wrong epoch;
- canonical/raw time при равных, будущих и откатившихся часах;
- прежние HTTP statuses и receipts, включая старый M1 receipt;
- сохранённая команда при фактической ошибке post-commit stop delivery.

Runtime state в command-сценариях задаётся fixture. Физическое выполнение,
изоляцию и provider transport проверяют отдельные synthetic/runtime/provider
gates и явно разрешённый [Codex live gate](../infra/runtime/CODEX_LIVE.md).
Качество ответов модели эти проверки не доказывают.

## Приёмка — 6 сентября 2026

TASK-05 закрыта в рабочем дереве поверх `b685480`. Services-free suite: 16 passed;
PostgreSQL suite: 17 passed. Workspace tests, strict Clippy, formatting и полный
`just test-integration` прошли. CLI smoke завершил Task в `done` с одним Artifact.

Независимое RO-review проверило перенос handlers, commit boundary, legacy replay,
reference constraints и regression tests. Найденные ранее расхождения uniqueness
и проверки physical ownership исправлены; финальное review не выявило P0/P1/P2.
Проверены лимит 500 эффективных строк на Rust-файл и `git diff --check`.

Production остаётся PostgreSQL-backed. M0–M1 runtime acceptance и границы live
проверки зафиксированы отдельно в [M1_RUNTIME.md](M1_RUNTIME.md). Knowledge loop,
Summarizer и память Employee не входят в эту приёмку.
