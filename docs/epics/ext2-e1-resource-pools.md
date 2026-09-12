# Epic EXT2.1 — Resource pools

**Milestone:** EXT2 — Resource pools
**Статус:** Deferred extension; design gate
**Источник:** [UI implementation plan](../UI_IMPLEMENTATION_PLAN.md), [backend alignment](../UI_BACKEND_ALIGNMENT.md)

## Цель

Исследовать first-class `ResourcePool` для явного распределения ограниченных
ресурсов без ослабления existing admission, concurrency и fencing invariants Core.

## Базовая граница

- M0–M3 уже ограничивают admission, Employee capacity, provider budgets и
  одновременные Run, но отдельного aggregate/registry ResourcePool не имеют.
- Lease fence, run epoch и physical quiescence остаются источниками истины для
  исполнения; pool не может заменить их логическим reservation.
- Resource policy проекта не доказывает membership, fairness или accounting
  первого класса.
- UI может показать текущие caps без ожидания ResourcePool extension.

## Предлагаемое расширение

- Спроектировать project-scoped pool identity, policy/version и объяснимые
  admission decisions для shared quotas.
- Описать relation pool к provider account, host capacity, budget или иной
  ресурсной единице, не предполагая, что все они имеют одну физику.
- Определить claim/release lifecycle и audit, совместимые с retry, stop,
  recovery, reassignment и потерянным Supervisor observation.
- Сохранить Core как единственного автора admission и canonical state.

## Не в границах

- Новый scheduler, billing system, autoscaling, quota marketplace или cloud
  orchestration.
- Замена capacity/fence/lease протоколов M0–M3 и выдача Run без них.
- Обязательные pools для каждого Project/Employee или UI prerequisite M4.
- Реализация data model, admission algorithm либо physical metering сейчас.

## Зависимости

- UI3.3 задаёт операторские contracts для resource presentation и controls.
- Existing capacity, budget и fencing contracts должны быть прочитаны как
  инварианты, а не переписаны данным extension.
- EXT2.1 design не блокирует UI0–UI4, M4 installer или текущие caps.

## Исследовательские решения до task breakdown

- Какие units pool действительно моделирует и какие остаются раздельными:
  concurrency, token/cost budget, host slots, provider account limits.
- Как scope, membership, hierarchy и priority/fairness работают между Project,
  Employee, provider profile и WorkSurface.
- Чем logical reservation отличается от подтверждённого Run observation и когда
  reservation освобождается после failure/recovery.
- Как выбрать atomic boundary с existing lease fence/epoch и объяснить отказ
  admission пользователю без раскрытия чужих секретов.
- Как version, policy change, audit и degradation ведут себя при недоступном
  metering source или неполной capacity information.

## Будущая декомпозиция

После утверждения дизайна возможны policy model, admission integration, audit
projection, operator read model и bounded controls; это не разрешение на реализацию.

## Exit gate

Утверждённый design record задаёт resource units, scope, reservation semantics,
atomic interaction с fence/epoch и degraded-mode policy. Отдельное решение
нужно до любых storage, scheduler или UI изменений.

## Проверка

- Проверить trace конкурирующих claims, stop, force-stop, retry и host reboot.
- Проверить, что stale/foreign Run не получает resource authority от pool.
- Сверить UI3.3 contract с объяснимыми admission decisions, а не с raw secrets.

## Риски

Главный риск — принять логический pool за физическое доказательство остановки
или исполнения; typed units, audit и existing fences уменьшают этот риск.
