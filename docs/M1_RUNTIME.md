# M1: локальный runtime

M1 связывает canonical Core с изолированным employee-процессом. Управление
остаётся в Core; shell работает внутри выданного sandbox. Pipeline принимает
только явный typed outcome с требуемыми артефактами. Выход процесса, ответ модели
и summary сами по себе не закрывают Task.

## Граница реализации

| План | Реализованный участок | Проверка |
| --- | --- | --- |
| TASK-11 | UDS gRPC, durable Supervisor journal, replay/ACK, inventory | protocol/session tests |
| TASK-12 | rootless Podman, Task-private Git/filesystem/none surfaces, RO snapshots | actual Podman fixture tests |
| TASK-13 | frozen context/RunSpec, immutable handoff, redacted evidence spool/MinIO | domain, MinIO and M1 integration tests |
| TASK-14 | scoped HTTP/MCP Gateway, fixed catalog, revocation and separate dedupe | Gateway acceptance tests |
| TASK-15 | watchdog, incidents, physical reservations, boot policies, explicit assessments | recovery failure-injection suite |
| TASK-16 | JSON tracing, W3C, bounded metrics, local readiness | observability tests and optional Prometheus adjunct |
| TASK-17 | encrypted Secret Store, immutable profiles, isolated delivery and CAS writeback | secret/profile tests |
| TASK-18 | Core-owned LiteLLM routes/keys and OpenCode API runtime | synthetic HTTP plus opt-in actual service tests |
| TASK-19 | Codex CLI wrapper, capability contract and offline image preflight | conformance, keyless pinned CLI probes; subscription smoke is opt-in |

Реальные компоненты и реальные LLM-запросы — разные проверки. Podman fixture
не доказывает поддержку Codex API. Actual OpenCode → LiteLLM → stub доказывает
транспорт и обработку протокола, но не доступность платной модели. Проверка двух
параллельных Codex Run через одну подписку требует явного выбора credentials и
разрешения потратить лимит; она не запускается общим test target.
Подготовленная opt-in команда — `just test-codex-live`; обязательные параметры
и границы этой проверки описаны в [Codex live guide](../infra/runtime/CODEX_LIVE.md).
Результат явно разрешённого live-прогона приведён ниже; повторный запуск также
требует разрешения расходовать лимит выбранного аккаунта.

## Результат проверки — 6 сентября 2026

Проверен рабочий срез TASK-11–19, включая одиночный реальный Codex Run и
двухпоточный subscription gate. API lane проверена с локальным upstream stub;
успех Codex не доказывает доступность платных моделей остальных провайдеров.

| Проверка | Результат |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| Workspace Clippy, all targets/features, `--locked`, `-D warnings` | PASS |
| `cargo test --workspace --all-features --locked` | PASS; opt-in тесты запускаются отдельно |
| `just test-integration` | PASS: MinIO, Podman fixture, Core/Gateway/recovery и CLI smoke |
| `just test-provider-integration` с явным image digest | PASS: реальные CLI, LiteLLM, OpenCode session и Core API lane; upstream — локальный stub |
| OpenAPI YAML и внутренние `$ref`, shell syntax, `git diff --check`, 500 эффективных строк на Rust-файл | PASS |
| `just test-codex-live` | PASS: два реальных Codex Run, общий auth binding, раздельные homes/surfaces, named stop; 1 passed за 28.61 s |
| Runtime build context | PASS: Podman копирует только семь явно разрешённых файлов из synthetic context |

Runtime image для provider gate:
`localhost/forge-runtime@sha256:b9f92552224c7f7d98cd725c4711c36b1703cc94260423406f8a4a9d17d2da83`.
После проверки работающих контейнеров с меткой `forge.managed` не осталось;
служебные сервисы и retained evidence не удалялись.

Независимое read-only review выполнено, подтверждённые findings исправлены и
покрыты регрессиями. Исправления включают стартовое inventory/adoption,
освобождение reservation после подтверждённого nonstart, неполный orphan spool,
адрес рабочего каталога CLI, отзыв inference во время запроса и монотонность
доменных изменений при откате системных часов. Повторное review этих изменений
не выявило новых P1/P2. Границы synthetic/provider/live проверок сверены отдельно.

Live profile: Codex CLI `0.153.2`, модель `gpt-6-astra`, выбранная ChatGPT
авторизация и указанный выше image digest. Private evidence root:
`/tmp/fc-01a076c6-52a4-7e71-9018-77268b87a0b4`. Этот локальный временный каталог
может быть очищен ОС; не публикуйте его содержимое целиком.

Run IDs: `01a076c6-5aa9-7410-9c95-4028ea20fd6d` и
`01a076c6-5b3a-7441-bf2d-1775a858efd1`. Оба создали маркеры в своих surfaces
и одновременно находились в Running. После `StopProjectExecution` тест получил
runtime reports и interrupted handoffs, Task остались в `waiting`. Независимая
проверка Podman подтвердила `running=false`, `status=exited`, `exit=0` у обоих.
Исходный auth не изменился; supplied tokens отсутствовали в проверенных
diagnostics. Это снимает live acceptance blocker M1, но не доказывает все
варианты OAuth refresh races или поведение других моделей.

### Повтор после TASK-05 — 6 сентября 2026

M0–M1 закрыты после выделения общего command engine. Новый reference backend —
это in-memory persistence для command tests, не память Employee. Контракт и
проверки описаны в [COMMAND_CONFORMANCE.md](COMMAND_CONFORMANCE.md).

На рабочем дереве поверх `b685480` прошли workspace tests, strict Clippy/fmt,
16 services-free и 17 PostgreSQL conformance tests, полный `just test-integration`
и CLI smoke (`done`, один Artifact). Shell launcher regressions также прошли.
Отдельный provider gate проверил реальные CLI, LiteLLM, OpenCode и Core API lane
с локальным upstream stub. Финальное независимое RO-review не выявило P0/P1/P2.

Один явно разрешённый повтор `just test-codex-live` завершился с
`1 passed; 0 failed` за **32.72 секунды**. Profile и image digest — те же, что
указаны выше. Private evidence root:
`/tmp/fc-01a0773d-191d-74d1-a8a3-dbf221c339ed` (owner-only, mode 0700).
Run IDs: `01a0773d-1d73-7723-b40d-16e7996e5c9e` и
`01a0773d-1dfd-78c2-894f-394a38b032e3`. Оба выполнили marker commands в отдельных
surfaces/homes и работали одновременно. Named stop оставил Task в `waiting`
и собрал runtime reports/interrupted handoffs. Source auth не изменился;
проверенные diagnostics не содержат выданных tokens.

После теста отдельно проверены точные контейнеры
`forge-run-01a0773d-1d73-7723-b40d-16e7996e5c9e-1-1` и
`forge-run-01a0773d-1dfd-78c2-894f-394a38b032e3-2-1`: оба имеют
`running=false`, `status=exited`, `exit=0`. Работающих `forge.managed` containers
не осталось. Evidence, exited containers и служебные данные сохранены.

## Быстрый ручной запуск

Для одной собственной задачи на текущем Linux dev-host:

```sh
cd /home/zov/projects/forge
just run-m1
```

Launcher предлагает текущую модель из простого top-level `model` в Codex config,
спрашивает текст задачи и подтверждение расходования подписки. В интерактивном
режиме источник auth по умолчанию — `$CODEX_HOME/auth.json` или
`$HOME/.codex/auth.json`. Его путь показывается до чтения credentials; можно
выбрать другой через `--auth-file`. Если файловой ChatGPT-авторизации нет,
launcher просит выполнить `codex login` или выбрать готовый snapshot. Keyring,
API key и автоматический login этим helper не поддерживаются.

Полностью явный вариант:

```sh
just run-m1 --auth-file /absolute/private/auth.json \
  --model '<выбранная модель>' \
  --task 'Создай README.md и проверь его содержимое.' --yes
```

`--yes` разрешает реальный запрос и расход лимита. Без TTY обязательны все три
параметра и `--yes`; скрытого выбора credentials или модели нет. Host config не
импортируется: модель используется только как подсказка, без profiles/hooks/MCP.

Каждый вызов создаёт отдельный Project, Employee и Task, новую PostgreSQL БД
`forge_m1_<uuid>` и приватный каталог `$HOME/.forge-m1/run.<random>`.
Core/Supervisor запускаются из свежих локальных binaries; image собирается
через обычный cached build и закрепляется по digest. Employee получает пустую
filesystem surface, shell и Forge MCP, но не checkout Forge и не свободную сеть.
Core проверяет обязательный `stage_evidence` и явный outcome `completed`.
Лимиты: 2 CPU, 4 GiB RAM, 256 PID и 600 секунд под наблюдением Supervisor;
это не независимый от хоста денежный или токенный лимит.

При завершении, ошибке или Ctrl-C launcher сначала вызывает named project stop,
затем проверяет физическую остановку. При проблемах с API/БД он отдельно проверяет
только собственные контейнеры по полному ID, host label и точному runtime mount.
Force stop или невозможность подтвердить остановку возвращают ошибку. Не прерывай
cleanup повторно. SIGKILL, потеря хоста или повреждение runtime требуют проверки
retained session; одного исчезнувшего launcher-процесса недостаточно.

Каталог сессии сохраняет `session.json` с IDs/именем БД/профилем запуска,
рабочие файлы, логи и Secret Store. Если результат был прочитан, `result.json`
содержит публичную карточку и артефакты; при ранней ошибке или Ctrl-C файла может
ещё не быть. БД и exited containers не
удаляются. Временная входная копия auth удаляется после encrypted enrollment и
настройки профиля; при ранней ошибке её убирает безопасный EXIT cleanup. Исходный
auth остаётся нетронутым. Per-Run copies обслуживает обычная Core cleanup policy;
при ошибке они могут остаться в приватной среде. Не публикуй каталог сессии целиком.

Проверки helper без credentials и inference:

```sh
bash scripts/tests/m1-input-test.sh
bash scripts/tests/m1-commands-test.sh
bash scripts/tests/m1-session-test.sh
```

Проверка `.containerignore` требует rootless Podman, но не credentials, сеть или
base image:

```sh
bash scripts/tests/runtime-build-context-test.sh
```

Тест строит scratch image только из искусственных файлов и проверяет точный
список скопированных путей. Контейнер не запускается; synthetic context, image
и созданный контейнер сохраняются для диагностики. Build context разрешает семь
конкретных файлов, а не целиком `infra/` или `target/`. Повторная сборка
`just build-runtime` с этим ограничением прошла и сохранила приведённый выше digest.

Launcher `run-m1` не заменяет `just test-codex-live`: один ручной Run не доказывает
параллельную работу двух Run с одной подпиской или корректность её refresh races.

Проверка 6 сентября 2026: один реальный `run-m1` с текущей выбранной ChatGPT
авторизацией и моделью `gpt-6-astra` создал `hello.txt`, приложил `stage_evidence`
и перевёл Task в `done` через MCP. Launcher завершился с кодом 0, контейнер
остановлен без emergency cleanup. Исходный auth не изменился; временная входная
копия удалена. Это подтверждение одиночного Run, не двухпоточного live gate.

### Инструменты среды разработки

Shell доступен внутри sandbox. Общий образ M1 содержит Git, Node и provider CLI,
но не Python и не универсальный набор toolchains; наличие Node не означает
наличие npm в PATH. Root filesystem read-only, а текущий Codex egress не открывает
пакетные репозитории. Поэтому произвольная установка через apt/pip/npm во время
Run не является поддержанным workflow.

Оператор может заранее собрать совместимый образ с нужными инструментами и
назначить его через `RuntimeBinding.image`. `just run-m1` пока использует общий
образ без параметра `--image`. Настраиваемые bundles, подготовка зависимостей
и wizard требуют следующего этапа дизайна.

Task `done` означает принятый Pipeline outcome с требуемым Artifact, а не
независимую проверку качества. Artifact может содержать исходник inline, поэтому
пустой worktree сам по себе не означает потерю результата. В ручном Python
сценарии 6 сентября код был сохранён в Artifact, файл не создавался, а отчёт
явно указал `python3: command not found` и отсутствие выполненных runtime tests.

## Подготовка

Нужен Linux x86_64 с rootless Podman, cgroup v2 и делегированными CPU/memory/PID
контроллерами. Forge не переключается на запуск employee прямо на хосте при
ошибке sandbox. Базовые сервисы запускаются через `just dev-init`, `just dev-up`,
`just migrate`; конфигурация находится в owner-only `var/dev/`.

```sh
just build-runtime
just build-runtime-fixture
just check
cargo test --workspace --all-features --locked
just test-integration
```

Actual CLI/API transport проверяется отдельно:
`FORGE_PROVIDER_RUNTIME_IMAGE='<image@sha256:digest>' just test-provider-integration`.
Сначала поднимите internal-only fixture по
[инструкции](../infra/dev/litellm/README.md). Общий integration target явно
показывает, что эти проверки в нём не запускались.

`just build-runtime` печатает OCI digest; именно `name@sha256:digest` указывается
в RuntimeBinding. Поддерживаемые pins: Codex CLI 0.153.2, OpenCode 1.18.29,
LiteLLM 1.99.0. Supervisor проверяет реальный `--version` в отдельном контейнере
без credentials, mounts и сети, прежде чем выдавать Run рабочую среду.

## Core и Supervisor

Выберите постоянный абсолютный owner-only каталог, например
`/home/USER/.local/state/forge/execution`, и отдельный Secret Store. Используйте
одинаковый execution root для обоих процессов. Родительские каталоги создаёт
оператор; Forge проверяет владельца, mode и отсутствие symlink-подмены.
Путь должен быть коротким: вместе с `gateways/<Run UUID>/gateway.sock` он
должен занимать менее 108 байт. Core проверяет этот Linux UDS limit до выдачи
credentials; длинное имя пользователя может потребовать другого state root.

```sh
cargo run -p forge-core --bin forge-secret-store -- \
  --directory /home/USER/.local/state/forge/secrets --initialize file
```

`file` — явный выбор owner-only master-key файла. Альтернатива `keyring`
использует доступный Linux Secret Service. Потеря или недоступность выбранного
master key закрывает доступ: автоматической генерации нового ключа нет.

Перед запуском передайте `FORGE_DATABASE_URL` и `FORGE_NATS_URL` через приватное
окружение. Не вставляйте credentials в аргументы shell, отчёты или логи.

```sh
cargo run -p forge-core --bin forge-core -- \
  --execution-root /home/USER/.local/state/forge/execution \
  --secret-store /home/USER/.local/state/forge/secrets
cargo run -p forge-supervisor --bin forge-supervisor -- \
  --state-directory /home/USER/.local/state/forge/execution --host-id local-forge
```

Это два долгоживущих процесса в отдельных терминалах. При нестандартном UDS
пути укажите Core `--supervisor-socket`, Supervisor `--socket`. M0 simulator
включается только явным `--fake-runtime`; реальный режим не использует его как
fallback. Installer/systemd wizard относится к M4.

## Credentials и профили

Команды используют общий `/v1/commands/{name}` envelope: `project_id`, актуальный
`expected_revision`, `payload` и `Idempotency-Key`. CLI принимает JSON из файла
через общий command interface; wire contract описан в `openapi/v1.yaml`.

1. `enroll_credential`: передайте UUIDv7 `secret_id`, `binding_id`, `kind`
   (`codex_chatgpt` или `api_key`) и абсолютный `source_file`. Файл должен иметь
   mode 0600; тело команды содержит путь, а не секрет.
2. `configure_employee_runtime`: укажите `employee_id` и `RuntimeBinding`
   с immutable `ExecutionProfile`, image digest, surface, limits, budget и двумя
   prompt. Profile `id + revision` нельзя переопределить другим содержимым.
3. Одобрите Task и откройте выполнение проекта. Каждый Run закрепляет профиль,
   Task context, credential version, scope, fence и epoch.

Codex получает отдельную копию ChatGPT auth в собственном `CODEX_HOME`.
Параллельные Run не делят writable auth-файл и не блокируют подписку на время
работы. После положительного подтверждения выхода Core делает короткий CAS:
актуальное обновление сохраняет новую encrypted version; конкурирующее —
encrypted recovery candidate и incident. Forge не является OAuth broker.

API lane требует отдельного LiteLLM сервиса: см.
[его настройки и проверки](../infra/dev/litellm/README.md). Core принимает
`--litellm-url` и `--litellm-master-key` с путём к приватному файлу. Только Core
передаёт upstream key сервису; employee получает Run-scoped virtual key.
Gateway разрешает фиксированные inference routes/model alias и sampling fields,
отклоняя request-level overrides credentials, proxy routes и бюджета.

## Остановка, восстановление и отчёты

`stop_project_execution` закрывает новые claims и Run scopes, затем запрашивает
мягкую остановку всех активных Run. Grace берётся из закреплённых limits;
watchdog может перейти к force stop. Task переходит в waiting независимо от
физического завершения. Reservation Task/surface/employee сохраняется, пока
Supervisor не подтвердит quiescence: второй writer не запускается по одному
лишь revoked Lease или принятому outcome.

После reboot Core применяет `manual_hold`, `recover_safe_then_hold` (default)
или `reconcile_then_resume_queue`. Настройка — `configure_boot_recovery_policy`.
`accept_run_recovery_assessment` записывает решение оператора о предыдущем Run;
`unknown` и возможные side effects не означают разрешение повторить работу.
Подробнее: [recovery contract](2026-09-06-recovery-implementation.md).

GET `/v1/projects/{project_id}/runs/{run_id}` возвращает `diagnostics`: incidents,
handoff, redacted evidence receipts, provider exit/usage и observed proxy spend.
Счётчики могут быть `null`; расход LiteLLM может запаздывать и превышать порог
из-за запросов in flight. Нативной подписке Forge не приписывает выдуманную цену.

Для MinIO добавьте `--evidence-config /absolute/private/s3.json`. JSON содержит
`endpoint`, `region`, `bucket`, `access_key_id`, `secret_access_key`, `allow_http`.
Без object storage bounded spool сохраняет `pending_upload`; это не `stored`.
Object references/hash и canonical timestamps создаёт система, не Summarizer.

После exit, durable auth writeback и импорта обоих redacted streams Core удаляет
только временные delivery copies credentials и stdin. Encrypted records,
conflict candidates, work surfaces и logs сохраняются. Небезопасный returned
auth остаётся в карантине; неполный импорт evidence также удерживает delivery
copies для восстановления redaction. Это не гарантированное стирание блоков диска.

Ограничения: sandbox доверяет ядру/rootless Podman; владелец Forge на хосте всё
ещё привилегирован. Redaction знает выданные literal credentials, но не может
обнаружить любую их кодировку или неизвестный секрет. Лимит evidence admission
и stdout/stderr не является общей filesystem quota для writable поверхности.
Review/retry/integration Git-процесса, другие CLI, Summarizer и installer
остаются следующими milestones; они не подменяются заглушками M1.
