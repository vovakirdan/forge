# Forge: Control Room implementation plan

**Дата:** 11 сентября 2026
**Статус:** UI0.1/UI0.2/UI0.3/UI1.1/UI1.2/UI1.3/UI2.1 in_progress; остальные epics planned; milestones не закрыты
**Иерархия:** Milestone → Epic → будущие Task
**Baseline:** backend `1439211`; frontend — неизменённый импорт пользовательского UI

## 1. Рамка и источники

Этот workstream адаптирует Control Room к продукту Forge. Доменный смысл задают
[PRD](../forge-prd-v0.1.md) и уточняющие domain-model документы, а не mock-модели
frontend. Backend расширяется там, где отсутствует необходимый API или отдельно
согласованная продуктовая возможность. Второй scheduler/state machine в UI не создаётся.

Входные документы:

- [матрица UI ↔ backend ↔ PRD](UI_BACKEND_ALIGNMENT.md);
- [backend-план](IMPLEMENTATION_PLAN.md), [стек](STACK.md),
  [архитектура](ARCHITECTURE.md), [правила](PROJECT_RULES.md);
- [M2 closeout](2026-09-11-m2-closeout.md), [M3 acceptance](M3_ACCEPTANCE.md);
- [индекс эпиков](epics/README.md).

По решению пользователя UI идёт до installer M4. Закрытые backend M0–M3
не перенумеровываются и не открываются повторно из-за отсутствующих UI-экранов.
`UI0–UI4` — отдельные delivery milestones. `EXT1–EXT3` — design-gated расширения,
а не автоматически принятый объём UI или обязательная зависимость установщика.

Первоначально план содержал только milestones и отдельные epic-документы.
17 сентября начата [FRONTEND-001](../tasks/frontend/frontend-001-local-demo-baseline.md)
в UI0.3, затем [FRONTEND-002](../tasks/frontend/frontend-002-task-pipeline-contracts.md)
и [FRONTEND-003](../tasks/frontend/frontend-003-run-read-contracts.md)
в UI0.1. [FRONTEND-004](../tasks/frontend/frontend-004-browser-smoke.md)
добавляет воспроизводимый browser smoke в UI0.3.
[FRONTEND-005](../tasks/frontend/frontend-005-task-run-presentation.md) добавляет
pure Task/Run presentation в UI0.1.
[FRONTEND-006](../tasks/frontend/frontend-006-static-hosting-boundary.md) добавляет
static hosting proof и [browser boundary ADR](UI_BROWSER_BOUNDARY.md), без Core
connection. [FRONTEND-007](../tasks/frontend/frontend-007-live-owner-gateway.md)
добавляет Rust owner gateway, отдельный live entry, login и Project read.
[FRONTEND-008](../tasks/frontend/frontend-008-live-task-reads.md) добавляет
реальные Task list/detail и чтение pinned PipelineVersion в этом live entry:
ранний read-only срез UI1.3, не адаптация mock Board.
[FRONTEND-009](../tasks/frontend/frontend-009-live-run-reads.md) добавляет
Project-wide Run list/detail и diagnostic availability в разделе Runs:
ранний read-only срез UI2.1, без commands, SSE и body viewer.
[FRONTEND-010](../tasks/frontend/frontend-010-live-pipeline-reads.md) начинает
ранний read-only срез UI1.2: версии Pipeline, fresh detail и stage inspector;
не editor, publication или hook execution.
Это согласованные ранние срезы UI0.2 до полных gates UI0.1/UI0.3; реализация
не заменяет результаты security/browser приёмки. Статус и порядок Task —
в [индексе](../tasks/INDEX.md). Остальные Task
появятся при разборе выбранного эпика. Описание будущего gate не означает, что
он уже пройден.

## 2. Границы первой UI-поставки

### Входит в UI0–UI4

- Сохранение визуального каркаса: shell, navigation, dense tables, карточки,
  drawers, формы, timeline и общие компоненты.
- Адаптация типов, экранов и действий к lifecycle, PipelineVersion, Employee,
  Run, TaskWorkSurface, Inbox, Findings, knowledge и SystemJob.
- Недостающие scoped read projections и описания API для существующих доменов.
- Защищённый локальный browser transport, typed client, live updates,
  обработка ошибок, revisions и подтверждений команд.
- Реальные workflows управления, наблюдения и памяти; UI-тесты и отдельная
  ограниченная live-приёмка после подключения возможностей.

### Не входит автоматически

Goal/Epic/planning waves, named ResourcePool, reusable SkillPack и CodeIndex
registry требуют новых backend-контрактов. Они сохранены в EXT milestones.
Remote access, cloud/RBAC, distributed runners, marketplace, собственный RAG,
биллинг и автоматическое управление проектом LLM сюда не добавляются.

Backend M4 сохраняет Linux installer, wizard, systemd, reboot и clean-host proof.
UI-план готовит совместимый build/config contract, но не реализует установщик.

## 3. Общие правила реализации

1. UI показывает три оси: Task lifecycle, stage pinned PipelineVersion и activity
   Run/queue. Priority и properties берутся из Project; Goal/Epic необязательны.
2. Мутации идут через named Core commands. Повтор после потери ответа сохраняет
   idempotency key/payload; revision conflict требует обновления данных и решения
   пользователя, а не скрытого повторного применения команды.
3. Project ID входит в запросы, cache keys и subscriptions. Переключение Project
   закрывает прежние подписки; запоздалый ответ не попадает на новый экран.
4. Live-режим не подмешивает mock-данные. Не реализованные разделы скрыты или
   явно недоступны с причиной. Fixtures разрешены только в отдельном demo/test mode.
5. Stop receipt, physical quiescence, Task waiting и Task cancellation — разные
   факты. UI не обещает окончательную остановку до подтверждения backend.
6. Employee identity не равна provider или Run; taskless и System Runs не
   получают вымышленных Employee/Task. Ни один экран не показывает hidden CoT.
7. Policies/Decisions, submitted/accepted Artifacts, derived memory и retrieval
   status отображаются отдельно. Summary не заменяет event/artifact history.
8. Отсутствующие usage/cost/CPU/IO/familiarity данные отображаются как unknown
   или unavailable, не как ноль, успешная проверка либо выдуманный процент.
9. Все проверки Forge — тесты нашего продукта. Они не добавляют обязательные
   hooks, QA, MR или «run all tests» в проекты пользователя.
10. Credentials остаются за локальной trusted boundary. Browser не получает
    master key, provider tokens, container socket или произвольное чтение host paths.

## 4. Milestones и epics

### UI0 — Модель интерфейса и локальная API-граница

**Результат:** проверяемая основа клиента, согласованные domain projections и
локальная browser boundary; mock-прототип не выдаётся за работающий Forge.

| Epic | Результат |
|---|---|
| [UI0.1 — Domain alignment](epics/ui0-e1-domain-contracts.md) | Матрица полей/действий и frontend-модель без доменных противоречий |
| [UI0.2 — Browser transport и API client](epics/ui0-e2-browser-api.md) | Защищённый локальный доступ, полный API contract, typed command/read client |
| [UI0.3 — Frontend toolchain и test harness](epics/ui0-e3-frontend-tooling.md) | Воспроизводимая сборка, typecheck, fixtures и browser test harness |

**Exit gate:** type/contract tests различают lifecycle/stage/activity; локальный
browser проходит health/read transport smoke; forbidden origin и command replay
проверены. Сборка воспроизводится без зависимости от Lovable editor.

### UI1 — Проекты, команда, Pipeline и Task

**Результат:** оператор видит реальные объекты и выполняет поддерживаемые
операции через Core, сохраняя версионность и project scope.

| Epic | Результат |
|---|---|
| [UI1.1 — Projects, Team и profiles](epics/ui1-e1-projects-team-profiles.md) | Project selector, Employee catalog/hire, безопасная конфигурация исполнения |
| [UI1.2 — Pipeline versions и hooks](epics/ui1-e2-pipeline-versions-hooks.md) | Version-aware designer и явно настроенные необязательные hooks |
| [UI1.3 — Task и Board](epics/ui1-e3-task-board-management.md) | Создание, draft/approval, properties/priority, зависимости и корректная доска |
| [UI1.4 — Surfaces и artifacts](epics/ui1-e4-surfaces-artifacts.md) | Git/non-Git, snapshots/source policy, evidence и candidate-bound результаты |

**Exit gate:** в двух изолированных Projects создаются Pipeline, Employee и Task;
смена Project не смешивает данные. Новая версия Pipeline не меняет прежнюю Task;
Git-поля не обязательны для non-Git работы. Непройденный onboarding виден как gate,
не обходится наймом или approval. Для этого gate inference не требуется.

### UI2 — Наблюдение, коммуникация и вмешательство человека

**Результат:** из UI понятно, что выполняется, почему работа ждёт и какое
управляющее действие допустимо сейчас.

| Epic | Результат |
|---|---|
| [UI2.1 — Runs, Activity и evidence](epics/ui2-e1-run-activity-evidence.md) | Live/replay история разных Run purposes, context и безопасные logs |
| [UI2.2 — Inbox и Communication](epics/ui2-e2-inbox-communication.md) | Настоящие threads/messages, адресность и отдельные delivery/answer receipts |
| [UI2.3 — Management, resolution и recovery](epics/ui2-e3-management-resolution-recovery.md) | Stop/pause/resume, alarms, escalation queue, human resolution и recovery |

**Exit gate:** остановка не маскируется под completion; сообщение без Task не
создаёт карточку; stale human resolution/revision отклоняется с понятным UI.
SSE reconnect не теряет и не дублирует каноническую историю.

### UI3 — Knowledge loop и операционная конфигурация

**Результат:** человек управляет authoritative knowledge и видит личную/проектную
память, onboarding, Summarizer и фактические ограничения исполнения.

| Epic | Результат |
|---|---|
| [UI3.1 — Knowledge и memory](epics/ui3-e1-knowledge-memory.md) | Canonical pages, revisions, source-linked personal/project memory и поиск |
| [UI3.2 — SystemJobs и onboarding](epics/ui3-e2-system-jobs-onboarding.md) | Настройка, статус, запрос/retry и audited skip без фиктивного Employee |
| [UI3.3 — Resources и Settings](epics/ui3-e3-resources-settings.md) | Существующие caps/usage/readiness, причины ожидания и честные настройки |

**Exit gate:** страницы authority отличимы от derived memory; stale/withdrawn
источник не отображается как актуальное правило. Ошибка индекса или задержка
summary видна, но не объявляет исходную Task неуспешной. Startup-only настройки
не притворяются применяемыми онлайн, неизвестная стоимость остаётся unknown.

### UI4 — Сквозная UI-приёмка и передача установщику

**Результат:** локальный Control Room пригоден для дальнейшей ручной проверки
пользователем; installer получает стабильный контракт запуска и сборки.

| Epic | Результат |
|---|---|
| [UI4.1 — Integrated acceptance](epics/ui4-e1-acceptance.md) | Keyless regression/failure matrix и отдельно согласованный live scenario |
| [UI4.2 — Installer handoff](epics/ui4-e2-installer-handoff.md) | Build/config/runbook contract и понятная граница с backend M4 |

**Exit gate:** приняты UI1–UI3; реальные commands/read models и сохранённая
история проходят browser acceptance. Live evidence отделено от fixtures;
ограничения зафиксированы. M4 ещё должен отдельно доказать установку и reboot.

### EXT1–EXT3 — Продуктовые расширения, требующие отдельного дизайна

| Milestone | Epic | Новая возможность |
|---|---|---|
| EXT1 — Planning | [EXT1.1](epics/ext1-e1-goals-epics.md), [EXT1.2](epics/ext1-e2-bounded-planning.md) | Optional Goal/Epic и bounded planning proposals/waves |
| EXT2 — Resource pools | [EXT2.1](epics/ext2-e1-resource-pools.md) | Именованные pools, membership и admission/drain semantics |
| EXT3 — Reusable context integrations | [EXT3.1](epics/ext3-e1-skill-packs.md), [EXT3.2](epics/ext3-e2-code-intelligence.md) | SkillPack catalog и подключаемый code intelligence |

**Статус всех EXT:** design-gated. Сначала согласуются доменные контракты и
минимальный объём, затем нарезаются Task. Наличие файлов не даёт разрешения
добавлять эти домены попутно. EXT не блокируют UI4 или M4; пользователь может
явно изменить порядок после обсуждения соответствующего эпика.

## 5. Зависимости и параллельность

Таблица — полный набор hard dependencies между новыми эпиками. Backend M0–M3
являются общей входной предпосылкой; approvals EXT — дополнительные внешние gates.

| Epic | Depends on |
|---|---|
| UI0.1 | — |
| UI0.2 | UI0.1, UI0.3 |
| UI0.3 | — |
| UI1.1 | UI0.1, UI0.2, UI0.3 |
| UI1.2 | UI0.1, UI0.2, UI0.3 |
| UI1.3 | UI1.1, UI1.2 |
| UI1.4 | UI1.3 |
| UI2.1 | UI1.3, UI1.4 |
| UI2.2 | UI1.1, UI1.3 |
| UI2.3 | UI2.1, UI2.2 |
| UI3.1 | UI1.1, UI1.3, UI1.4 |
| UI3.2 | UI3.1, UI2.1, UI2.3 |
| UI3.3 | UI1.1, UI2.1, UI2.3 |
| UI4.1 | UI1.1, UI1.2, UI1.3, UI1.4, UI2.1, UI2.2, UI2.3, UI3.1, UI3.2, UI3.3 |
| UI4.2 | UI4.1 |
| EXT1.1 | UI1.3 |
| EXT1.2 | EXT1.1, UI2.2 |
| EXT2.1 | UI3.3 |
| EXT3.1 | UI1.1, UI3.2 |
| EXT3.2 | UI3.1 |

Основной порядок: `UI0 → UI1 → UI2/UI3 → UI4 → backend M4`.
После UI0 можно параллельно разбирать Team и Pipelines. После Task/surface
contracts — Runs, Inbox и Knowledge. Одновременные правки общей OpenAPI schema,
command client и project cache координируются одним владельцем контракта.

Исключение от 17 сентября: после FRONTEND-001–005 пользователь согласовал
FRONTEND-006 (ADR/static proof), затем FRONTEND-007 (owner gateway/session и
отдельный health/Project read экран), не ожидая полного закрытия UI0.1/UI0.3.
Это не обход их exit gates и не разрешение подключать остальные features без
security proof. Полный API client, commands/replay/conflicts и SSE требуют
следующих отдельно нарезанных Task; FRONTEND-007 не закрывает весь UI0.2.
Следом согласован FRONTEND-008, ранний scoped Task read срез UI1.3 поверх
проверенной границы, без management commands и новых Core API. Полные gates
UI0/UI1 и зависимости остальных частей UI1.3 сохраняются.
FRONTEND-009 аналогично начинает UI2.1 существующими Run reads: два раздела
Tasks/Runs, ручное обновление и ограниченная диагностика. Contracts FRONTEND-003
и presentation FRONTEND-005 переиспользуются без новых Core schemas. Полные
зависимости UI2.1, включая UI1.4 для body/context viewer, не отменяются.
FRONTEND-010 аналогично начинает UI1.2 третьим разделом Pipeline versions.
Список версий не объявляется полным unique-Pipeline catalog и не содержит
выдуманного pinned Task usage. Catalog metadata в read DTO изменяема,
definition версии immutable; policies с `null` остаются «не настроено».
Редактор и named commands остаются будущим объёмом UI1.2.

Исключение от 18 сентября: FRONTEND-011 добавляет первую command из UI0.2/UI1.3
в существующую live Task-карточку — только title/description draft через
`amend_draft`. Scope включает frozen retry, revision refusals, безопасный
gateway POST и keyless proof; создание Task, Board, остальные commands и SSE
не входят. Полные epic gates сохраняются; evidence находится в Task.

Следом согласован FRONTEND-012: отдельный canonical PriorityScheme read
UI0.1/UI1.1 и названия приоритетов в live Task UI1.3. ProjectView и Task
mutations не расширяются. Это частичное закрытие `project-task-catalogs`, не
редактор схемы и не полный gate UI1.1. Core пока создаёт три стандартных
уровня; 1/10/retired fixtures остаются явно synthetic.

FRONTEND-013 добавляет ранний command-срез UI1.3/UI0.2: назначение active
priority существующей non-terminal Task через `set_task_priority` с fresh
baseline, receipt/conflict и exact retry. Текущий Run не прерывается;
очередью управляет Core. Это не создание Task, настройка схемы или полный gate.

FRONTEND-014 продолжает UI1.3/UI0.2 созданием draft Task через `create_task`:
явный kind и точная PipelineVersion, active priority, optional DoD, properties:{}.
Receipt возвращает новый ID; conflict сохраняет ввод, unknown replay не создаёт
вторую Task, failed readback не отменяет receipt. Approval, property editor,
исполнение и конфигураторы остаются вне среза; полный epic gate не закрывается.

FRONTEND-015 продолжает UI1.3/UI0.2: DoD edit через `amend_draft` и отдельный
`approve_task` с новым явным подтверждением после conflict. UI показывает
сохранённый pin и gate, не меняет gate и не предполагает ready после approval.
Приёмка проходит без провайдеров; properties/cancel/execution controls и полные
epic gates остаются за пределами среза. Evidence — в отдельной Task.

FRONTEND-016 продолжает UI1.3/UI0.2: project cancellation catalog, отдельная
команда отмены и сохранённые reason/note/actor/time в карточке. Receipt отделён
от readback и от физической остановки Run. Настройка каталога, property editor,
execution controls и полные epic gates остаются вне среза; evidence — в Task.

Run-приёмка разделяет реальные Core reads с M0 fake execution и synthetic
cases пяти purposes. Ни то ни другое не является новым provider proof. Browser
limits не закрывают gap серверной Run pagination, пока Core загружает все строки
Project перед разбиением на страницы. Фактический результат проверок — в Task.
Pipeline version list содержит full DTO и получает лимит 1 MiB; Task/Run lists
сохраняют 64 KiB, все эти details — 1 MiB. Core загружает все версии и делает
catalog read для каждой (N+1) до выдачи страницы. Законный page может превысить
лимит: явная size error не подменяется empty list, усечением или adaptive retry.

На ближайший разбор: оставшиеся gates UI0.1/UI0.3 и implementation UI0.2;
после их gates — UI1.1 и оставшийся объём UI1.2.
Это очередь эпиков для будущей нарезки, не пять заранее выданных coding tasks.

## 6. Проверки и evidence

- Каждый эпик получает component/contract tests вместе с поведением, а не в UI4.
- UI0.3 фиксирует реальные frozen install/build/typecheck/lint/test команды.
  До его выполнения команды, перечисленные в эпиках, являются требованиями
  к harness, а не утверждением о существующих scripts.
- Backend additions проверяются по PROJECT_RULES: named command conformance,
  scope/revision/replay, migrations при необходимости и совместимость CLI.
- Browser fixtures и keyless Core integration не доказывают работу провайдера.
  Live Run требует отдельного выбора точной модели, credentials, лимита Runs,
  времени/allowance и явного согласия оператора. Прошлое разрешение M3 не переносится.
- Сквозной сценарий использует отдельный mock Git repository и private session;
  исходный Forge repository и личные provider auth files не изменяются.
- Performance проверяется на 1000 Task/20 Employee без полного скачивания
  логов/объектов; ориентир update latency — 2 секунды в здоровом локальном режиме.
  Недоступность транспорта отображается явно, без обещания этого SLA во время outage.

## 7. Definition of ready для будущей Task

Перед нарезкой выбранного эпика уточняются DTO/commands, error semantics,
владельцы изменений, тестовый harness и безопасный validation path. Task имеет
выходной результат, non-goals, зависимости, DoD и проверку. Оценка размера эпика
не является обещанием количества дней или provider Runs.

UI0.2 сначала закрывает browser session/Origin/CSRF и hosting decision; UI0.3 —
воспроизводимость импортированного toolchain. Эти вопросы не прячутся в обычной
задаче «подключить API». EXT сначала проходят собственный design/approval gate.
Утверждение этого roadmap не означает выполнение или закрытие какого-либо epic.
