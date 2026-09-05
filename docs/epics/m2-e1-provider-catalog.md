# Epic M2.1 — Provider catalog по единому adapter contract

**Milestone:** M2 — Engineering Pipeline и provider matrix
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-20–23

## Цель

Расширить native CLI catalog без размножения логики Core и без ложного обещания
одинаковых возможностей у всех providers.

## В границах

- Claude Code, Cursor, Gemini и Grok adapter crates;
- pinned fixtures, preflight/config/credential delivery и capability matrices;
- общий fake-CLI conformance suite и opt-in real CLI smoke.

## Не в границах

Изменения scheduler/Pipeline, host-profile mount, provider/model fallback и
выдуманные structured events или session resume.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-20 | Claude Code adapter |
| TASK-21 | Cursor adapter |
| TASK-22 | Gemini adapter |
| TASK-23 | Grok adapter |

## Exit gate

Каждый adapter проходит общую conformance suite на pinned fixture. Real smoke
явно opt-in и фиксирует фактический transport/capability matrix. Неподдержанная
возможность даёт policy_denied или preflight failure, не молчаливую деградацию.

## Риски

Vendor CLI изменяются со временем; pinned version и проверенные capabilities
должны оставаться частью profile/test evidence.
