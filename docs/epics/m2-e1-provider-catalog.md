# Epic M2.1 — Provider catalog по единому adapter contract

**Milestone:** M2 — Engineering Pipeline и provider matrix
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-20–23

**Уточнение M2:** актуальны [specification](../2026-09-06-m2-specs.md) и
[execution ledger](../2026-09-06-m2-tasks.md). Текущая матрица: Codex CLI,
Claude CLI, OpenRouter API и OpenAI API. Cursor, Gemini и **Grok Build CLI**
отложены; отсутствие настройки не подменяется другим провайдером.

## Цель

Расширить native CLI catalog без размножения логики Core и без ложного обещания
одинаковых возможностей у всех providers.

## В границах

- Claude Code adapter и native session drivers Codex/Claude/OpenCode;
- четыре явных immutable runtime profiles с разными credential contracts;
- pinned fixtures, preflight/config/credential delivery и capability matrices;
- общий fake-CLI conformance suite и opt-in real CLI smoke.

## Не в границах

Изменения scheduler/Pipeline, host-profile mount, provider/model fallback и
выдуманные structured events или session resume.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-20 | Claude Code adapter |
| TASK-21 | Cursor adapter — отложено |
| TASK-22 | Gemini adapter — отложено |
| TASK-23 | Grok Build CLI adapter — отложено |

## Exit gate

Каждый adapter проходит общую conformance suite на pinned fixture. Real smoke
явно opt-in и фиксирует фактический transport/capability matrix. Неподдержанная
возможность даёт policy_denied или preflight failure, не молчаливую деградацию.

## Риски

Vendor CLI изменяются со временем; pinned version и проверенные capabilities
должны оставаться частью profile/test evidence.
