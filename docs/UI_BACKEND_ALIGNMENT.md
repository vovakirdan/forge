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

Текущий срез: FRONTEND-007 добавил отдельный owner live entry;
[FRONTEND-008](../tasks/frontend/frontend-008-live-task-reads.md) подключает
реальные Task list/detail и pinned PipelineVersion. Матрица ниже описывает
импортированный baseline; его mock Board/Team ещё не подключены к Core.

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

## 9. Run read contracts FRONTEND-003

**Сверка:** 17 сентября 2026, Core из baseline `a1c62e6`.
[FRONTEND-003](../tasks/frontend/frontend-003-run-read-contracts.md) продолжает
изолированный read layer для двух существующих маршрутов:
`GET /v1/projects/{project_id}/runs` и `.../runs/{run_id}`.
Источник wire-формата — [RunView/RunDetailView](../crates/forge-core/src/http/views.rs),
[assignments](../crates/forge-domain/src/assignment.rs),
[SystemJob assignment](../crates/forge-domain/src/system_job.rs),
[Run states](../crates/forge-storage/src/model.rs) и
[diagnostics projection](../crates/forge-storage/src/run_evidence.rs).

### 9.1. Поля Run и ownership

| Поле / смысл | Источник | Граница представления |
|---|---|---|
| Run identity, attempt, RunSpec version | RunView | RunSpec version — положительное число, не название provider; raw RunSpec отсутствует |
| Project scope | Параметр маршрута | `project_id` не добавляется в DTO; будущий клиент сохраняет scope отдельно |
| `assignment.purpose/owner` | ExecutionAssignment | Пять purposes; owner каждого задаёт отдельные ссылки, не универсальную Task |
| Верхние `task_id/stage_id` | RunView, только TaskStage | Для остальных purposes null; Task/stage внутри Hook owner остаются контекстом, не writer authority |
| `employee_id` | RunView | Nullable, в том числе Hook и SystemJob; несколько Run одного Employee сохраняются отдельно |
| `desired_state` | Core request/result state | `stop_requested` и `force_stop_requested` не подтверждают физическую остановку |
| `observed_state` | Supervisor observation | `unknown`, `lost`, `running`, `stopping`, `stopped`, `failed`, `provisioning` не заменяют Task lifecycle |
| Fence/epoch/last sequence | RunView | Fence/epoch положительные, sequence допускает ноль; все значения должны быть safe integers |
| Pagination | ListView | `items` обязателен, конечный `next_cursor` отсутствует, не равен null |

`ExecutionAssignment` сохраняет текущие поля owner: TaskStage — Task/queue/stage;
Communication — assignment/thread/source message; Resolution — assignment/
escalation/lease generation; Hook — invocation/contextual Task/PipelineVersion/
stage visit/candidate; SystemJob — job/attempt/generation/kind. Последний имеет
виды `summarization` и `onboarding`, но не Employee-владельца.

Task lifecycle, Pipeline stage и Run activity читаются независимо. Сочетание
waiting Task с ещё running/stopping Run не преобразуется в другой lifecycle.
Процесс остановился — не значит, что результат Task принят или она стала done.
Схемы проверяют wire shape, не дублируют Core transitions/authority checks.
Required nullable fields не становятся optional. Additional fields сохраняются,
включая специальные JSON keys; coercion/defaults/нормализация не применяются.

### 9.2. Диагностика без интерпретации отчётов

Run detail содержит обязательный `diagnostics` object. Его внешняя структура:

- `runtime_report`, `handoff`, `proxy_usage`, `git_source` — JSON object либо null;
- `incidents`, `evidence` — массивы JSON objects;
- `streams` — массив объектов `{stream: string, incomplete: boolean}`.

Все семь полей обязательны; SQL projection выводит null/пустые массивы, если
соответствующих записей ещё нет. Вложенные отчёты остаются JSON: эта задача не
определяет их detailed schemas, units, quality, доступность body или acceptance.
Отсутствующее usage не заменяется нулём; неполный stream не становится полным.
`git_source` сам по себе не создаёт generic WorkSurface view или путь к checkout.

Schemas валидируют структуру, но не выполняют redaction и не выдают разрешений.
Даже прошедший проверку JSON нельзя считать безопасным HTML, доверенным URL,
host path или указанием скачать object key. Browser boundary, safe body reads и
rendering остаются в UI0.2/UI1.4/UI2.1; этот слой никуда не подключается сам.

### 9.3. Named gaps и OpenAPI drift

| Gap | Что отсутствует / расходится | Владелец следующего шага |
|---|---|---|
| `employee-catalog-read` | Нет generic Employee list/detail; threads/memory reads их не заменяют | UI0.1 согласует projection, UI1.1 открывает scoped reads |
| `employee-runtime-profile-read` | Нет полного safe profile/readiness view; нельзя получать его из credentials или одного Run | UI1.1; без новых provider Runs для health |
| `task-work-surface-read` | Нет generic TaskWorkSurface view; source-policy/snapshot/candidate endpoints дают отдельные факты | UI1.4; без обязательного Git и host paths |
| `run-runtime-metadata-read` | RunView не отдаёт timestamps, provider/model, heartbeat или environment/surface metadata | UI2.1 согласует safe projection; не вычислять из ID/RunSpec version |
| `run-diagnostics-detail-contracts` | Вложенные reports/receipts/usage/handoff/GitSource пока opaque JSON | UI2.1/UI1.4 определяют typed bodies, bounds, provenance и безопасное чтение |
| `openapi-run-system-job` | ExecutionAssignment не содержит `system_job`; описание `employee_id` допускает null только у Hook | UI0.2 синхронизирует OpenAPI с M3, frontend следует serializer |
| `openapi-run-spec-version` | Описание RunSpec заканчивается v5, хотя есть v6 Task и v7 SystemJob | UI0.2; frontend не ограничивает положительный u16 списком версий |
| `openapi-run-timestamps` | OpenAPI описывает optional created_at/updated_at, serializer их не выводит | UI0.2; отсутствие полей не маскируется synthetic timestamps |
| `openapi-run-counter-bounds` | Fence/epoch minimum 0 против canonical SQL constraints `>0` | UI0.2; frontend принимает положительные safe integers |
| `openapi-run-pagination` | OpenAPI допускает null cursor; ListView пропускает поле на последней странице | UI0.2; frontend различает null и omission |

Изменений API/OpenAPI/backend/PRD здесь нет. Synthetic tests доказывают поведение
контрактов, не live Core integration, безопасность browser boundary или поддержку
полного Employee/Surface API. Экраны и demo services остаются прежними.

## 10. Task/Run presentation FRONTEND-005

[FRONTEND-005](../tasks/frontend/frontend-005-task-run-presentation.md) добавляет
чистый [presentation layer](../frontend/src/presentation/index.ts) поверх уже
валидированных DTO. Каждая функция возвращает `{ source, presentation }`;
`source` — исходный объект без клонирования и мутаций. Его надо считать read-only
и пересчитывать presentation при смене данных. Это не дополнительная wire schema.

| Builder | `presentation` | Данные, оставшиеся в `source` |
|---|---|---|
| `presentTaskSummary` | `kindLabel`, `lifecycleLabel`, `stage: TaskStageResolution` | Priority stable ID, pinned version, identity/revision и дополнительные JSON fields |
| `presentTaskDetail` | Summary presentation + `loadedWaitConditionCount`, `loadedArtifactCount` | Properties, все waits и artifacts без интерпретации |
| `presentRun` | `purposeLabel`, `desiredStateLabel`, `observedStateLabel`, nullable `systemJobKindLabel` | Typed assignment/owner, nullable Employee ID и прочие Run fields |
| `presentRunDiagnostics` | Наличие четырёх nullable reports, `loadedIncidentCount`, `loadedEvidenceCount`, `loadedIncompleteStreamCount` | Полные возвращённые reports, incidents/evidence и stream flags |

Stage resolver сохраняет `no_stage` и три причины `unavailable`; latest/default
и soft-delete не заменяют pinned graph. Подписи закрытых enum проверяются
компилятором на полноту. Lifecycle Task, стадия Pipeline, desired state Run и
observed state Run остаются независимыми; запрос остановки не подтверждает её.

Счётчики — длины загруженных массивов, не totals. У summary нет detail counts.
`null` report недоступен, `{}` присутствует, но наличие не доказывает acceptance,
правдивость, успешное выполнение или полноту истории. Нет извлечения cancellation
reason из properties, Employee aggregation, current Run, постоянного assignee,
priority rank или безопасного HTML/URL из JSON.

`just ui-test-presentation` запускает отдельные synthetic unit tests. В срезе
FRONTEND-005 слой ещё не подключался к экранам; FRONTEND-008 использует его для
read-only Task UI. Сами builders не вызывают API, команды или side effects.
Именованные gaps разделов 8–9, UI0.1 и UI0.3 остаются открытыми.

## 11. Live Task reads FRONTEND-008

Отдельный `src/live` использует существующие Project/Task/PipelineVersion DTO,
не legacy demo types. Gateway расширен только scoped GET; новые Core endpoints,
DTO, migrations и domain state отсутствуют.

| Данные | Live UI | Ограничение |
|---|---|---|
| TaskSummaryView | Список по 20, Previous/Next/Refresh | Без total/filter/auto-pagination; stage/priority — raw IDs |
| TaskDetailView | Plain-text поля, properties JSON, каждый wait | Нет реконструкции cancellation reason, assignee или Run state |
| ArtifactView | ID/kind/title/created_at | Body/metadata links не открываются; detail целиком bounded 1 MiB |
| PipelineVersionView | Стадия выбранной Task по свежему detail pin | Не default/latest; ошибка не скрывает Task; no_stage отдельно |

Ограничения DTO разделов 8–9 остаются явными. В частности, approval неизвестных
properties требует Project schema, но её публичная команда настройки ещё не
открыта. Fixture проверяет typed properties в draft через named CreateTask;
не меняет schema напрямую в БД. Это не реализация schema editor UI1.1/UI1.3.

Gateway проверяет новые IDs/query/bounds; live API валидирует Zod и совпадение
запрошенных detail/version IDs. Scope находится в route/query key, не выдуманном
поле DTO. Неактивные page/detail/pipeline queries удаляются при навигации,
поздние ответы отменяются; полные auth/transport limits остаются из FRONTEND-007.
Настоящие Core и fault-injection проверки перечислены в
[Task evidence](../tasks/frontend/frontend-008-live-task-reads.md).
