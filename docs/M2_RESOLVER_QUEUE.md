# M2: очередь эскалаций и ограниченные решения

## Контракт

Escalation — отдельный вопрос менеджменту, не новая Task и не стадия Pipeline.
Для Task он фиксирует Project, Task, immutable PipelineVersion, stage_id,
stage_visit и собственный `escalation_pending` wait. Вопрос, автор и выбранная
версия маршрута неизменяемы; revision, generation и результат меняются отдельно.

ResolverRoute содержит упорядоченный список до 32 Employee и deadline для
Employee assignment: 30–86400 секунд. Изменение маршрута увеличивает revision
каталога; уже созданные Escalation используют свой snapshot. Пустой маршрут и
отсутствие маршрута означают Human fallback. `action_approval` всегда требует
Human: Employee не может одобрить опасное действие обычным техническим ответом.
Отмена Task остаётся отдельной `cancel_task`.

ResolutionLease принадлежит SystemManager, не resolver. Generation и текущий
assignment исключают ответ прежнего resolver. Координация выполняется под
транзакционной блокировкой Project; `held_by` — атрибуция управляющего решения,
не вечная блокировка процесса. Для Human `expires_at = null`: вопрос не исчезает
по таймеру. Для Employee deadline обязателен. История законченных assignment
не редактируется и не удаляется.

## Решение не заменяет результат стадии

Ответ содержит summary и один disposition:

| Disposition | Эффект |
| --- | --- |
| `continue_stage` | Снимает только исходный escalation wait; остальные ожидания сохраняются. |
| `needs_management_change` | Записывает ответ; Task ждёт отдельной management command. |
| `forward_to_human` | Завершает старый assignment и создаёт Human assignment с новым generation. |

`recommended_outcome_key`, если указан, должен присутствовать в исходной стадии
закреплённой PipelineVersion. Это рекомендация, не переход. Resolver не может
выдумать `planning`, завершить Employee stage, засчитать себе DoD или отменить
Task. Обычный submission / разрешённый внешний outcome по-прежнему проверяет
собственные артефакты и правила Pipeline.

Принятый ответ — immutable `decision_record`, прикреплённый к исходной Task с
producer `resolution_assignment`, assignment ID и generation. Проверяется
структура/область ответа, не его смысловая истинность. Artifact, Task,
assignment, Escalation, Event, outbox и command receipt фиксируются атомарно.

## Именованные команды

- `configure_resolver_route`: `route_key`, `employee_ids`,
  `assignment_timeout_seconds`; Project expected revision защищает каталог.
- `raise_escalation`: `task_id`, `expected_task_revision`, `category`, `question`,
  необязательный `route_key`. Добавляет ожидание и запрашивает graceful stop
  исходных Task Runs; физические reservations сохраняются.
- `submit_human_resolution`: `escalation_id`, `expected_escalation_revision`,
  `assignment_id`, `lease_generation`, `answer`. Только Human, даже если другому
  actor вручную выдать capability с таким именем.
- `reroute_escalation`: `escalation_id`, `expected_escalation_revision`, `reason`.
  Явно передаёт вопрос человеку с новым generation и сохранением истории.

Команды используют обычные Project scope, expected revision, capability,
idempotency key и audit. Ответ отклоняется при изменившемся stage visit,
исчезнувшем wait, terminal Task или устаревшем assignment/generation. Изменение
приоритета само по себе не уничтожает вопрос: source связан с visit и wait,
а не с каждым увеличением Task revision.

Обычные `resume_task` и `schedule_task_resume` не могут снять ожидание,
принадлежащее canonical Escalation. Due timer также проверяет владельца wait
и фиксирует `resolution_required`, не меняя Task. Старые M1
`escalation_pending` без Escalation-записи сохраняют прежний ручной resume.

## Исполнение Resolver

Employee получает отдельный `ResolutionRunSpec` v4 и неизменяемый
ResolutionContext. Это настоящий Run выбранного Employee в общих таблицах
Run, Lease и physical reservations, но без Task, stage, queue entry или
TaskWorkSurface в его исполнительной идентичности. Workspace — только личный
scratch; Project board и исходные сведения доступны через scoped Gateway.

Dispatcher перебирает snapshot маршрута по порядку. Занятый, выключенный или
ненастроенный Employee пропускается. Когда кандидаты исчерпаны, создаётся Human
assignment: без Run, вымышленного Employee и расхода Employee capacity.
Project stop/recovery hold закрывают новые admissions. Generic Employee stop
включает Resolution Runs; остановка контекстной Task не притворяется владением
её Resolver Run.

Resolver имеет `resolution.read`, `resolution.submit`, `resolution.decline` и
Project-scoped `board.list` / `task.read`. У него нет Task submission, writer,
management или approval полномочий. V4 исполняется одним turn даже при
LiveInput-capable профиле Employee; effective launch не изменяет сохранённый
профиль и не включает native input transport.

Deadline, техническая остановка или отказ завершают прежний assignment.
Старый ответ теряет authority; следующий Employee не запускается до положительного
подтверждения физической остановки прежнего Run. Отзыв Lease сам по себе не
освобождает sandbox. Отмена/смена исходной стадии превращает незавершённый вопрос
в `superseded`, без выдуманного ответа и Artifact.

Проектная выборка исключает вопросы с активной Lease или неподтверждённой
физической остановкой до ограничения размера страницы. Даже 256 старых
удерживаемых запусков не заслоняют следующий готовый вопрос. После получения
Project lock эти условия проверяются повторно. Общий watchdog всё ещё видит
удерживаемые вопросы и обходит их по кругу для recovery.

`continue_stage` отклоняется, если исходный Task Run ещё удерживает активную
Lease или физическую reservation. `needs_management_change` может записать
ответ, пока остановка завершается, но не разрешает продолжение.

## Вопрос из Communication

Gateway `escalation.raise` фиксирует либо точный Task stage visit, либо
CommunicationAssignment + source message/thread + Run/fence/epoch. Во втором
случае беседа переходит в hold, но optional context Task не получает ожидание
и не меняет состояние. Ответ — Project-level `decision_record` с тем же
ResolutionAssignment producer.

Для Communication `continue_stage` означает принятое намерение продолжить,
а не автоматический новый Run. M2 не умеет доказать оставшийся общий allowance
между попытками, поэтому беседа остаётся в hold до явного `retry_communication`
менеджера. Неразрешённая Escalation блокирует этот retry. Даже принятый ответ
не снимает physical/recovery/admission checks и не возобновляет старый процесс.
Новая попытка того же CommunicationAssignment получает canonical ответы в
своём контексте инструкции. Сам факт ответа никогда автоматически не пополняет
исчерпанный бюджет.

`human.request` сохраняет совместимый формат вопроса и receipt, но новые
вызовы создают canonical Human Escalation. Receipt сохраняется до запроса stop.
Точный повтор stop-producing вызова по HTTP/MCP возвращает тот же receipt,
даже если собственная Lease уже отозвана; новый message ID, изменённый payload
или другая fencing scope не получают новых полномочий.

## Проверки и границы

Shared conformance проверяет PostgreSQL/reference atomicity, idempotency,
Human authority, source wait, route snapshots и запрет обхода через resume/timer.
`m2_resolution_runs` проверяет реальные PostgreSQL/Core/UDS с ручным Supervisor
и синтетической авторизацией без обращения к провайдеру: v4 one-shot,
generation/deadline, physical barrier, занятость/stop Employee, Human fallback,
HTTP/MCP replay и отсутствие Task-stage полномочий. Это не paid-provider proof.
Отдельные сценарии проверяют насыщенную очередь и полный путь Communication:
Project-level ответ, неизменность context Task, отсутствие автоматического
рестарта и новая fenced попытка только после stop и команды менеджера.

Отдельного HTTP read-view очереди пока нет: оператор читает canonical Event
stream. Resolver читает собственное назначение через `resolution.read`.
Завершение этой вертикали не заменяет остальные критерии TASK-026 / M2.
