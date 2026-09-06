# M1: watchdog и recovery

Core остаётся единственным владельцем решения о дальнейшей работе. Supervisor
поставляет физические наблюдения; ни exit code, ни inventory не завершают stage.

## Часы и остановка

`watchdog_tick(now, deadlines)` сравнивает переданный clock с сохранёнными
canonical timestamps. Этот clock нужен только для deadline-наблюдения, включая
детерминированную fault injection. После захвата Project lock Core отдельно
выбирает время доменного изменения: `max(observed_wall, project.updated_at)`.
Это сохраняет causal ordering при ожидании lock и откате системных часов;
сам откат с исходным wall time попадает в WARN. Сохранённый временной порог
может временно опережать wall time. Переданный watchdog deadline не становится
временем доменного изменения, а Domain по-прежнему отклоняет регрессивные даты.

Queue eligibility, Lease expiry, liveness и время получения наблюдений используют
реальные часы без этого порога. Audit-only Events также могут иметь немонотонные
timestamps: порядок истории задаёт `project_sequence`, а не сортировка по времени.
Default: start deadline 120 секунд, liveness deadline 90 секунд, stop grace
30 секунд. `Running` и `Heartbeat` обновляют liveness только после принятия
точного fence/epoch/sequence. Текст, progress и submissions эти часы не обновляют.
Provider-reported timestamp сохраняется как evidence и не управляет deadline.

Пропущенный deadline создаёт typed Incident, отзывает Lease, закрывает доступ
через Gateway, добавляет `Interrupted` wait и immutable interrupted Handoff.
Physical reservation остаётся занятой до положительного подтверждения остановки.
Core запрашивает graceful stop; после grace сохраняет force-stop request и
повторяет доставку. Повторные ticks не создают дополнительные Handoff или
Incident того же класса. Никакой timeout не доказывает `not_started_confirmed`.

Рестарт Core получает ограниченный startup grace для повторного подключения
Supervisor. При прежнем boot живой Run сохраняет identity и Lease. Core повторно
поднимает его Gateway, но не запускает provider заново.

## Inventory и host reboot

Только ответ на текущий `RequestInventory` открывает dispatch после attach.
Повторный или unsolicited inventory не запускает новый recovery/dispatch cycle.
Каждая физическая запись дедуплицируется в одной транзакции со своим audit.
Отсутствующий Run — неопределённость, а не разрешение на повторный запуск.

Core сохраняет один local host/boot ledger. Run reservation получает эту
identity при выдаче. Изменение boot на том же host отзывает pre-boot Lease и
позволяет освободить его physical reservation: старый процесс не переживает
перезагрузку. Изменение host не доказывает остановку прежней среды; reservation
остаётся quarantined. Обе ситуации создают Incident и interrupted Handoff.
Project/host/boot receipt делает recovery устойчивым к падению Core между
обработкой отдельных проектов и обновлением общего host ledger.

В M0 simulator boot reconciliation и watchdog M1 отключены: тестовый fake boot
не может менять physical authority реальных Run.

## Управление

Команды используют стандартные Project revision, actor, idempotency key,
transactional event и outbox:

- `configure_boot_recovery_policy`: `{"policy":"recover_safe_then_hold"}`.
- `accept_run_recovery_assessment`:
  `{"run_id":"<uuid-v7>","assessment":"not_started_confirmed"}`.

| Policy | Обычная очередь после reboot | Accepted `not_started_confirmed` |
|---|---|---|
| `manual_hold` | Удерживается | Только фиксируется; требуется explicit resume |
| `recover_safe_then_hold` (default) | Удерживается | Отдельная новая attempt |
| `reconcile_then_resume_queue` | Возобновляется | Отдельная новая attempt |

Recovery hold — отдельный scheduler gate; он никогда не открывает остановленный
пользователем Project breaker. `start_project_execution` явно снимает hold.
Default policy пропускает только конкретную queue entry принятого safe recovery,
а не все задачи проекта и не последующие stages этой Task.

Принять `not_started_confirmed` можно только для retired Run с подтверждённой
physical quiescence, без противоречащих сохранённых Artifacts/accepted Handoff,
пока Task остаётся на том же stage. Core снимает только recovery wait этого Run;
другие blockers остаются активными. Новый Run получает новый Lease/fence и
предыдущий Handoff. Partial work, possible external effects и unknown никогда
не запускают работу автоматически. Accepted assessment для Run фиксируется
один раз; дальнейшие management действия используют обычные named commands.

Это не semantic evaluator: человек принимает assessment. Возможный будущий
evaluator будет поставлять candidate evidence, но не принимать его сам.

## Proxy cleanup

Отзыв scoped LiteLLM key повторяется также для уже quiescent/retired Runs.
Ошибка внешнего broker не возвращает старому Run execution scope. Pending
revocation остаётся видимой до успешного завершения; upstream credentials
не раскрываются при cleanup.

Перед внешним выпуском ключа сохраняется `issuance_pending`. Для одного Run
выпуск сериализован коротким issuance lock, но отзыв не ждёт его завершения.
Если отзыв увидел 404, а задержанный запрос затем создал ключ, Core выполняет
повторный compensating delete. Только завершивший собственный единственный
запрос процесс может снять `issuance_pending`; старую неопределённость после
crash/потери ответа последующий успешный запрос не снимает.

Pending intent продолжает попадать в cleanup даже при заполненном `revoked_at`
и без сохранённого key hash. Между попытками действует durable backoff 30 секунд.
Локальный `expires_at` не закрывает неопределённость автоматически: LiteLLM
выпускает ключ с относительным `duration`, поэтому задержанный server-side
запрос может сдвинуть его фактический срок. Без доказательства завершения старого
запроса cleanup продолжает повторы; Gateway всё это время запрещает retired
Run. Это сознательно консервативная граница, не гарантия отмены чужой HTTP-работы.

## Проверки

`recovery_acceptance` использует PostgreSQL, NATS и настоящий UDS/gRPC transport,
synthetic auth и manual Supervisor. Provider inference отсутствует. Failure
injection покрывает stale heartbeat, liveness deadline, force escalation,
удержание writer reservation, same-boot reconnect и три boot policy.
Дополнительные регрессии удерживают Project lock при watchdog tick и передают
будущий deadline clock; последующая обычная management команда остаётся валидной.
`canonical_clock_acceptance` проверяет сохранённый порог на час впереди wall time,
ожидание Project lock и цепочку dependency → queue → dispatch → outcome. Отдельные
SQL assertions подтверждают, что очередь, Lease и liveness не получают этот порог.

Каждый `M0Harness` создаёт собственную PostgreSQL schema
`forge_test_<uuid>` и применяет embedded migrations только в ней. Все соединения
пула используют только эту schema, без fallback на `public`: очередь, boot ledger,
credentials и migration ledger разных fixtures не смешиваются. Регрессия
`harness_isolation` оставляет неподхваченную очередь первого Core и проверяет,
что attach второго Core не трогает её. Схемы успешных и неуспешных тестов остаются
для диагностики; автоматического удаления или очистки старой общей БД нет.
Общий integration target остаётся последовательным (`--test-threads=1`), чтобы
ограничить нагрузку на локальный Podman и внешние fixture-сервисы.

`proxy_issuance_acceptance` использует real Core/SQL и локальный HTTP broker
test double: задержка create после revoke и потеря ответа с повторным появлением
ключа проверяют compensating delete и durable pending cleanup. Это failure
injection контракта, а не доказательство поведения настоящего LiteLLM сервера.
