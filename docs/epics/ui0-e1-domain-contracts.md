# Epic UI0.1 — Domain alignment и frontend contracts

**Milestone:** UI0 — Модель интерфейса и локальная API-граница
**Статус:** in_progress; read-contract срезы, epic не закрыт
**Тип / приоритет:** foundation / P0
**Источники:** [UI-план](../UI_IMPLEMENTATION_PLAN.md),
[матрица соответствия](../UI_BACKEND_ALIGNMENT.md)

**Task:** [FRONTEND-002](../../tasks/frontend/frontend-002-task-pipeline-contracts.md)
покрывает существующие Project/Task/PipelineVersion reads и synthetic tests.
[FRONTEND-003](../../tasks/frontend/frontend-003-run-read-contracts.md) добавляет
Run assignments, состояния и внешний diagnostics envelope.
[FRONTEND-005](../../tasks/frontend/frontend-005-task-run-presentation.md) добавляет
pure Task/Run presentation поверх этих DTO, сохраняя независимые состояния и
исходные данные. Экраны пока не подключаются. Employee/Surface,
детализация diagnostics, недостающие каталоги и миграция demo-потребителей
остаются следующими шагами; эти Task не закрывают полный exit gate ниже.
FRONTEND-005 проверена 18 presentation tests, прежними 47 contract tests,
семью browser tests, typecheck/lint/build и независимыми spec/quality reviews.

## Цель

Сделать модель Control Room точной проекцией Forge, сохранив визуальный каркас
архива. Зафиксировать какие поля приходят из Core, какие вычисляются только для
отображения и какие отсутствуют до отдельного продуктового расширения.

## Исходная точка

`frontend/src/data/types.ts` смешивает lifecycle и stage; Task содержит постоянных
assignee/reviewer, обязательные Git fields и fixed priority. Services работают
через mockDb; прямого переноса этих типов на canonical API быть не должно.

## В границах

- Frontend contracts для Project/Task/Employee/PipelineVersion/Run, optional
  surface и разных Run purposes; lifecycle, stage и activity различаются.
- Поля priority/properties/cancellation берутся из схем Project. Отсутствующие
  значения и unknown metrics не подменяются нулём или синтетическими значениями.
- Маппинг экрана/поля/action на существующие API или именованный read gap;
  отсутствие generic mutation не разрешает локально реализовать переход.
- Разделение live и explicit demo fixtures; неготовые live actions недоступны
  с причиной. Fixtures сохраняют дизайн и покрывают настоящие domain cases.
- Уточнение устаревших overview-формулировок PRD про pin и Integration по
  действующим domain/M2 контрактам; новые инварианты требуют отдельного решения.

## Не в границах

Новый scheduler, Goal/Epic, ResourcePool, SkillPack/CodeIndex, второй источник
canonical state, смена дизайна или объявление mock UI интегрированным.

## Контракты и зависимости

**Зависимости:** нет новых эпиков; baseline M0–M3 и доменные документы доступны.
Выход — типизированная карта DTO/projection и error/unknown состояний, пригодная
для API client UI0.2 и feature epics. API DTO и UI presentation types могут
различаться, но адаптер не создаёт domain policy.

## Направления будущей нарезки

1. Сверка Task/Project/Pipeline DTO и схем свойств с интерфейсом.
2. Employee/Run/Surface variants и provenance результатов.
3. Fixtures и presentation adapters с exhaustive handling состояний.
4. Doc alignment и реестр отсутствующих/неподдерживаемых UI actions.

## Exit gate и проверка

- Type/unit tests различают `waiting` Task в произвольной stage и running/
  stopping Run, а также ownerless SystemJob и taskless Communication.
- Fixtures включают analysis без Git, два Pipeline с разными stages, одну и
  десять priority levels, несколько Runs одного Employee и cancelled с reason.
- Маппинг не теряет pinned version, неизвестные measurements и multiple waits.
- В live режиме ни одна не реализованная операция не подтверждает fake success.
- Схемы/типы и выбранные domain sections проходят независимое review.

## Параллельность и риски

Может идти параллельно UI0.3. UI0.2 требует согласованной карты contracts.
Основной риск — принять удобное поле mock-модели за требование к backend.
Граница эпика — согласованное значение данных; каждый feature epic отвечает
за своё полноценное UI/API подключение и поведенческие тесты.
