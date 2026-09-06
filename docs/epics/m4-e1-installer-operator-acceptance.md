# Epic M4.1 — Installer, operator workflow и MVP acceptance

**Milestone:** M4 — Local product proof и interface readiness
**Источник:** ../IMPLEMENTATION_PLAN.md, TASK-28–29

## Цель

Сделать local Forge устанавливаемым одной командой/wizard и доказать полный
operator flow, включая отказы, без UI или remote control.

## В границах

- prerequisite checks, rootless data services, initial wizard, state dirs,
  ownership/permissions, Core/Supervisor binaries и user systemd units;
- foreground development mode, reboot/BootRecoveryPolicy smoke;
- happy path/failure matrix, fixture repositories/provider stubs, CLI runbook и
  release evidence checklist.

## Не в границах

Kubernetes/cloud deploy, external S3, desktop packaging, distributed runners,
web UI acceptance или remote operator access.

## Состав будущих Task

| Task | Результат |
|---|---|
| TASK-28 | Linux installer, wizard и user systemd services |
| TASK-29 | full MVP acceptance, failure matrix и operator runbook |

## Exit gate

Чистый Linux host устанавливает Forge документированной командой, reaches
readyz, корректно restart/reconcile-ит после reboot и завершает acceptance
scenario через CLI. Provider failure, budget exhaustion, force stop, dependency,
review/merge, summarizer lag и reboot дают известные Incident/Task outcomes.

## Риски

Installer создаёт processes и config, но не становится вторым Core или source of
domain truth. Unsupported provider capability показывается оператору явно.

## Открытый дизайн: среда разработки

Wizard должен позволять настроить bundle среды разработки проекта, включая
непредусмотренные заранее toolchains и зависимости. До реализации нужно
согласовать формат описания, подготовку окружения, доступ к пакетным репозиториям
и кеширование. Текущий M1 умеет выбирать digest-pinned runtime image через
`RuntimeBinding.image`, но удобного bundle wizard и автоматической подготовки
зависимостей ещё нет. Это развитие installer/runtime workflow, не новый долг M1.
