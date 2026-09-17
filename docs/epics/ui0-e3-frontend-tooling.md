# Epic UI0.3 — Frontend toolchain и test harness

**Milestone:** UI0 — Модель интерфейса и локальная API-граница
**Статус:** in_progress; первая Task — локальный demo baseline, epic не закрыт
**Тип / приоритет:** foundation / P0
**Источники:** [UI-план](../UI_IMPLEMENTATION_PLAN.md),
[матрица соответствия](../UI_BACKEND_ALIGNMENT.md)

**Task:** [FRONTEND-001](../../tasks/frontend/frontend-001-local-demo-baseline.md).
Команды и границы локального запуска — в [frontend README](../../frontend/README.md).
Test harness, CI и остальные gates эпика остаются отдельными шагами.

## Цель

Сделать frontend обычной проверяемой частью repository, с воспроизводимой
сборкой и изолированными тестами, независимо от Lovable editor и платных моделей.

## В границах

- Проверить React/TypeScript/TanStack/Vite dependency graph, `bun.lock` и
  Lovable/Nitro-specific config. Зафиксировать один package manager и runtime
  versions после frozen-install/build preflight; не вести конкурирующие lockfiles.
- Проверить baseline packaging и убрать обязательную зависимость локальной
  разработки от редактора/hosted preview; не менять визуальный язык попутно.
  Дальнейшая адаптация под выбранный hosting входит в UI0.2.
- Документированные install/dev/build/typecheck/lint/component-test/browser-test
  команды и root developer entrypoints; selective CI для затронутых частей.
- Explicit demo fixtures, isolated keyless Core fixtures, project-scoped query
  harness; отдельный признак simulated evidence, не fallback живого клиента.
- Базовые loading/error/empty/permission/offline states, keyboard navigation,
  focus restoration, narrow viewport и regression coverage общих компонентов.
- Ignore rules для node_modules/build/browser reports/state/secrets; lockfile
  и исходные assets остаются tracked. Frontend codegen контролируется diff-check.

## Не в границах

Полный redesign, массовое обновление зависимостей без причины, перенос backend
на другой стек, системная установка Forge, новые provider Runs и UI analytics.

## Контракты и зависимости

**Зависимости:** нет новых эпиков; UI0.1 может идти параллельно.
Baseline build/toolchain проверяется без ожидания hosting ADR UI0.2, чтобы
не создавать циклическую зависимость. Выход — проверенные команды и harness,
которым пользуются все feature epics; импортированный README не считается
доказательством работоспособной сборки.

## Направления будущей нарезки

1. Toolchain/lockfile compatibility preflight и documented local setup.
2. Static/type/component checks и explicit fixture isolation.
3. Browser harness, screenshot baselines и keyboard/error states.
4. Root/CI entrypoints и ignore/secret hygiene.

## Exit gate и проверка

- Clean checkout устанавливается frozen-командой, собирается и typechecks
  без ручного исправления lockfile. Точные команды сохранены в runbook.
- Неготовый backend приводит к явному unavailable, не к mock-success.
- Component/browser smoke проходит без provider credentials или inference.
- Generated output/reports/dependencies не попадают в Git.
- Дизайн до/после согласован по ключевым экранам; исправление compiler/lint
  ошибок не объявляется функциональной интеграцией.

## Риски

Архив содержит framework-specific wrapper и beta build dependency. При отказе
frozen install сначала фиксируется причина/минимальное совместимое изменение,
а не заменяется весь frontend framework. Технические решения принимаются до
декомпозиции зависящих feature tasks и отражаются в STACK/ARCHITECTURE.
