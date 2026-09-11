# Epic M3.1 — Каноническая память и база знаний

**Уточнение 11 сентября:** full M3 утверждён в ../2026-09-11-m3-specs.md;
execution ledger — ../2026-09-11-m3-tasks.md. Этот Epic реализует canonical
TaskSummary/EmployeeMemoryEntry/ProjectKnowledgeEntry и versioned Markdown
KnowledgePage с author/publish/supersede/withdraw commands. AgentMemory,
SystemJob, Summarizer, context, onboarding и acceptance выделены в M3.2–M3.7.
Приёмка и её границы зафиксированы в ../M3_ACCEPTANCE.md.

**Milestone:** M3 — Knowledge loop
**Статус:** implemented and accepted; evidence in ../2026-09-11-m3-tasks.md
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-27

## Цель

Хранить source-linked производную память и человекочитаемые versioned Markdown
страницы. Не смешивать наблюдение, summary и опубликованное правило/решение.

## В границах

- TaskSummary, EmployeeMemoryEntry и ProjectKnowledgeEntry с source links;
- canonical revisions, hashes, visibility и append-only history;
- KnowledgePage author/publish/supersede/withdraw через именованные команды;
- durable projection intent в той же PostgreSQL transaction.

## Не в границах

LLM runtime, retrieval adapter, context compiler и onboarding: отдельные эпики
M3.2–M3.6. Собственный RAG, automatic knowledge acceptance и semantic audit
отчётов не входят в M3 вообще.

## Исполняемая часть

| Task | Результат |
|---|---|
| M3-01 | canonical memory/knowledge, revisions, commands, sources and projection intents |

## Exit gate

Команды сохраняют revision/history/Event/outbox атомарно; повтор не дублирует
запись. Employee/Summarizer не публикуют authority. Исходники и scope проверяются
Core; withdrawal немедленно исключает запись из разрешённых чтений независимо
от eventual index cleanup.

## Риски

Производная запись никогда не становится authority только из-за наличия цитат.
Область видимости хранится в Forge и не делегируется search engine.
