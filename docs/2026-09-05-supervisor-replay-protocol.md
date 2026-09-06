# Supervisor: execution identity и replay

**Дата:** 5 сентября 2026

**Область:** TASK-11; transport foundation для TASK-12–19.

Core остаётся единственным владельцем canonical state. Локальный журнал
Supervisor — operational state для reconciliation и доставки, не вторая доменная
база данных. Supervisor не получает credentials PostgreSQL или NATS.

## Lifetime и идемпотентность

Registry и worker lifetime принадлежат процессу Supervisor, а не gRPC stream.
Потеря Core connection не останавливает worker. Новый stream начинает отдельный
Hello, сохраняя `supervisor_instance_id` и время старта процесса.

Перед запуском worker Supervisor сохраняет исходный Provision и scope
`(run_id, lease_fencing_token, environment_epoch)`. Повтор с тем же payload и новым
`command_id` не запускает worker ещё раз. Изменённый payload того же scope
отклоняется. Следующий epoch допустим только после подтверждённой quiescence
предыдущего epoch; меньший fence или epoch не принимается.

Завершённые identities сохраняются как tombstones, в том числе после ACK всех
сообщений. Неизвестное состояние не является разрешением повторить работу.

## Delivery

Каждое наблюдение/submission сохраняется до передачи в gRPC. Message ID, sequence
и payload остаются неизменными при replay. ACK `accepted`, `ignored_stale` или
окончательный `rejected` удаляет сообщение из pending, но не сбрасывает sequence.
Ошибки `canonical_storage_unavailable`, `supervisor_channel_unavailable` и
`internal_error` не подтверждают сохранение: Supervisor переподключается с обычной
задержкой и повторяет оригинал.

В каждом scope только одно сообщение ожидает ACK: `N+1` не обгоняет отказ `N`.
Разные Run могут передавать сообщения параллельно.

ACK отражает решение Core, а не истинность произвольного provider output.
Inventory и Hello не входят в run-scoped sequence.

## Inventory

Core отправляет `RequestInventory(command_id)` после Hello. Supervisor отвечает
`SupervisorInventory` с message ID, correlation ID, текущими host/boot и полным
списком ещё требующих reconciliation identities. Повторный или конфликтующий Provision также получает
inventory, correlated к его command ID, без запуска второго worker.

Quiescent tombstone можно исключить из inventory только после durable `accepted`
ACK именно наблюдения `Stopped` и опустошения его pending replay. Подтверждение
хранится отдельным `quiescence_confirmed` flag; сам tombstone остаётся в journal
и продолжает запрещать duplicate execution. ACK `StartFailed`, `ignored_stale`
или `rejected` такого подтверждения не даёт. Active, Unknown, unacknowledged и
старые tombstones без explicit confirmation всегда остаются в inventory.

Общий предел одного inventory — 4096 unresolved identities. При достижении
предела Supervisor явно отказывает в admission нового scope (`InventoryCapacity`),
не обрезает список и не удаляет историю. Подтверждённая завершённая история не
расходует этот лимит, но по-прежнему входит в отдельный byte budget journal.
Paging не реализован: legacy journal, уже содержащий более 4096 неподтверждённых
identities, требует отдельного безопасного reconciliation/migration процесса;
новый admission не может создать такое состояние. Это не lifetime limit в
4096 выполненных задач и не автоматическое разрешение удалить old records.

Entry содержит run/fence/epoch, последний сохранённый sequence, исходный boot ID,
environment ID и presence:

| Presence | Значение |
|---|---|
| `active` | Текущий процесс владеет зарегистрированным worker |
| `quiescent` | Исполнение физически завершилось; это не verdict задачи |
| `unknown` | Нет актуального доказательства существования или остановки |

После перезапуска Supervisor сохранённый `active` становится `unknown`. Pending
сообщения сохраняются и повторяются. До первого inventory реальный Podman backend
проверяет retained RunSpec и выполняет initial physical inspection: максимум
16 проверок параллельно, общий deadline 10 секунд. Только inspected running
container с matching scope labels возвращает `active`; неизвестный результат
или deadline оставляет `unknown`, с дальнейшим monitor и stop control. Проверка
не запускает контейнер заново. Повреждённый retained RunSpec останавливает startup
до adoption, без угаданных limits или ложного `not_started_confirmed`.

Для fake executor нет безопасного adoption: он не перезапускается автоматически.

## Operational storage

Binary принимает `--state-directory PATH`. Default —
`$XDG_STATE_HOME/forge/supervisor`, затем
`$HOME/.local/state/forge/supervisor`, затем `/tmp/forge-supervisor-state`.
Последний fallback не гарантирует сохранность после reboot; daemon setup должен
указывать постоянный путь. Library `SupervisorConfig::new` размещает state рядом
с socket; это удобно для изолированных тестов, но daemon composition переопределяет
путь при использовании volatile runtime directory.

Directory должен быть owner-only (`0700`), файлы — `0600`; symlinks и hardlinked
state files отклоняются. OS file lock запрещает два процесса с одним journal.
Замена snapshot проходит через новый private файл, `fsync`, atomic rename и
`fsync` directory. Другой host или неподдерживаемый journal version отклоняются.

Размер journal по умолчанию ограничен 16 MiB (`journal_max_bytes` в config).
Переполнение отклоняет admission/append, не продвигая sequence. Автоматической
очистки identities нет: после quiescence и ACK всех сообщений полный RunSpec
компактируется до scope/task identity и SHA-256 immutable request. Это сохраняет
проверку повторов без постоянного хранения prompts. Дальнейшая retention policy
остаётся TASK-13. Journal
не предназначен для raw logs; credentials не входят в Provision или сообщения.

## Проверки TASK-11

Unit tests проверяют duplicate/conflicting Provision, сохранение sequence и
pending после reopen, ACK compaction, временный отказ Core, capacity и private
storage/lock. Реальный UDS integration test разрывает Core stream во время Run,
повторяет Provision после reconnect и проверяет один execution, оригинальные
message IDs и нормальное завершение без скрытого stop/restart.

M1 regressions дополнительно сохраняют 4097 confirmed historical Runs вместе с
Active/Unknown/unacknowledged identities, проверяют admission capacity и legacy
flags. Локальный inspection-only test double задерживает `inspect` на секунду:
первое inventory ждёт результата и сообщает `active`; ошибка inspect сохраняет
`unknown`. Эти проверки не удаляют retained technical evidence.
