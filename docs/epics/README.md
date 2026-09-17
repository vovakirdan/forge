# Forge: Epic Index

Эти документы раскладывают [backend-план](../IMPLEMENTATION_PLAN.md) и
[Control Room план](../UI_IMPLEMENTATION_PLAN.md) по planning-иерархии:
Milestone → Epic → Task. Epic не является runtime-сущностью Forge и не создаёт
Task на доске.

## Backend milestones

| Milestone | Epic | Будущие Task | Exit gate |
|---|---|---|---|
| M0 | M0.1 Workspace и test topology | 01–02 | repeatable local environment |
| M0 | M0.2 Domain и command model | 03–05 | pure rules и mutation boundary |
| M0 | M0.3 Durable scheduler и simulator | 06–10 | fake Pipeline проходит end-to-end |
| M1 | M1.1 Supervisor, sandbox и failure boundary | 11–16 | isolated observed execution |
| M1 | M1.2 Credential-safe first provider lanes | 17–19 | первый real provider без утечки secrets |
| M2 | M2.1 Provider catalog | 20–23 | native CLI contract-tested |
| M2 | M2.2 Git delivery, verification и review | 24–25 | delivery Task проходит engineering loop |
| M2 | M2.3 Project management и resolver queues | 26 | named management без власти Employee |
| M3 | [M3.1 Canonical knowledge](m3-e1-derived-memory-retrieval.md), [M3.2 AgentMemory](m3-e2-agentmemory-projection.md), [M3.3 SystemJobs](m3-e3-system-jobs.md), [M3.4 Summarizer](m3-e4-summarizer.md), [M3.5 Context](m3-e5-context-retrieval.md), [M3.6 Onboarding](m3-e6-onboarding.md), [M3.7 Acceptance](m3-e7-acceptance.md) | [отдельный ledger](../2026-09-11-m3-tasks.md) | bounded canonical knowledge loop |
| M4 | M4.1 Installer и operator acceptance | 28–29 | one-command local MVP proof |

M0–M3 сохраняют свои ledgers и приёмку. По решению 11 сентября UI0–UI4 идут
до M4; это не перенос завершённой backend-работы в новый roadmap.

## Control Room milestones

UI0.1 и UI0.3 имеют статус in_progress:
[FRONTEND-001](../../tasks/frontend/frontend-001-local-demo-baseline.md) проверяет
local demo baseline, [FRONTEND-002](../../tasks/frontend/frontend-002-task-pipeline-contracts.md)
добавляет Task/Pipeline read contracts, а
[FRONTEND-003](../../tasks/frontend/frontend-003-run-read-contracts.md) — Run и
diagnostics contracts без подключения экранов.
[FRONTEND-004](../../tasks/frontend/frontend-004-browser-smoke.md) добавляет
browser smoke существующего demo в UI0.3. Остальные UI epics
— planned; полные gates UI0.1/UI0.3 не пройдены. Задачи — в [Task index](../../tasks/INDEX.md).

| Milestone | Epic | Выход |
|---|---|---|
| UI0 | [UI0.1 Domain alignment](ui0-e1-domain-contracts.md) | Core-compatible frontend model |
| UI0 | [UI0.2 Browser/API](ui0-e2-browser-api.md) | protected local client boundary |
| UI0 | [UI0.3 Toolchain/tests](ui0-e3-frontend-tooling.md) | reproducible development harness |
| UI1 | [UI1.1 Projects/Team/profiles](ui1-e1-projects-team-profiles.md) | real scoped management catalog |
| UI1 | [UI1.2 Pipelines/hooks](ui1-e2-pipeline-versions-hooks.md) | immutable version-aware editor |
| UI1 | [UI1.3 Task/Board](ui1-e3-task-board-management.md) | configurable Task projection and commands |
| UI1 | [UI1.4 Surfaces/artifacts](ui1-e4-surfaces-artifacts.md) | safe source/evidence navigation |
| UI2 | [UI2.1 Runs/Activity](ui2-e1-run-activity-evidence.md) | observed execution and history |
| UI2 | [UI2.2 Inbox/Communication](ui2-e2-inbox-communication.md) | durable addressed conversations |
| UI2 | [UI2.3 Management/resolution/recovery](ui2-e3-management-resolution-recovery.md) | controlled human intervention |
| UI3 | [UI3.1 Knowledge/memory](ui3-e1-knowledge-memory.md) | canonical authority and derived memory |
| UI3 | [UI3.2 SystemJobs/onboarding](ui3-e2-system-jobs-onboarding.md) | bounded background work controls |
| UI3 | [UI3.3 Resources/settings](ui3-e3-resources-settings.md) | actual limits and readiness |
| UI4 | [UI4.1 Acceptance](ui4-e1-acceptance.md) | keyless and opt-in live evidence |
| UI4 | [UI4.2 Installer handoff](ui4-e2-installer-handoff.md) | stable package/config/runbook contract |

## Design-gated extensions

Это новые продуктовые домены, не недостающие GET для готовой функции.
Они не блокируют UI4/M4 и требуют отдельного scope/design approval.

| Milestone | Epic | Выход после согласования |
|---|---|---|
| EXT1 | [EXT1.1 Goals/Epics](ext1-e1-goals-epics.md) | optional planning model |
| EXT1 | [EXT1.2 Bounded planning](ext1-e2-bounded-planning.md) | audited proposals and waves |
| EXT2 | [EXT2.1 Resource pools](ext2-e1-resource-pools.md) | named pool admission/drain contract |
| EXT3 | [EXT3.1 Skill packs](ext3-e1-skill-packs.md) | versioned reusable context catalog |
| EXT3 | [EXT3.2 Code intelligence](ext3-e2-code-intelligence.md) | scoped external index integration |

Dependency graph, milestone gates и очередь ближайших эпиков — в
[UI_IMPLEMENTATION_PLAN.md](../UI_IMPLEMENTATION_PLAN.md).
Следующие Task-файлы создаются при отдельном разборе выбранного эпика.
