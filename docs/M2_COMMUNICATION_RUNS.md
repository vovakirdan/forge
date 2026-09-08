# M2: taskless Communication Run

Employee Inbox не создаёт скрытую Task. Сообщение с `target.kind = inbox`
порождает source-bound Communication assignment в той же транзакции, что и
само сообщение. Собственные сообщения Employee-получателя — только output и
нового запуска не создают. Сообщения других отправителей, включая человеческий
follow-up Reply, — новый input. Это позволяет продолжать разговор без бесконечной
очереди ответов самому себе. ID assignment совпадает с ID исходного сообщения.

## Владение и версии

`ExecutionAssignment` — закрытый discriminator владельца Run:

| Purpose | Владелец | Доступность |
| --- | --- | --- |
| `task_stage` | Task + queue entry + stage | M0/M1, RunSpec v1/v2 |
| `communication` | Assignment + Employee thread + source message | M2, RunSpec v3 |
| `resolution` | Escalation + ResolutionLease generation | M2, RunSpec v4 |
| `hook` | Hook invocation + pinned Task/stage/candidate context | M2, RunSpec v5 |

У всех четырёх purposes общие `runs`, `leases`, fencing, environment epoch,
physical reservations, наблюдатель и evidence. Provider Runs также используют
общую доставку credentials и provider budgets. Hook выполняет явную команду
без Employee, провайдера и его credentials; его Task context не даёт writer lease.
Таблица `communication_assignments` содержит очередь и результат разговора,
а не вторую Run ledger. SQL проверяет отдельную полную ветку ownership для
каждого purpose. Gateway разрешает только инструменты соответствующего контекста;
Hook не получает agent Gateway.

Исторические RunSpec v1/v2 не переписываются. V3 имеет собственный validated
контекст, исходное сообщение и явный Protobuf owner; `task_id` и `stage_id` в
transport пусты. Несовпадение owner/schema или неизвестная версия отклоняются.
`RuntimeLaunchSpec` — только внутреннее общее представление после декодирования,
не новая сериализация старого RunSpec.

В HTTP Run view добавлена `assignment`; `task_id` и `stage_id` nullable.
Опциональный `context_task_id` в conversation context — ссылка для чтения,
не владение Task, стадией или рабочими файлами.

## Admission и окружение

Run может начаться только при открытом Project gate, включённом Employee,
настроенном runtime и свободной общей capacity. По умолчанию
`max_concurrent_runs = 1` для всех Employee purposes вместе: TaskStage,
Communication и Resolution. Hook не занимает место Employee. Значение `3` позволяет,
например, два Task Run и один Communication Run. Активная Lease и её физическая
reservation занимают одно место, а не два. Неопределённое физическое состояние
продолжает занимать capacity после отзыва логической Lease.
Дополнительно действуют общие [host/Project/account лимиты](M2_ADMISSION_LIMITS.md).

В одном thread одновременно исполняется максимум один Communication Run.
Следующий не стартует, пока предыдущая физическая reservation не освобождена.
Сообщения остаются в PostgreSQL, когда Employee занят или Project остановлен.

Communication использует выбранный runtime profile, prompts, credentials и
лимиты Employee, но явно получает `SurfaceSpec::None`: только собственный
scratch/runtime, без Task Git или filesystem surface. Shell остаётся внутри
sandbox. Поддержаны Codex CLI, Claude CLI и OpenCode API lanes. Профиль может
использовать [native session driver](M2_NATIVE_INPUT.md), но Communication
остаётся ограниченной одним source message: последующие общие сообщения
получают отдельные assignments, а не попадают в чужой текущий разговор.

## Gateway и завершение

Доступны `inbox.list`, `inbox.acknowledge`, `inbox.reply`,
`communication.complete`, `escalation.raise`, `board.list`, `task.read`. Listing показывает
ограниченную историю своего thread до исходного сообщения и ответы на него.
ACK/reply разрешены только для source message текущей assignment. Поля actor,
Run или чужой authority не принимаются из arguments.

`artifact.submit`, `outcome.submit` и Task-ориентированный `human.request` не
выдаются Communication. `escalation.raise` создаёт канонический вопрос с
Communication source, сохраняет исходное сообщение и останавливает этот Run.
Вопрос поступает в [Resolver queue](M2_RESOLVER_QUEUE.md); необязательная
контекстная Task от этого не получает ни wait, ни новую стадию. После ответа
возобновление held Communication требует отдельного разрешённого retry.

Reading не считается ACK. Требование `answered` требует ответа, а не только
ACK; для остальных source messages завершению предшествует явный ACK или reply.
Затем Employee вызывает `communication.complete` с пустыми arguments. Core
атомарно сохраняет completion, receipt и audit. Повтор того же `message_id` с
тем же содержимым возвращает первоначальный receipt; новый reply после
completion запрещён.

Completion не означает остановку процесса: Lease и physical reservation остаются
до подтверждённой quiescence. Exit zero сам по себе не отвечает на сообщение и
не завершает assignment. Project stop останавливает все purposes, включая
разговоры, и закрывает admission; он не уничтожает Inbox.

## Ошибки и повтор

Provider failure, timeout, потеря host или остановка переводят незавершённый
разговор в `held`; его source, принятые ответы и технические свидетельства
сохраняются. Это не меняет lifecycle `context_task_id`. Watchdog, graceful/force
stop, boot reconciliation и secret cleanup используют общий Run scope.

Именованная операторская команда `retry_communication` принимает:

```json
{
  "run_id": "<UUIDv7 последней остановленной попытки>",
  "reason": "Провайдер снова доступен; повторить сохранённый вопрос"
}
```

Нужны последняя `held` попытка, отозванная/освобождённая Lease и положительное
подтверждение physical quiescence. Создаётся новая попытка с новым Run/fence,
но той же assignment и исходным сообщением. Команда не запускает работу через
закрытый Project gate. Специальное разрешение retry относится только к этой
assignment при project recovery hold. В `manual_hold` принятие оценки
`not_started_confirmed` само по себе не заменяет явную команду retry.

Существующие ответы не удаляются. Повтор inference не является exactly-once
гарантией; canonical calls дедуплицируются по immutable call identity.

Technical evidence taskless Run хранится в пространстве Project/Run без
фиктивного Task ID; старые Task evidence keys не меняются. Summarizer не
становится владельцем исполнения или источником истины о завершении.

## Проверки

```sh
source scripts/lib/dev.sh
require_dev_environment
FORGE_INTEGRATION=1 cargo test --locked -p forge-testkit --test m2_communication_runs -- --ignored --test-threads=1
```

Тест использует настоящие PostgreSQL, Core и UDS Gateway, но ручной Supervisor
и синтетический credential. Он проверяет одновременные Task/Communication,
общую capacity, запрет Task mutation, ACK/answer/completion, сохранение physical
reservation, Project stop и retry после failure/quiescence. Платная inference
и native duplex этим тестом не подтверждаются. Отдельные регрессии проверяют
позднее событие старой попытки после retry, человеческий follow-up в том же
thread и отсутствие повторного запуска от собственного ответа Employee.
