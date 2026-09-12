# Epic UI3.3 — Resources и Settings существующего runtime

**Milestone:** UI3 — Knowledge loop и операционная конфигурация
**Статус:** planned; реализация не начата
**Источник:** [UI plan](../UI_IMPLEMENTATION_PLAN.md),
[backend alignment](../UI_BACKEND_ALIGNMENT.md)
**База backend:** `1439211`; dashboard отражает существующий admission model.
**Зависимости:** UI0.1–UI0.3, UI1.1, UI2.1, UI2.3.

## Цель

Показать реальные ограничения и занятость локального Forge, доступные runtime
settings и способы их изменения. Отделить заданные лимиты от измеренного
потребления, а поддерживаемые настройки — от запланированных возможностей PRD.

## В границах

- Host/project/credential-account admission caps и retained occupied Runs.
- Employee max concurrent Runs и per-Run CPU/RAM/PIDs/time/output budgets.
- Известный usage, unknown measurements, readiness и диагностические причины.
- Безопасные metadata profiles/credential references и настройки восстановления.
- Read-only startup settings с явным указанием способа изменения через operator
  configuration; формы только для уже существующих named commands.
- Ссылки на настройку Employee runtime в UI1.1 и SystemJobs в UI3.2.

## Что уже есть и каких чтений нет

Backend хранит singleton admission policy; startup environment задаёт host,
project и account caps, изменение требует общей physical quiescence.
`amend_employee` меняет Employee capacity, `configure_employee_runtime` — future
Run binding. ResourceLimits применяют CPU/RAM/PIDs/time; это лимиты, не метрики.
Отдельная SystemJobPolicy задаёт concurrency/attempt/input/output allowances.

Нужны read projections существующих caps/occupancy, безопасных runtime/profile/
credential metadata и boot recovery configuration. `/metrics` и Run diagnostics
помогают диагностике, но не являются готовым типизированным Resources DTO.
Опорный код: `crates/forge-storage/src/admission.rs`,
`crates/forge-domain/src/runtime.rs`, `crates/forge-domain/src/employee.rs`.

## Контракты и зависимости

- UI0 согласует bounded DTO и OpenAPI; UI2.1 даёт actual Run evidence, UI2.3 —
  management state. Dashboard не пересчитывает собственное право на dispatch.
- Lease и physical reservation могут удерживать один Run: это одна занятая
  единица. Lease expiry, failed provider и stop request не освобождают её в UI.
- Startup caps остаются read-only до отдельного одобрения online command;
  браузер не пишет env-файлы, не перезапускает Core и не меняет БД напрямую.
- Лимиты и observations имеют разные labels, timestamps и unknown state;
  отсутствующие cost, CPU/IO series или account usage не заполняются нулями.
- Credential reads возвращают только разрешённые metadata/status. Raw secrets,
  auth files, sealed records и arbitrary local paths не становятся settings form.
- Формы Project schemas/catalogs допустимы лишь после согласования настоящих
  command contracts; наличие поля в макете не доказывает backend CRUD.

## Не в границах

Generalized ResourcePool с именами normal/heavy/review/verification, pool drain,
IO quotas, adaptive scheduler и новые online host-budget commands.
Reusable SkillPack, CodeIndex registry, provider discovery/model marketplace,
secrets CRUD и M4 installer остаются отдельными продуктово-доменными работами.
Существующее SQL-поле skills, заполняемое пустым массивом, не является каталогом.

## Направления будущей декомпозиции

1. Уточнить available/unavailable поля Resources и безопасные settings reads.
2. Построить canonical occupancy/caps projection без второго capacity ledger.
3. Подключить dashboard и существующие формы с revision/confirmation handling.
4. Проверить held ownership, неизвестные измерения и ограничения startup config.

Это направления работ; отдельные TASK-ID и файлы задач создаются позднее.

## Exit gate и проверки

- Dashboard отличает configured limit, observed value, unknown и stale data.
- Один Run с Lease и reservation учитывается один раз; failed/stopping Run
  остаётся occupied до канонического подтверждения physical quiescence.
- Startup-only settings не предлагают несуществующее online сохранение.
- Employee/runtime mutations используют реальные команды и сохраняют старые
  Run snapshots; stale revision не перезаписывает новое состояние.
- Contract/browser tests проверяют bounds, empty/unavailable backend, keyboard
  access и отсутствие credential material в responses, storage и browser logs.

## Риски

Фиктивные named pools создают впечатление несуществующей scheduler policy.
Мгновенный красивый график без наблюдений опаснее честного unknown; настройки
не должны обещать денежную стоимость, которую CLI-подписка не сообщает.
