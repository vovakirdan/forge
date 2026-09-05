# Epic M3.1 — Derived memory и retrieval projection

**Milestone:** M3 — Knowledge loop
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-27

## Цель

Превратить Task evidence в полезные summaries и retrieval entries, не смешивая
факт события, summary и authoritative project knowledge.

## В границах

- outbox-triggered SummarizationJob, coalescing, capacity/budget и stop-gate;
- TaskSummary, EmployeeMemoryEntry и ProjectKnowledgeEntry с source links;
- AgentMemory indexing/retrieval adapter и Core revalidation перед injection в
  ContextSnapshot.

## Не в границах

Собственный RAG/graph engine, automatic knowledge acceptance, semantic audit
report, blocking summary или domain-write власть Summarizer.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-27 | summarization dispatcher, derived entries и AgentMemory projection |

## Exit gate

Canonical TaskHandoff доступен следующему Run сразу. Delayed/failed summary не
останавливает Pipeline; Summarizer не меняет Task/Artifact state. Core отклоняет
stale, invisible или out-of-scope retrieved entry до context injection.

## Риски

Модель Summarizer остаётся config/profile choice. Его budget измеряется отдельно,
чтобы knowledge loop не поглотил capacity engineering Runs.
