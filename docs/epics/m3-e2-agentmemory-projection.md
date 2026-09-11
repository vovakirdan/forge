# Epic M3.2 — AgentMemory projection

**Milestone:** M3 — Knowledge loop
**Статус:** implemented and accepted; evidence in ../2026-09-11-m3-tasks.md
**Contract:** ../2026-09-11-m3-specs.md
**Ledger:** ../2026-09-11-m3-tasks.md

## Scope

Dedicated instance/local embeddings; fixed BM25 upsert, strict import readiness, immutable projection IDs, retry/rebuild and canonical ACL revalidation.

## Exit gate

Repeated imports/update/delete/restart and partial index failures tested; real offline embedding/search smoke.

UI, installer, custom RAG and deferred M2 provider gates excluded.
