# Epic UI4.2 — Control Room handoff для backend M4

**Milestone:** UI4 — UI-приёмка и передача установщику
**Статус:** planned; blocked by UI4.1
**Тип / приоритет:** integration / P1
**Источники:** [UI-план](../UI_IMPLEMENTATION_PLAN.md),
[M4 installer epic](m4-e1-installer-operator-acceptance.md)

## Цель

Передать установщику стабильный контракт локального Control Room после UI
проверок. Installer должен упаковать проверенное приложение, а не заново
изобретать auth, frontend hosting или startup dependencies.

## В границах

- Build artifact/runtime requirements выбранного в UI0.2/0.3 hosting решения;
  воспроизводимая сборка без Lovable editor и без секретов в assets.
- Документированные адрес/port/socket, session bootstrap, owner permissions,
  health/ready, startup ordering, restart и teardown поведения UI host.
- Совместимость с CLI: UI и CLI используют тот же Core, UI disconnect/reload
  не означает stop/restart проекта и не возобновляет interrupted Runs.
- Operator runbook: обычный старт dev environment, вход в UI, диагностика
  недоступного Core/index, сбор evidence без credential disclosure.
- Обновлённый M4 scope для UI artifact/service/config и отдельной install smoke;
  browser acceptance остаётся в UI4.1, clean-host/systemd/reboot proof — в M4.

## Не в границах

Сам installer, изменение системных services на машине пользователя, remote
ingress, TLS/cloud accounts, desktop/TUI package, миграция provider credentials
или выполнение уже описанного в M4 clean-host proof.

## Контракты и зависимости

**Зависимости:** UI4.1. Hosting/security и toolchain decisions UI0 входят в
handoff как принятые документы, а не открытые варианты для installer implementer.
EXT milestones не являются зависимостями поставки локального UI.

## Направления будущей нарезки

1. Artifact/config/service interface для installer и проверки compatibility.
2. Local operator runbook и evidence troubleshooting.
3. M4 planning/docs update и проверка dependency handoff.

## Exit gate и проверка

- Собранный UI запускается по документированному foreground workflow на
  существующем dev host; закрытие browser не запускает/останавливает Run.
- Отдельно проверены session expiration/re-entry и совместный доступ CLI/UI.
- M4 получает точные requirements и smoke checklist без обязательства ставить
  frontend dev dependencies на конечный host, если выбран static artifact.
- Ссылки, команды и ownership/config boundaries проходят независимое review.
- M4 не помечается выполненным на основании этого handoff.

## Риски

Dev server нельзя незаметно объявить production host. Новая UI session boundary
не отменяет BootRecoveryPolicy и не предоставляет доступ удалённым пользователям.
