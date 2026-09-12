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

Некоторые обзорные формулировки PRD старше уточняющих контрактов. Перед задачей
на соответствующий UI нужно согласовать текст документов с действующей моделью:

- PRD §9.4/FR-021 описывает pin «при approval», тогда как
  [M2 Pipeline management](M2_PIPELINE_MANAGEMENT.md) выбирает версию уже в
  `create_task`; смена default не меняет существующий draft.
- PRD §23 ещё упоминает Integration rebase. [M2 Git integration](M2_GIT_INTEGRATION.md)
  и PRD §14.4 требуют exact-candidate/CAS и не выполняют скрытый rebase за Employee.

Это doc-alignment работа UI0.1, не разрешение менять работающий Core под старую
строку PRD.

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
