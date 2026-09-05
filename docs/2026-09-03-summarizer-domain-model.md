# Forge: абстракция Summarizer

**Статус:** Draft 0.1, обсуждается
**Дата:** 3 сентября 2026
**Область:** derived summaries и memory entries из событий и Artifacts Task.

## 1. Определение

`Summarizer` — project-scoped вспомогательный worker, который превращает
canonical историю Task в короткие, читаемые и пригодные для retrieval summaries.
Он может использовать локальную модель, внешний provider или другой механизм;
доменный контракт от реализации не зависит.

Summarizer не является участником Pipeline. Он не меняет Task, Artifact,
acceptance, lifecycle, stage, queue, Run или назначение Employee. Его outputs —
только derived projections и memory entries с явными источниками.

## 2. Входы и пробуждение

Core публикует immutable domain event после значимого действия: прикрепления
Artifact, StageOutcome, ArtifactAcceptance, изменения stage/lifecycle, завершения
Run, review outcome, escalation или её resolution. Надёжный event/outbox не
обязан создавать один вызов модели на каждый Event: он передаёт source sequence
в `SummarizationPolicy`, которая создаёт или расширяет coalesced
`SummarizationJob` для затронутой Task.

При завершении или остановке stage Core синхронно создаёт canonical
`TaskHandoff`: actor, producer scope, optional Run/ResolutionAssignment, outcome,
timestamp, `source_stage_id`, `target_stage_id` и ссылки на Artifacts. При
force-stop или lost environment его `outcome = interrupted`, а record ссылается
на RunIncident и partial evidence без Pipeline transition. Handoff доступен
следующему Employee сразу и не зависит от Summarizer. Только после этого или
параллельно с ним event/outbox ставит асинхронный Job.
У entry stage первой Task handoff не создаётся: её Run использует initial Task
context без предыдущей stage history.

Job содержит `task_id` и covered source event sequence. Summarizer может быть
запущен несколько раз для одного диапазона; он дедуплицирует работу по sequence
и пересобирает summary из canonical данных. Сбой summarizer не откатывает и не
блокирует исходную Task operation: job повторяется отдельно.

Summarizer читает только разрешённые ему Task data: TaskSpec, lifecycle/stage,
прикреплённые Artifacts и их acceptance, доступные Run outcomes, Events и уже
созданные summaries. Для observation он может также читать отобранные
source-linked `RunEvent` и redacted, bounded `TechnicalLogExcerpt`; это не даёт
ему свободный доступ к raw transcript, provider secrets, private memory другого
Employee или закрытому content вне scope Task.

`TechnicalLogExcerpt` — immutable технический источник, а не mutable ссылка на
"последние логи". Он содержит `excerpt_id`, `run_id`, точный source event/log
range либо список source ids, visibility, redaction policy version, content hash,
размер и время создания. Сам content может храниться отдельно. Изменение
redaction или source range создаёт новый excerpt, поэтому Summarizer может
воспроизводимо дедуплицировать Job по источникам.

### 2.1. Capacity policy

`SummarizationPolicy` принадлежит Project и задаёт, какие source events важны,
как они coalesce, какие summaries имеют приоритет и какой resource/cost budget
доступен Summarizer за период. Policy также задаёт разрешённые типы technical
excerpts и их максимальный объём. Поэтому completion stage, interruption и
длительное исследование могут быть приоритетнее частых progress events.

При исчерпании budget policy выбирает `defer`, дальнейшее coalescing или
`skip_derived` для низкого приоритета. Это не теряет canonical события: следующий
разрешённый Job может покрыть более широкий source range, а UI показывает
freshness summary. Ни одна Task transition, handoff или management action не
ждёт summarization.

## 3. Выходы

| Output | Назначение | Обязательные метаданные |
|---|---|---|
| `TaskSummary` | Быстрый handoff для следующего Employee, reviewer, System Manager и Human | `task_id`, covered event sequence, generated time, source Artifact/Event links, derived marker |
| `EmployeeMemoryEntry` | Краткая личная заметка о выполненной работе или handoff автора Run | employee id, task id, source links, visibility `personal`, derived marker |
| `ProjectKnowledgeEntry` | Поисковая запись о проверенных путях, фактах и ограничениях Project | Project id, source links, trust `derived` или `accepted`, generated time |

Forge canonical store хранит scope, visibility, revision, content hash и source
links каждого memory/knowledge entry. AgentMemory служит сменной retrieval
projection, а не source of truth: outbox синхронизирует в него новую revision,
а Forge rechecks canonical visibility/revision после поиска. Удаление или смена
visibility создаёт новую index operation; сбой index не отменяет canonical entry.

`TaskSummary` описывает canonical факты: что произошло, на какой stage находится
Task, какие Artifacts созданы/приняты, какие проверки заявлены в outcome, что
вернул review и какие blockers/escalation активны. Утверждение Employee
сохраняется с attribution: «Bob сообщил, что проверки зелёные», а не «проверки
зелёные». Summary не объявляет предположение принятой истиной и не скрывает
отсутствие решения.

Например, после work stage summary может сослаться на MR и SHA, назвать
изменённые области и проведённые проверки. После review он фиксирует номер
итерации и замечания. После незавершённого research он перечисляет проверенные
варианты и evidence, явно указывая, что Decision не принят.

## 4. Связь с памятью и наблюдаемостью

Task остаётся human-readable источником своей хронологии: человек видит в ней
прикреплённые Artifacts, их авторов, stages и acceptance. Summary не заменяет
эту ленту, а сокращает её для быстрого контекста.

Следующий Employee получает `TaskSummary` как часть retrieval context, а не
длинный пересказ в employee prompt. Это позволяет вернуть Task прежнему
Employee или передать другому без ручного объяснения истории. System Manager
читает summary вместе с authoritative lifecycle/stage Core; он не делает
management decision только по устаревшей сводке.

ProjectKnowledgeEntry с trust `derived` полезен для повторного исследования, но
должен показывать ссылки на Artifacts и свой trust. Только accepted knowledge
может быть представлено как утверждённый вывод Project.

Derived observation может ссылаться на RunEvent или TechnicalLogExcerpt и
формулирует только наблюдаемый факт: «в Run X команда Y завершилась ошибкой Z,
после чего был выбран путь W». Human или уполномоченный Manager отдельно
принимает такое observation как project operating rule; Summarizer не выводит
правило из единичного incident сам.

## 5. Работа во время project stop

`stop_project_execution` не отменяет canonical Events и не запрещает их чтение,
но останавливает запуск новых SummarizationJob. Уже созданные Jobs остаются
pending. После явного `start_project_execution` они снова допускаются к работе
по policy очереди; Task lifecycle от их задержки не зависит.

## 6. Границы и надёжность

1. Summarizer не имеет команд, меняющих canonical доменные объекты.
2. Каждый output хранит source links и covered event sequence; UI и Employee
   могут показать его свежесть.
3. Повторный Job с теми же входами не создаёт дубликат summary или memory entry.
4. Новый event делает предыдущий summary potentially stale, но не удаляет его
   историю.
5. Summarizer не может принять Artifact, разрешить escalation, выдать Lease,
   изменить priority или запустить Pipeline.
6. Derived output сохраняет visibility и redaction rules входных источников.
7. Если summarization недоступен, Core продолжает работу; observer видит
   последнюю summary и её покрытие, а canonical Task history остаётся доступна.
8. Summarizer выполняется в пределах `SummarizationPolicy` budget и не получает
   неограниченный доступ к raw technical logs.
9. Employee не пишет persistent memory напрямую; Human/Manager создаёт
   authoritative knowledge только именованной командой.
