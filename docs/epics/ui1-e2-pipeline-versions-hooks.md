# Epic UI1.2 — PipelineVersion, stages и optional hooks

**Milestone:** UI1 — browser interface before M4 installer
**Статус:** `in_progress` — ранний read-only срез FRONTEND-010; полный объём зависит от gates `UI0.1`, `UI0.2`, `UI0.3`.
**Зависимости:** UI0.1 model contracts, UI0.2 browser/API boundary, UI0.3 UI tooling.
**Контракты:** ../2026-09-03-pipeline-domain-model.md,
../2026-09-03-task-domain-model.md и ../M2_PROJECT_HOOKS.md.

## Цель

Сделать Pipeline catalog и stage contracts наблюдаемыми и управляемыми без
редактирования исторического графа. Пользователь публикует новую immutable
PipelineVersion, выбирает default для будущих Task и видит, какие hooks реально
применимы к конкретной версии.

## В границах

- project-scoped Pipeline catalog: current default/latest version, deletion state и число pinned Task в safe projection;
- immutable graph view: applicability по `TaskKind`/properties, entry stage, outcomes, transitions, terminal routes и board presentation;
- создание и publication полной новой PipelineVersion вместо inline edit published version;
- выбор уже опубликованной version как default через named command и receipt;
- soft delete Pipeline: запрет новых назначений отдельно от продолжения pinned Task;
- stage contract: executor, capability, resource/retry boundary, Artifact requirements и acceptance policy без UI validation;
- optional hooks в новой PipelineVersion: configuration/read projection, applicability, invocation и mapped result, включая explicit skip.

## Не в границах

- automatic Task migration, попытка "починить" старую version или delete PipelineVersion;
- запуск hook, просмотр его private runtime, shell output или provider credential;
- глобально обязательные QA, review, Git, MR или hook;
- прямое перемещение Task между stages и runtime management действий.

## Базовое состояние и разрыв

Domain/M2 определяют publication, default selection, soft delete, immutable graph
и optional hook semantics. Импортированный browser UI сохраняет mock mutable
config и fixed stage names. Отдельный live entry FRONTEND-010 начинает чтение
реальных версий и stages; UI0.2 подтверждает commands до будущих форм.

Отсутствующий editor endpoint — backend/UI gap. Новый stage outcome или hook policy — domain вопрос, не свободный JSON editor в browser.

## Контракт интерфейса

Pipeline mutable только как catalog: его default pointer и `deleted_at` меняются
аудируемыми командами. Definition PipelineVersion всегда read-only после
publication; UI объясняет изменение графа действием «создать следующую version».
Read DTO также содержит изменяемые name, catalog revision, default/latest и
deleted_at; весь ответ нельзя считать immutable или кэшировать навсегда по
version ID. Default может указывать не на latest; soft delete сохраняет history.

Изменение default влияет лишь на новые Task. Уже созданные draft и активные Task
остаются pinned; UI не предлагает авто-migrate и не выводит warning как
разрешение изменить их stage mapping.

Stage — не lifecycle status. Имена implementation, review, test или deploy —
данные конкретной версии, а не фиксированные колонки или enum клиента. Hook
рисуется только при его applicability; отсутствующий hook не нуждается в
фиктивном passed/skip result.

Форма передаёт complete definition, expected revisions и idempotency key через
API boundary. Receipt, validation error и conflict сохраняют draft формы, но
не создают client-side опубликованную version.

## Зависимости

- UI0.1 фиксирует DTO graph/version identity/authority; UI0.2 — publication/default/delete transport и fresh projections; UI0.3 — diagram/form/testing primitives.
- Hooks остаются optional M2 contract и не делают UI1 зависимым от Git runtime.

## Направления будущей декомпозиции

[FRONTEND-010](../../tasks/frontend/frontend-010-live-pipeline-reads.md) —
согласованный ранний срез: список **версий** по 20, fresh detail, stages и
transitions, read-only typed inspector. Это не полный каталог уникальных
Pipeline; total и pinned Task usage не выдумываются. Entry stage выбирается
первой; unresolved reference остаётся явным ID. Instructions — plain text,
system actions только описываются, private hook runtime не открывается.
`null` workspace/acceptance/max_stage_visits означает «не настроено», а не
отсутствие WorkSurface, автоматическую acceptance или unlimited execution.

Full-definition list и detail ограничены 1 MiB; Task/Run lists остаются 64 KiB.
Core пока загружает все версии до pagination и читает catalog для каждой (N+1).
Допустимая страница может превысить cap; UI показывает size error, не empty list,
truncation или adaptive retry. Fresh detail/cache isolation и отдельная приёмка
фиксируются в Task; это не закрывает полный gate эпика.

Оставшиеся части:

- catalog/version history и pinned-task usage views;
- graph navigation и развитие contract inspector;
- publish-next-version form с server validation/receipt handling;
- default selection и soft-delete confirmation flow;
- hook applicability/result projection без runtime-console access.

## Exit gate

Две версии Pipeline — независимые immutable graphs. Пользователь не может inline-edit/delete published version; default change виден только для новых Task, pinned Task остаются на прежней версии. Soft-deleted Pipeline не предлагается новой Task, но history читается.

Stage graph не использует fixed review/test enum. Pipeline без hooks/Git/review отображается полноценно; applicable hook показывает canonical status/evidence metadata без ложного запуска.

## Проверки

- browser fixtures с несколькими versions, soft-delete и pinned old Task;
- API contract tests для complete definition, stale revision, replay receipt и validation error;
- snapshot tests для arbitrary stage IDs, human/external/system executors и Pipeline без review/QA/hook;
- authorization/scope tests, что UI не получает private hook runtime data.

## Риски

Риски: mutable-looking editor для historical graph; stage/lifecycle conflation; optional hook как global quality gate. Каждый случай останавливает server receipt/error и ясный UI copy.
