# Epic EXT1.1 — Optional Goal и Epic

**Milestone:** EXT1 — Planning
**Статус:** Deferred extension; design gate
**Источник:** [UI implementation plan](../UI_IMPLEMENTATION_PLAN.md), [backend alignment](../UI_BACKEND_ALIGNMENT.md)

## Цель

Описать optional planning-объекты `Goal` и `Epic`, через которые проект может
связать дальний замысел с ограниченным набором Task. `Task` остаётся единственной
исполняемой единицей и единственной карточкой runtime-очереди.

## Базовая граница

- PRD допускает цепочку `Goal → Epic → Task`, но не делает её обязательной.
- Текущий Task contract допускает optional planning references; M0–M3 не
  объявляют canonical API, storage или UI для самостоятельных Goal/Epic.
- Pipeline не создаёт эти объекты и не превращает их в stage или lifecycle.
- Проект без Goal/Epic должен сохранять полный существующий task-only flow.

## Предлагаемое расширение

- Ввести проектные planning records с identity, human-readable intent, audit и
  явным жизненным циклом, если дизайн подтвердит необходимость отдельных records.
- Разрешить Task ссылаться на Goal и/или Epic, но не требовать `parent_task_id`,
  Goal или Epic при создании Task.
- Показать иерархию как planning projection, не как ownership Run, lease или
  PipelineVersion.
- Хранить изменения и архивирование так, чтобы historical Task links оставались
  объяснимыми после смены planning context.

## Не в границах

- Автоматическое создание Task, запуск Run или изменение lifecycle по progress.
- Обязательная иерархия, глобальный portfolio management и cross-project goals.
- Новые Core `TaskKind`, Pipeline stages или подмена Task объектом Epic.
- Реализация storage, API, UI или миграции в рамках этого design gate.

## Зависимости

- Контракты UI1.3 задают, какие read/write projections реально понадобятся.
- M0–M3 остаются самостоятельной backend-базой; EXT1.1 не блокирует UI0–UI4
  и не является условием M4 installer/acceptance.
- До task breakdown нужна отдельная approval на выбранный domain/API design.

## Исследовательские решения до task breakdown

- Нужны ли отдельные aggregate `Goal` и `Epic`, либо достаточно одного
  versioned planning record с relation type.
- Какая project policy включает capability и как version/archival/audit работают
  без переписывания historical Task.
- Допускаются ли Task без Epic внутри Goal, несколько planning links и как
  запрещать циклы без введения mandatory parent Task.
- Как рассчитывать и показывать progress: manual, advisory или derived, не
  превращая расчёт в lifecycle command.
- Какие API/filter/pagination/read-model contracts нужны UI1.3 и CLI.

## Будущая декомпозиция

После утверждения дизайна работа может разделиться на domain contract,
canonical persistence/audit, named Manager commands, read projections и UI.
Это будущий breakdown, а не разрешение на реализацию.

## Exit gate

Утверждённый design record фиксирует optional semantics, lifecycle/archival,
Task-link invariants, audit и API boundary. Он доказывает совместимость с
task-only проектом и получает отдельное явное разрешение до implementation.

## Проверка

- Сверить design record с PRD: только Task executable; Goal/Epic optional.
- Проверить contract matrix для Task без links, с Goal, с Epic и с архивным link.
- Проверить, что UI1.3 не требует нового Core state до утверждённого API.

## Риски

Главный риск — незаметно сделать planning hierarchy обязательной или позволить
progress менять исполнимую Task. Второй риск — потерять historical context при
архивировании Goal/Epic; его закрывает явный audit и stable link semantics.
