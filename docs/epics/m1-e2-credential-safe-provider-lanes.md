# Epic M1.2 — Credential-safe first provider lanes

**Milestone:** M1 — Первый реальный Employee
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-17–19

## Цель

Выдать первому реальному Employee provider/runtime profile без попадания ключей
в canonical state или observability и доказать общий adapter contract на Codex.

## В границах

- Forge Secret Store, master-key resolution, CredentialBinding,
  CapabilityProfile и immutable ExecutionProfile;
- LiteLLM proxy-only API lane, Run-scoped virtual keys и opencode_runtime;
- adapter conformance harness, pinned Codex CLI preflight/start/stop/collect.

## Не в границах

Vault/KMS, provider auto-fallback, shared host auth home, остальные CLI adapters
и изменение Task state со стороны LiteLLM или adapter.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-17 | Secret Store, profile и credential policy |
| TASK-18 | LiteLLM и OpenCode API lane |
| TASK-19 | conformance harness и Codex CLI lane |

## Exit gate

Codex запускается в sandbox по pinned profile; selected engine и capabilities
видны в Run evidence. API runtime использует только Run-scoped virtual key.
Expired/over-budget key и unavailable capability дают typed Incident/preflight
failure, а не неявный provider, model или auth fallback.

## Риски

isolated_runtime_secret — явный аудируемый риск. Его нельзя превращать в
незаметное копирование host credentials ради удобства CLI.
