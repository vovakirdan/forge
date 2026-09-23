# Epic UI3.1 — Knowledge и Memory

**Milestone:** UI3 — Knowledge loop и операционная конфигурация
**Статус на 23 сентября 2026:** локальные authoritative Knowledge, derived memory и source-linked reads реализованы и проверены.
**Источник:** [UI plan](../UI_IMPLEMENTATION_PLAN.md),
[backend alignment](../UI_BACKEND_ALIGNMENT.md)
**База backend:** `1439211`; M3 каноническая память отделена от search projection.
**Зависимости:** UI0.1–UI0.3, UI1.1, UI1.3, UI1.4.

## Цель

Дать оператору читать и редактировать canonical knowledge, исследовать разрешённую
память и её источники. На экране должно быть понятно, что является опубликованным
правилом, что — наблюдением, а что — отстающим состоянием индекса.

## В границах

- Knowledge pages: introduction, architecture, guide, policy, decision;
  draft/published/withdrawn, revision history и editorial commands.
- TaskSummary, EmployeeMemoryEntry и ProjectKnowledgeEntry как разные записи.
- Project/personal scope, source refs, revision/hash, coverage и provenance.
- Scoped search, canonical detail/history и состояние projection/freshness.
- Ссылки на разрешённые Task/Artifact/Knowledge/Event sources и их версии.
- Явные indexed/canonical fallback, pending/retired и stale-source состояния.

## Что уже есть и каких чтений нет

Есть `GET /v1/projects/{project_id}/knowledge`, page detail/history и команды
`author_knowledge_page`, `publish_knowledge_page`, `supersede_knowledge_page`,
`withdraw_knowledge_page`. Memory API предоставляет list/search/detail/history/
status; индексный текст Core заменяет каноническим после проверок scope и revision.

Нужно описать фактические M3 endpoints и commands в общей OpenAPI/typed boundary:
текущий `openapi/v1.yaml` не покрывает их полностью. Дополнительные source reads
допустимы в пределах UI1.4/UI2.1, с отдельной авторизацией каждого объекта.
Опорный код: `crates/forge-core/src/http/knowledge.rs`,
`crates/forge-core/src/http/memory.rs`, `crates/forge-domain/src/knowledge/`.

## Контракты и зависимости

- Published Policy/Decision выше derived memory по authority. Наличие citation
  не делает observation принятой истиной; auto-publish от summarizer исключён.
- Personal scope выбирается только в разрешённом operator read-контракте;
  параметр Employee не превращает браузер в его Run Gateway.
- Withdrawal немедленно запрещает новое использование canonical записи даже
  при eventual index cleanup. История остаётся аудируемой в разрешённых границах.
- Editorial command несёт idempotency key и нужные revisions; конфликт обновления
  не перезаписывает чужую редакцию. UI показывает источник текущей версии.
- UI не придумывает confidence/importance, validity intervals или candidate
  approval statuses: текущая derived schema этих полей не предоставляет.
- Renderer обрабатывает Markdown и source text как недоверенные данные.

## Не в границах

Новый RAG engine, semantic truth validator, автоматическая consolidation,
управление личным AgentMemory daemon, самостоятельный candidate acceptance
workflow или объявление полного соответствия PRD memory schema.
Targeted context и отсутствие повторного introduction не «исправляются» UI.

## Направления будущей декомпозиции

1. Зафиксировать M3 OpenAPI/DTO, authority/scope и редакционные формы.
2. Подключить page catalog/detail/history и существующие editorial commands.
3. Подключить scoped memory search/detail/history и source navigation.
4. Проверить withdrawal, index lag/outage, приватность и безопасный rendering.

Это направления работ; отдельные TASK-ID и файлы задач создаются позднее.

## Exit gate и проверки

- Operator workflow создаёт draft, публикует, заменяет и отзывает страницу;
  history и revisions совпадают с Core после reload и idempotent replay.
- Personal/project записи различаются; чужой scope, stale source и withdrawn
  result не попадают в разрешённое чтение через сохранённый search hit.
- Index outage сохраняет canonical fallback; UI не показывает pending как ready.
- Источник и coverage доступны без обещания семантической истинности summary.
- Keyless contract/browser tests покрывают malformed Markdown, source errors,
  empty search, pagination и revision conflict без paid inference.

## Риски

Общий раздел «база знаний» может визуально смешать правила и чужие наблюдения.
Кешированный search hit не является разрешением на чтение; local UI cache должен
учитывать scope, revision и повторную каноническую проверку.
