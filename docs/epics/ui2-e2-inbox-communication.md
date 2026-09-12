# Epic UI2.2 — Inbox и Communication

**Milestone:** UI2 — Наблюдение, коммуникация и вмешательство человека
**Статус:** planned; реализация не начата
**Источник:** [UI plan](../UI_IMPLEMENTATION_PLAN.md),
[backend alignment](../UI_BACKEND_ALIGNMENT.md)
**База backend:** `1439211`; операторский доступ наследует границу UI0.
**Зависимости:** UI0.1–UI0.3, UI1.1, UI1.3.

## Цель

Дать оператору адресный разговор с Employee и показать судьбу сообщения:
сохранение, попытку доставки, подтверждение и ответ. Разговор может существовать
без Task и без постоянно запущенного процесса Employee.

## В границах

- Список Employee threads и переписка с pagination и безопасным rendering.
- Открытие thread, отправка instruction/question/notification и чтение reply.
- Явный выбор Inbox, TaskExecution или ExactRun с каноническим контекстом.
- Informational, acknowledged и answered requirements и их реальные receipts.
- Видимые pending/held/failed состояния доставки, retry и management waiver.
- Связи между thread, message, Communication Run и contextual Task, если она есть.

## Что уже есть и каких чтений нет

Backend принимает `open_employee_thread`, `send_employee_message`,
`waive_message_requirement` и `retry_communication`. Есть
`GET /v1/projects/{project_id}/employees/{employee_id}/threads` и
`.../threads/{thread_id}/messages`. Domain различает target, message kind и
delivery requirement; transport acceptance не заменяет Employee acknowledgement.

Для экрана не хватает согласованной проекции delivery/ack/answer/waiver state.
Project-wide inbox допустим как bounded read существующих threads/messages,
если он нужен навигации; новый Conversation aggregate для этого не требуется.
Опорный код: `crates/forge-core/src/http/communication.rs`,
`crates/forge-domain/src/communication/`, `crates/forge-storage/src/communication.rs`.

## Контракты и зависимости

- UI1.1 предоставляет настоящие Employee IDs и состояние допуска, UI1.3 — Task
  identity, revision, stage и visit. Получатель не выбирает произвольный процесс.
- ExactRun использует точные Run/fence/epoch; stale target отклоняется Core.
- Command receipt означает durable acceptance команды, а не прочтение текста.
  UI отдельно показывает persisted, delivered, acknowledged и answered evidence.
- Повтор после transport timeout сохраняет idempotency key и исходный payload;
  новая попытка доставки остаётся отдельной существующей командой Core.
- Waiver требует явного подтверждения и причины; он не рисуется как ответ Employee.
- UI0 фиксирует response DTO/OpenAPI, bounds и origin/auth checks; сообщения
  считаются недоверенным содержимым, а не разрешёнными HTML или командами UI.

## Не в границах

Автономный Lead/Manager LLM, planning waves, создание Task из каждого сообщения,
универсальный chat backend, подключение к произвольной provider session или
изменение lifecycle Task посредством свободного текста.
Chat UI не обещает live input там, где pinned provider capability его не допускает.

## Направления будущей декомпозиции

1. Спроектировать bounded message delivery projection и её read-контракт.
2. Подключить threads/messages и формы существующих named commands.
3. Отобразить target, requirement, receipts, held delivery и explicit waiver.
4. Проверить reconnect/replay, taskless Communication и отказ stale ExactRun.

Это направления работ; отдельные TASK-ID и файлы задач создаются позднее.

## Exit gate и проверки

- Keyless сценарий сохраняет вопрос без Task и показывает фактический reply.
- Сохранение сообщения, transport delivery и acknowledgement проверяются
  отдельными assertions; answered requirement не закрывается одной доставкой.
- Reload/reconnect не повторяет отправку; pagination сохраняет порядок и scope.
- Неподдерживаемый live input, busy Employee и held conversation дают явное
  состояние, а не имитацию активного ответа.
- Waiver и retry проверены на revision/idempotency errors и смену состояния.
- Browser-тесты покрывают клавиатуру, длинные сообщения, ошибки и опасную разметку;
  для UI acceptance не требуется автоматически запускать платный provider.

## Риски

Метка «прочитано» без receipt создаёт ложную уверенность оператора. UI не должен
терять question/requirement при обновлении thread или выдавать новый Run за
продолжение старой provider session.
