# Epic UI0.3 — Frontend toolchain и test harness

**Milestone:** UI0 — Модель интерфейса и локальная API-граница
**Статус:** in_progress; local demo baseline и browser smoke — отдельные срезы, epic не закрыт
**Тип / приоритет:** foundation / P0
**Источники:** [UI-план](../UI_IMPLEMENTATION_PLAN.md),
[матрица соответствия](../UI_BACKEND_ALIGNMENT.md)

**Tasks:** [FRONTEND-001](../../tasks/frontend/frontend-001-local-demo-baseline.md)
и [FRONTEND-004](../../tasks/frontend/frontend-004-browser-smoke.md).
Команды и границы локального запуска — в [frontend README](../../frontend/README.md).
FRONTEND-004 вводит browser smoke навигации, диалогов и клавиатуры для mock-only
demo. Component harness, CI, visual baselines и error/offline/permission states
остаются отдельными шагами; browser smoke не закрывает live API gate.
В FRONTEND-004 проверены семь сценариев двумя последовательными root runs,
error collector, отказ на занятом порте и cleanup после global timeout.

[FRONTEND-006](../../tasks/frontend/frontend-006-static-hosting-boundary.md) в UI0.2
переиспользует эти сценарии для отдельного static build и file server на 4174.
Он не заменяет dev-suite на 4173 и не закрывает component/CI/live API gates.

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

FRONTEND-004 выявила hydration errors при слишком раннем клике по SSR-разметке.
Smoke ожидает клиентские query data, не скрывая console/page errors. Это не
исправление самого раннего пользовательского взаимодействия: такую проверку
нужно вернуть при работе с loading/keyboard states, не объявляя её покрытой
обычным navigation smoke.
