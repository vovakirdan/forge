# Epic UI2.1 — Runs, Activity и evidence

**Milestone:** UI2 — Наблюдение, коммуникация и вмешательство человека
**Статус:** planned; реализация не начата
**Источник:** [UI plan](../UI_IMPLEMENTATION_PLAN.md),
[backend alignment](../UI_BACKEND_ALIGNMENT.md)
**База backend:** `1439211`; транспортные ограничения зафиксированы в UI0.
**Зависимости:** UI0.1–UI0.3, UI1.3, UI1.4.

## Цель

Показать, что было запрошено у Run, что наблюдал Supervisor и какие результаты
сохранил Core. Связать Activity, Task, Run, Artifact и техническое evidence без
подмены канонического результата текстом провайдера.

## В границах

- Список и detail Runs с фильтрами по Project, purpose и доступным владельцам.
- Раздельные desired/observed state, attempt, fence, epoch и время наблюдения.
- TaskStage, Communication, Resolution, Hook и SystemJob как разные assignments.
- Activity с canonical Event identity/sequence и переходами к доступным объектам.
- Runtime report, incidents, handoff, evidence receipts и известный usage.
- Ограниченный просмотр разрешённого технического evidence и redacted
  ContextSnapshotView после появления безопасных read-контрактов;
  состояния pending/incomplete/expired.

## Что уже есть и каких чтений нет

`GET /v1/projects/{project_id}/runs` и `.../runs/{run_id}` возвращают Run и
diagnostics. Последние содержат report, handoff, incidents, evidence receipts,
stream completeness, proxy usage и Git source, но исключают prompts/auth.
`.../events` возвращает конечную SSE-пачку: до 1000 событий, затем соединение
завершается. Есть `Last-Event-ID` и `after`, постоянной подписки пока нет.

Нужны scoped reads для разрешённого evidence body и redacted ContextSnapshotView,
если эти данные включены в экран. По умолчанию context view даёт allowlisted
IDs/revisions/hash/provenance, не raw manifest, полный prompt или RunSpec. Каждый
переход к содержимому повторно проверяет scope и доступность source, включая
personal memory и отозванные знания; frozen snapshot не обходит текущие права.
Receipt или object key сами по себе не дают браузеру доступ к MinIO, локальному
пути или секретам. Если безопасный body contract не определён, остаются только
метаданные/разрешённые ссылки с явным unavailable для содержимого.
Опорный код: `crates/forge-core/src/http/handlers.rs`,
`crates/forge-core/src/http/views.rs`, `crates/forge-storage/src/run_evidence.rs`.

## Контракты и зависимости

- UI0 определяет DTO, ошибки, browser access и покрытие OpenAPI для новых reads.
- SSE adapter сохраняет последний принятый sequence, подавляет дубликаты,
  возобновляет чтение с backoff и ограничивает буфер; смена Project отменяет запрос.
- При потере/отказе cursor UI явно обновляет snapshot, а не скрывает разрыв истории.
- `task_id` и `employee_id` могут отсутствовать. Ownerless SystemJob/Hook не
  получают выдуманного Employee; contextual Task не становится владельцем Run.
- Неизвестные tokens, cost и measurements показываются как неизвестные, не ноль.
- Все управляющие действия используют контракты UI2.3; универсальный Abort API
  не предполагается. Evidence viewer имеет bounds, scope и безопасный rendering.
- Bodies имеют явный allowlist типов/полей, ограничения размера и redaction;
  обработка auth material действует до выдачи ответа, не только в UI renderer.

## Не в границах

Прямой доступ браузера к PostgreSQL, NATS, MinIO или sandbox filesystem;
terminal/shell, full-text публикация всех prompts/logs, новый event broker,
интерпретация process exit как принятого результата или новый execution engine.

## Направления будущей декомпозиции

1. Уточнить Run/diagnostic DTO и недостающие object/context reads.
2. Подключить список, detail, ссылки на владельцев и evidence states.
3. Реализовать bounded Activity reader с reconnect, cursor и deduplication.
4. Добавить безопасный evidence/context viewer и интеграционные сценарии.

Это направления работ; отдельные TASK-ID и файлы задач создаются позднее.

## Exit gate и проверки

- Все пять purposes отображаются по реальным fixtures, включая taskless Runs.
- Разрыв SSE, повтор пачки, stale cursor, reload и переключение Project не
  создают дубли и не смешивают проекты; память клиента ограничена.
- Requested stop не рисуется как physical stopped до соответствующего observation.
- Body/context reads проверены на чужой scope (включая personal memory),
  withdrawn source, oversized content и недоступный объект; unsafe HTML,
  auth material и произвольные host paths не попадают в ответы или viewer.
- Keyless API/UDS и browser-тесты покрывают loading, empty, error и partial data;
  provider live не запускается автоматически для проверки интерфейса.

## Риски

Timestamp не заменяет sequence. Задержка evidence не означает отсутствие работы,
а текстовый отчёт не доказывает acceptance. Избыточные context/log reads могут
раскрыть приватный материал, который намеренно исключён из diagnostics.
