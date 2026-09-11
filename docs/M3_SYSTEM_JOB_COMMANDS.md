# M3: SystemJob management API

Все команды используют стандартный `POST /v1/commands/{name}`, Project
`expected_revision` и idempotency key. Payload не принимает `actor`, capability
или `operation`: действие определяет именованный маршрут. Local owner CLI
использует ту же границу как Human. Worker/Summarizer не получает эти команды.

| Command | Payload | Назначение |
| --- | --- | --- |
| `configure_system_jobs` | `expected_settings_revision`, `policy`, `binding` | Версионируемая настройка будущих SystemJob; `0` для первой конфигурации |
| `request_task_summary` | `task_id` | Явный запрос summary по canonical evidence указанной Task |
| `request_employee_onboarding` | `employee_id` | Явный запрос onboarding указанного Employee |
| `retry_system_job` | `job_id` | Явная повторная попытка после проверки состояния и physical quiescence; dispatch требует открытый stop gate |
| `skip_employee_onboarding` | `employee_id`, `reason` | Audited manager skip; не означает успешное onboarding |

`policy` — полный `SystemJobPolicy`. Он задаёт `enabled`, `max_concurrent`,
`max_attempts_per_job`, `max_attempts_per_day`, `max_input_bytes`,
`max_result_bytes`, `wall_seconds`, `coalesce_seconds`. Нулевые limits не
означают unlimited. По умолчанию semantic work выключена, concurrency `1`,
до `2` attempts на текущую generation job и `20` attempts за скользящие 24 часа, input `65536` bytes,
result `16384` bytes, wall limit `300` seconds, coalescing `5` seconds.

`binding` использует существующий immutable RuntimeBinding с явно выбранными
profile, image digest и credential reference. `surface` обязан быть `none`, а
ExecutionProfile — принадлежать Project команды. Это private scratch; SystemJob
не получает Task surface и не становится Employee. RuntimeBinding валидируется
до external work. Credential bytes не передаются в command JSON.

Новый Employee создаётся с onboarding `pending`, даже когда semantic work
выключена или ещё не настроена. До завершения onboarding либо явного
`skip_employee_onboarding` ему не выдаются Task, Communication и Resolution Run.
Чтобы начать без модели для onboarding, local operator должен явно выполнить
skip с причиной. Старые persisted Employees получают `legacy_bypass`; это не
запись об успешном знакомстве с проектом. Исторические M0/M1/M2 demo launchers
вызывают skip сами и сохраняют его audit, а M3 acceptance использует настоящий gate.

UUIDs должны быть v7. Settings revision — неотрицательный integer, остальные
state/revision проверки выполняются Core в транзакции. Skip reason обязателен,
не может состоять из пробелов и ограничен 2000 UTF-8 bytes.

`GET /v1/projects/{project_id}/system-jobs` возвращает policy, settings revision,
job states, attempts usage и onboarding receipts. Ответ не содержит runtime
binding или credentials. Native CLI monetary usage остаётся unknown; concurrency,
attempt allowance и local wall/output caps не объявляются денежным budget.

```sh
forge command request_task_summary \
  --project-id PROJECT_UUID --expected-revision CURRENT_REVISION \
  --idempotency-key RESERVED_RETRY_KEY --payload '{"task_id":"TASK_UUID"}'

forge command skip_employee_onboarding \
  --project-id PROJECT_UUID --expected-revision CURRENT_REVISION \
  --idempotency-key ANOTHER_RETRY_KEY \
  --payload '{"employee_id":"EMPLOYEE_UUID","reason":"Explicit manager decision"}'

forge get /v1/projects/PROJECT_UUID/system-jobs
```

Настройки и commands не запускают hidden fallback provider. Для live-проверки
нужны выбранные profile/image/account и отдельный bounded сценарий. Parser
tests проверяют UUID, обязательный reason и запрет подмены route/actor; они не
являются доказательством authenticated provider execution.

`m3_system_job_gateway` проверяет полный keyless-путь через настоящие Core,
PostgreSQL, NATS и UDS Gateway: компиляцию onboarding-источников, V7 provider stdin,
запрет чужих инструментов и источников, квитанцию/replay и принятие после
Supervisor `Stopped`. Второй сценарий создаёт Artifact через Gateway обычного
Task Run и получает TaskSummary с canonical Artifact refs и coverage. Supervisor
в этих тестах не исполняет provider; credential и выходной текст синтетические.
