# Forge: UI ↔ backend ↔ PRD

**Дата исследования:** 11 сентября 2026
**Baseline:** backend `1439211`, пользовательский архив в `frontend/`
**Статус:** исходная инвентаризация для [UI-плана](UI_IMPLEMENTATION_PLAN.md)

## 1. Как читать матрицу

Есть четыре разных вида работы: адаптировать frontend к существующему домену,
открыть существующие данные через API, добавить новую PRD-возможность либо убрать
неподтверждённое поведение прототипа. Отсутствие GET не означает отсутствие домена;
наличие колонки/кнопки в UI не означает наличие работающей операции.

Frontend использует React/TypeScript, TanStack Start/Query, Tailwind и UI components.
[Сервисы](../frontend/src/services/index.ts) читают `mockDb`; management actions
часто ограничены toast/local state. Исходный код сохранён для адаптации, не объявлен
production-ready. Сборка и реальная интеграция — будущие gates UI0–UI4.

## 2. Покрытие экранов

| Область | Frontend baseline | Backend baseline / gap | Направление |
|---|---|---|---|
| Projects / Overview | Selector, metrics, risks, current goal | Project detail есть; нет list/full config/aggregate dashboard | UI1.1, UI2.1, UI3.3 |
| Task / Board | Fixed stages/priorities, assignee, artifacts, findings | Task commands/detail есть; нужны schema/dependency/constraint/schedule/handoff views | UI0.1, UI1.3, UI1.4, UI2.3 |
| Team / Hire | Один manager/current Run, fixed engines, toast hire | Catalog mutations/runtime есть; нет Employee list/detail/profile reads | UI1.1 |
| Pipeline | Local editable stages/save | Immutable versions/default/soft-delete, graph/acceptance есть | UI1.2 |
| Run / Activity | Mock events, engine/agent/task, fake abort | Diverse Run purposes, evidence/incidents/usage есть; context/object reads неполны | UI2.1, UI2.3 |
| Chat | Mock channels, synthetic Lead answer, local pending | Inbox/threads/Communication/receipts есть; нужны aggregated reads | UI2.2 |
| Human decisions | Mock structured cards | Escalation/resolver/human commands есть; queue/detail read API нет | UI2.3 |
| Findings | Report cards/promote toast | Report/triage/promote и paginated GET реализованы | UI1.3 |
| Knowledge | Policies/decisions/lessons/cards | Canonical Markdown, scoped memory, history/search/projection status есть | UI3.1 |
| Onboarding / Summarizer | Нет полноценного management UI | SystemJob settings/request/retry/skip/status существуют | UI1.1, UI3.2 |
| Surfaces | Workspace/base SHA в Task | Git/non-Git, pin/latest, snapshots, reviews/integrations есть; generic views неполны | UI1.4 |
| Resources | Named pools, CPU/IO, capacity/drain controls | Admission/per-Run/SystemJob limits есть; generic ResourcePool нет | UI3.3; новая модель EXT2 |
| Settings/providers | Save/archive/integration placeholders | Частичные commands/startup config; нет полного metadata/readiness API | UI1.1, UI3.3 |
| Goals/Epics/waves | Структура planning UI | Нет отдельных aggregates/commands/tables; optional PRD domain | EXT1 |
| SkillPack / CodeIndex | Mock catalogs | Нет reusable catalogs или runtime integration; SQL skills field не означает готовый SkillPack | EXT3 |

## 3. Обязательные изменения frontend-модели

- Разделить lifecycle, stage закреплённой PipelineVersion и Run activity.
  `draft/ready/in_progress/waiting/done/cancelled` не заменяются списком стадий.
- Добавить `TaskKind: delivery | analysis`; типы вроде bug/report и optional
  risk/deadline/estimate представлять project-defined properties, не Core enums.
- PriorityScheme и CancellationReasonCatalog имеют project-defined stable IDs;
  значения/обязательность/отображение берутся из схем, а не из fixed UI enums.
- Task не владеет постоянным Employee: фактические исполнители принадлежат Runs,
  next-Run assignment constraint показывается отдельно от уже работающего Run.
- Employee catalog state, capacity/availability, provider preference и onboarding
  — разные данные. Resolver routes не сводятся к одному `managerId`.
- Pipeline edit создаёт новую version; default для новых Task и pinned version
  существующей Task различаются. Не добавлять silent Task migration.
- Git, MR, review, QA и hooks не обязательны для каждой Task. Источник результата,
  submission/acceptance, candidate SHA и artifact body type отображаются явно.
- Run допускает Task, Communication, Resolution, Hook и SystemJob purposes;
  Employee/Task/workspace могут отсутствовать без создания фиктивных сущностей.
- Project selector должен менять scope запросов, cache и подписок. Сейчас
  [Board query keys](../frontend/src/routes/board.tsx) не содержат Project ID.
- Personal memory не определяется текстовым массивом `lessons` профиля.
  Используются scoped canonical entries, revisions, source refs и job provenance.

Исходные упрощения видны в [frontend types](../frontend/src/data/types.ts),
[HireWizard](../frontend/src/components/team/HireWizard.tsx) и
[Pipeline screen](../frontend/src/routes/pipelines.tsx).

## 4. Backend-функции, которые нужно сделать видимыми

| Функция | Нужное представление |
|---|---|
| Dependencies | Views blocks/blocked_by, текущие create/remove и condition task_done; stage/artifact conditions требуют отдельного расширения Core |
| Management | Disable/stop/retire, Task pause/cancel, next-Run employee, resume alarm |
| Resolution | Очередь, route/assignment, human fallback, ограниченные варианты ответа |
| Recovery | Incident, accepted assessment, физическая остановка, доступные продолжения |
| Inbox | Persisted/delivered/acknowledged/answered отдельно; exact Run/Task targets |
| SystemJob | Ownerless execution, policy, attempts/limits, request/retry и audited onboarding skip |
| Knowledge loop | Authority, personal/project derived entries, history/withdrawal/index lag |
| WorkSurface | Snapshot/source policy, accepted candidate и handoff provenance |

Основания: [management](M2_MANAGER_CONTROLS.md), [resolution](M2_RESOLVER_QUEUE.md),
[Inbox](M2_INBOX.md), [knowledge](M3_KNOWLEDGE.md),
[SystemJobs](M3_SYSTEM_JOB_COMMANDS.md), [admission](M2_ADMISSION_LIMITS.md).

## 5. Интеграционные пробелы

- Operator API использует owner-only UDS. Browser нуждается в отдельной
  защищённой локальной HTTP boundary; простой public TCP/CORS proxy недостаточен.
- [OpenAPI](../openapi/v1.yaml) не содержит M3 knowledge/memory/SystemJob routes
  и commands, хотя они реализованы. Schema/client contract нужно синхронизировать.
- SSE сейчас выдаёт конечную пачку событий. Клиент обязан продолжать чтение
  по cursor с backoff/dedup; нельзя рассчитывать на вечную подписку.
- Commands проверяют idempotency и expected revisions. Toast появляется после
  receipt; physical stop отображается по observed state, не по успешному POST.
- Profiles, credential metadata и readiness нельзя получать чтением secrets
  или произвольных host files. Model selection остаётся явным; health не тратит quota.
- Не все настройки изменяемы онлайн. Startup-only admission caps нельзя
  выдавать за доступный ResourcePool CRUD; изменение требует безопасного workflow.

## 6. Сохранить, адаптировать, расширить, отложить

Сохраняются visual shell и components. UI0–UI4 заменяют модель, fixtures в live
режиме, запросы и actions; недостающие read projections входят в эти эпики.

В live UI убираются синтетические Lead answers, invented usage/familiarity,
неподдерживаемые engines и кнопки, сообщающие об эффекте без Core command.
Отсутствующие возможности не получают mock-success fallback. Unknown — не zero.

Goal/Epic/waves, named pools и context integration catalogs остаются продуктовым
roadmap EXT1–EXT3. Они не удаляются из PRD из-за отсутствующей реализации, но и
не добавляются в backend без отдельного дизайна и явного выбора объёма.

## 7. Известные расхождения документов

17 сентября в FRONTEND-002 исправлены две устаревшие обзорные формулировки PRD:

- PRD §9.4/FR-021 теперь описывает выбор версии уже в `create_task`, согласно
  [M2 Pipeline management](M2_PIPELINE_MANAGEMENT.md). Approval сохраняет этот
  выбор в execution-spec revision; смена default не меняет существующий draft.
- PRD §23 теперь описывает exact-candidate/CAS без скрытого rebase за Employee,
  согласно [M2 Git integration](M2_GIT_INTEGRATION.md) и PRD §14.4.

Это doc alignment UI0.1, без изменения работающего Core.

Отдельно сохраняются принятые ограничения M3:

- PRD §12.3 описывает targeted retrieval без повторного introduction. Текущий
  [M3 context](M3_CONTEXT.md) включает bounded introduction/guide и последние
  видимые записи из PostgreSQL; targeted retrieval не заменил этот baseline.
- FR-075 требует `confidence` для MemoryItem. Текущий
  [DerivedMemoryEntry](M3_KNOWLEDGE.md) хранит scope/evidence/revisions, но не
  такой confidence-контракт. UI не вычисляет и не подставляет его сам.

Эти отличия явно приняты при обсуждении M3; они не переоткрывают её gate и не
становятся автоматическим долгом UI. Дальнейшее развитие памяти или изменение
формулировки PRD требует отдельного решения; прототип не задаёт это решение.

## 8. Read contracts FRONTEND-002

**Сверка:** 17 сентября 2026, Core из baseline `3854acf`.
Новый изолированный слой [contracts](../frontend/src/contracts/index.ts) описывает
существующие read responses, не полный domain aggregate и не клиент HTTP.
Источники wire-формата — текущие [serializers](../crates/forge-core/src/http/views.rs),
[routes](../crates/forge-core/src/http/handlers.rs) и domain serialization tests.
[OpenAPI](../openapi/v1.yaml) сверяется с ними, но не используется для слепой
генерации: обнаруженный drift перечислен ниже. Эти уточнения не переписывают
историческую инвентаризацию 11 сентября в разделах 1–6.

### 8.1. Карта полей и представлений

| Поле интерфейса / смысл | Источник сейчас | Граница представления |
|---|---|---|
| Project id/name/revision, execution gate | `GET /v1/projects/{project_id}` → ProjectView | Gate `open/stopped` не означает состояние каждого Run |
| Task identity/title/kind/lifecycle/priority/updated_at/revision | TaskSummaryView в Task list/detail | Priority — stable ID без выдуманного rank/color; lifecycle отдельно от stage/activity |
| Project scope Task/Pipeline | Параметр маршрута, отсутствует в DTO | Будущий клиент передаёт scope явно; поле не добавляется в wire response |
| Task description/DoD/properties | TaskDetailView | DoD nullable, description может быть пустым; cancelled draft может не иметь DoD/stage |
| Текущая стадия Task | `current_stage_id` + версия по `pipeline_version_id` | `resolveTaskStage` находит только pinned stage; отсутствие/несовпадение возвращает явный результат |
| Pipeline graph, stages, transitions | `/pipelines` и `/pipelines/{pipeline_version_id}` → PipelineVersionView | Список содержит версии, не каталоги; порядок stages не задаёт линейный flow |
| Pipeline version/catalog/default/latest/deletion | Поля PipelineVersionView | Значения раздельны; новая default и soft-delete не подменяют pinned graph |
| Ожидания Task | `wait_conditions[]` в detail | Сохраняется каждый wait; kind открыт, не сводится к одному blockedReason |
| Артефакты | `artifacts[]` в detail | Kind открыт; metadata — JSON object, body — произвольный JSON. Разбор похожей на object-reference формы и загрузка объекта сюда не входят |
| Properties values | Tagged `{type,value}` в detail | Типы boolean/text/number/date/enum/multi_enum/reference; date — `[year, ordinal_day]`, reference — `{reference_type,reference_id}` |
| Assignee/reviewer/current Run, activity | Отдельные Run/assignment данные, вне этого среза | Нет постоянного владельца-Employee в Task; lifecycle не доказывает running/stopping |
| Git/worktree/base SHA/MR | Отдельные surface/evidence reads, вне этого среза | Не обязательные поля Task, analysis допустима без Git |
| Goal/Epic/waves и invented metrics | Только старый demo / design-gated roadmap | Не добавляются в Core DTO и не синтезируются из отсутствующих данных |

Схемы Zod проверяют известные поля и сохраняют дополнительные поля для additive
совместимости. Закрытые enum остаются закрытыми; числа revisions должны быть
safe integers JavaScript. Никаких coercion, defaults или переходов Core в схемах
нет. Nullable fields, которые serializer выводит всегда, обязательны даже при
`null`; пустые `artifact_requirements` и конечный `next_cursor` могут отсутствовать.
Расширяемые поля не дают разрешения показывать непроверенные данные или исполнять
команды. Artifact body не становится безопасным HTML или URL после JSON validation.

### 8.2. Именованные gaps и владельцы следующего шага

| Gap | Что отсутствует / расходится | Дальнейшая работа |
|---|---|---|
| `project-task-catalogs` | Project read не отдаёт PriorityScheme, TaskPropertySchema, CancellationReasonCatalog | UI0.1 согласует contracts; UI1.1 открывает Project reads. До этого нет ранжирования priority и schema-driven форм |
| `task-cancellation-read` | Task read не содержит reason/note | UI1.3 read projection; `cancelled` сейчас не позволяет показать reason, нельзя подставлять `unspecified` |
| `task-intent-history-read` | Нет labels/rationale/scope/source/creator/created_at/execution-spec revision/stage visit | Уточнение UI0.1 и scoped reads UI1.3; не реконструировать из title или fixtures |
| `pipeline-board-layout` | Нет board column order/config; stages обходятся как map | UI1.2/UI1.3 согласуют display layout, не трактуя порядок массива как transitions |
| `project-list-read` | Нет Project list endpoint | UI1.1, не подмена списком из mockDb |
| `openapi-task-key` | Regex `^TASK-[1-9][0-9]*$` не принимает реальный formatter `TASK-{:03}` → `TASK-001` | UI0.2 синхронизирует OpenAPI; этот read layer принимает непустой opaque key |
| `openapi-required-fields` | OpenAPI считает optional Task `current_stage_id/updated_at`, Wait `detail/source_stage_id`, Stage `acceptance_policy/system_action`; serializer выводит их всегда | UI0.2 синхронизирует required lists; frontend следует фактическому serializer |

Fixtures с одним/десятью уровнями полного Project-каталога и cancelled с доступной
причиной остаются следующей частью UI0.1 после согласования соответствующих reads.
Отсутствие поля — неизвестное значение, а не ноль, пустой каталог или отсутствие
сущности. JSON `u64` за пределом safe integer отклоняется клиентом; новый wire-формат
чисел этим срезом не вводится.

Тесты используют явно synthetic wire-shaped fixtures, без ответа запущенного Core.
Legacy `src/data/types.ts`/mock services и экраны не переведены на новые contracts.
Ни FRONTEND-002, ни её unit tests не закрывают live-integration или весь UI0.1.
