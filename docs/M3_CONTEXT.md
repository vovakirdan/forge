# M3: контекст Employee Run

Для новых Task, Communication и Resolution Run Core собирает `knowledge_context` под блокировкой Project. Точное JSON-значение одновременно помещается в канонический `context_manifest` и `run_spec.instruction`; подготовка провайдера использует эту закреплённую инструкцию. Старые снимки без поля продолжают читаться. Пустой Project сохраняет прежний Task/Handoff-контракт.

В новых Task snapshots `system_policy_revision` и `employee_prompt_revision` содержат независимые `sha256:`-идентификаторы фактических prompt. Поэтому изменение текста отражается даже при неизменном runtime profile; старые ссылки не переписываются.

В `required_pages` входят все текущие опубликованные Policy и Decision. Они читаются напрямую из PostgreSQL, без AgentMemory и top-k. Лимит — 128 страниц и 128 KiB сериализованного содержимого. Превышение останавливает preflight с `context.required_knowledge_overflow`, до Lease/Run и подготовки провайдера; правила не отбрасываются молча.

Дополнительный блок ограничен 32 KiB: до 16 опубликованных introduction/architecture/guide и до 32 последних видимых записей памяти. Core повторно проверяет канонические источники и исключает отозванные записи, недоступные источники и ссылки на устаревшие редакции страниц. Личные записи доступны только Employee данного Run. `omitted_optional_candidates` учитывает отброшенных кандидатов выбранной ограниченной выборки, но не является полным количеством записей вне выборки.

Производная память — свидетельства, а не authority; она не меняет системную политику, Project Policy/Decision или возможности инструментов. AgentMemory outage не блокирует исходный контекст: этот путь использует PostgreSQL. Нельзя продолжать при недоступном каноническом хранилище, поскольку без него невозможно проверить обязательные правила и действующий Run fence.

## Явное обновление

`memory.refresh` не принимает Employee или Project от вызывающего. Core определяет их по действующему `RunScope`, проверяет закреплённый grant, собирает новую версию и атомарно записывает `run_knowledge_context_refreshes`, Gateway receipt и Event/outbox. Ответ содержит новый `context_snapshot_id`, полный `context`, SHA-256, исходный идентификатор и `delivery: "tool_response"`. Это доказательство передачи через инструмент, а не утверждение, что новый контекст был в первоначальном prompt.

Исходный Run manifest остаётся неизменяемым. Повтор того же message ID/payload возвращает прежний снимок. Новый message ID создаёт новый снимок. После отзыва Run fence или остановки Project новые запросы запрещены. Hook и SystemJob не получают этот инструмент. Event содержит только идентификаторы/hash и личную область, без текста личной памяти.

## Проверка без inference

`m3_context` использует изолированную PostgreSQL schema, локальный NATS, синтетический credential и неисполняющий Supervisor. Проверяются реальные подготовленные provider stdin для трёх назначений Employee, равенство bundle/manifest, недоступность AgentMemory, личная область, устаревшие/отсутствующие источники, обязательное переполнение и Gateway refresh/replay. Эти проверки не заявляют запуск настоящей модели или sandbox.

```sh
source scripts/lib/dev.sh
require_dev_environment
FORGE_INTEGRATION=1 cargo test --locked -p forge-testkit --test m3_context -- --ignored --test-threads=1
```
