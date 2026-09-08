# M2: Employee management foundation

Employee — identity, а не единственный процесс. Один Employee может исполнять
несколько независимых Task при свободной настроенной capacity. Каждый Run
по-прежнему имеет собственные Lease/fence, context, provider session и private
home; Task и TaskWorkSurface не получают второго writer.

## Команды

Все команды используют общий `POST /v1/commands/{name}` и обязательные
Project `expected_revision` и idempotency key. Payload дополнительно содержит
`employee_id` и положительную `expected_employee_revision`.

| Команда | Дополнительный payload | Результат |
| --- | --- | --- |
| `amend_employee` | непустой `patch`: `name`, `role`, `stage_eligibility`, `max_concurrent_runs` — каждое поле optional | новая catalog revision для будущих назначений |
| `enable_employee` | нет | разрешает новые назначения |
| `disable_employee` | нет | запрещает новые назначения |
| `retire_employee` | нет | окончательно запрещает новые назначения; история сохраняется |

При новой принятой команде Employee revision увеличивается на один. Replay
возвращает прежний receipt и Event IDs. State, Employee Event, Project revision,
outbox и receipt фиксируются атомарно. Несовпадение Employee revision — conflict;
чужой Employee не доступен через Project scope.
События сохраняют имя, роль, допуск к стадиям, состояние и capacity Employee,
поэтому последующие изменения не стирают прежнюю конфигурацию из истории.

Ни одна из этих команд не останавливает действующие Run и не меняет их pinned
runtime/profile/context. Stop — отдельное явно запрошенное действие. Уменьшение
capacity ниже текущей занятости не вытесняет работников: оно блокирует новые
назначения до освобождения достаточного числа слотов. Retired Employee нельзя
вновь включить, выключить или перенастроить; повторный retire не удаляет историю.

## Capacity и совместимость

`max_concurrent_runs` — целое от 1 до 65535, default `1`. Это предел admission,
не обещание доступных ресурсов хоста или лимитов провайдера. Старые snapshots
декодируются с revision/capacity `1`; migration `0015` переносит существующую
индексированную revision в canonical snapshot и добавляет capacity.

Scheduler блокирует Employee row и повторно проверяет занятую capacity перед
выдачей Lease. Активная Lease и physical reservation одного Run занимают один
слот. Revoked/expired Lease не освобождает слот, если среда ещё не подтверждена
как quiescent. Уникальности writer по Task и Surface сохранены.

TaskStage, taskless Communication и Resolution занимают ту же Employee capacity;
provider-free Hook не создаёт фиктивного Employee. Дополнительно действуют
[общие admission limits](M2_ADMISSION_LIMITS.md) для локального хоста, Project и
credential/account. Это ограничения числа Run, не общий денежный budget и не
обещание параллельного использования любой подписки. Provider-specific delivery,
изолированные auth snapshots и writeback проверяются отдельно.

## Проверки

Domain tests проверяют legacy defaults, revision, capacity, immutable прошлый
snapshot и окончательность retirement. Общая command conformance выполняется
на memory и PostgreSQL: named mutations, replay, stale writes, scope/authority
refusals и rollback после каждой durable write boundary. PostgreSQL capacity
tests проверяют гонку за последний слот, physical retention после revocation,
default `1` и отсутствие неявного stop при изменениях catalog.
