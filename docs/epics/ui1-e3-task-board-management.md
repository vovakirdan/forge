# Epic UI1.3 — Task board и управляемые изменения

**Milestone:** UI1 — browser interface before M4 installer
**Статус:** `in_progress`; read-only FRONTEND-008 и draft edit FRONTEND-011 согласованы отдельно.
Остальной объём blocked by `UI0.1`, `UI0.2`, `UI0.3`, `UI1.1`, `UI1.2`.
**Зависимости:** UI0.1 model contracts, UI0.2 browser/API boundary, UI0.3 UI tooling,
UI1.1 Projects/Employees и UI1.2 PipelineVersion.
**Контракты:** ../2026-09-03-task-domain-model.md,
../2026-09-03-task-dependency-domain-model.md и
../2026-09-03-pipeline-domain-model.md.

## Цель

Показать одну долговечную Task через три независимые оси — lifecycle, stage и
Run activity — и дать человеку только разрешённые management commands. Board
отображает pinned PipelineVersion, project-defined priority/properties и
dependencies, а не хранит собственную Kanban state machine.

## В границах

- project-scoped board/list/detail: title, DoD, `TaskKind`, lifecycle, stage, waits, activity и pinned PipelineVersion;
- board mapping: lifecycle presentation (`draft/ready/in_progress/waiting/done/cancelled`) отдельно от произвольных Pipeline stage columns;
- dynamic `PriorityScheme` stable level ID/rank/labels и schema-driven typed properties;
- directed dependencies: `blocks`/`blocked_by` как две projection одного record; текущая supported condition — `task_done`, с видимым release state;
- named commands create/amend-draft/approve/change-priority/create-dependency/remove-dependency/cancel с reason catalog;
- command receipt, optimistic-revision conflict, immutable Event/activity refresh и rejected/precondition outcomes;
- Finding list/detail, triage и promotion через существующие commands.

## Не в границах

- stop/resume/force-stop Run, Project execution gate и physical reconciliation:
  это UI2.3;
- drag-and-drop как запись stage/lifecycle, свободная смена PipelineVersion или Employee-владелец Task;
- фиксированные types `bug`, `issue`, `research`, `review` или `test`;
- встроенные deadlines, estimates, risks или Goals для каждого Project.
- Goal/Epic references и commands (EXT1), dependency stage/artifact conditions
  и отдельная команда изменения dependency: их нет в текущем API, это не read gap.

## Базовое состояние и разрыв

Core/domain различают lifecycle, Pipeline stage, activity, `TaskKind`, PriorityScheme, properties, cancellation catalog и dependency record. Extracted frontend хранит fixed columns/priority, permanent assignee/reviewer и local toasts. Это UI/API gap, не другой Task model.

Отсутствующий board projection/property schema/command response UI0.2 фиксирует как backend contract. Новая semantics появляется только domain change, не browser filter/column name.

## Контракт интерфейса

`TaskKind` — только `delivery`/`analysis`; report, bug, issue и риск — property/label/presentation. Priority хранится stable level ID, не client string `critical/high/medium/low`.

PipelineVersion закрепляется при создании Task, включая draft; approve проверяет
готовность и не выбирает новый default. Draft можно amend в разрешённых границах.
Существенная правка после `ready` не предлагается без явно поддержанного Core
command/revalidation path и никогда не переписывает существующий Run context.

Cancel требует `cancellation_reason_id`, note optional. Board показывает wait/dependency gate, не объявляет done/cancelled по local click.

Dependency создаётся или удаляется canonical directed record через
`create_dependency`/`remove_dependency`; текущий `required_condition` — `task_done`.
UI показывает обе стороны, не держит две editable collections и не создаёт связь
для обычного review/verification stage. Замена связи не выдается за атомарный
update: отдельный change command или расширение conditions требуют дизайна Core.

## Зависимости

- UI1.1 даёт scoped Project/Employee reads; UI1.2 — pinned version, stage graph и board presentation; UI0.1–UI0.3 — DTO, command/event transport и tests.
- UI2.3 позже добавит execution controls поверх detail view.

## Направления будущей декомпозиции

[FRONTEND-008](../../tasks/frontend/frontend-008-live-task-reads.md) начинает
список/карточку Task в отдельном live entry: 20-record pagination, scoped reads,
точная pinned Pipeline stage, properties/waits и ID/kind/title/date артефактов. Mock Board,
commands, priority catalogs, dependencies и Run activity остаются вне Task.
Это не закрытие полного exit gate эпика.

[FRONTEND-011](../../tasks/frontend/frontend-011-draft-task-edit.md) добавляет
title/description edit существующей draft Task через `amend_draft`. Форма
фиксирует Project/Task revisions, сохраняет ввод после отказа и повторяет
неопределённый запрос только с исходными body/key. Receipt подтверждает запись,
а fresh reads обновляют карточку и список. Остальные поля и команды, создание
Task и mock Board не входят; evidence и независимые reviews фиксируются в Task.

- board/list/detail query model с lifecycle/stage/activity presentation;
- [FRONTEND-012](../../tasks/frontend/frontend-012-project-priorities.md): names
  из PriorityScheme в списке/detail, raw ID и retired/stale/unavailable состояния;
  без изменения приоритета, сортировки Task или полного подключения Board;
- schema-driven property и PriorityScheme controls;
- draft/create/approve/priority/cancel command forms и receipt conflicts;
- dependency graph/read panels и management command interactions;
- event-driven refresh/accessibility tests для waits, terminal states и empty boards.

## Exit gate

Project с одной, тремя и десятью priority levels рендерится без client enum. Pipeline без review/test не получает искусственные колонки; `waiting`/`done` не выдаются за stage. Detail разделяет lifecycle, stage, activity, pinned version и dependency gate.

Create/amend/approve/priority/cancel идут через receipt; stale revision сохраняет input, не меняет board. Cancel без reason ID не отправляется. Dependency sides согласуются после reload; stop/resume нет до UI2.3.

## Проверки

- browser/API fixtures с arbitrary stages, priority schemes и property schemas;
- lifecycle/stage/activity matrix, waits и terminal state regression tests;
- command contract tests для create/amend/approve/priority/cancel/dependency, stale revision, forbidden actor и idempotent replay;
- cross-project cache/scope tests и dependency direction negative cases.

## Риски

Риски: stage/lifecycle/activity в одном badge, hardcoded priority/types, optimistic drag-and-drop и две mutable dependency collections. UI показывает canonical receipt/Event, не локальную иллюзию успеха.
