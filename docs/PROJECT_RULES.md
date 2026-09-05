# Forge: Project Rules

**Статус:** принято для локального MVP
**Дата:** 4 сентября 2026
**Область:** правила реализации Forge до пересмотра архитектуры или границ MVP.

## 1. Источники истины

1. Продуктовая рамка — `../forge-prd-v0.1.md`.
2. Значение доменных сущностей и инварианты — `2026-09-03-*-domain-model.md`
   и `2026-09-04-*-domain-model.md`.
3. Технологии и deployment shape — `STACK.md`.
4. Границы модулей и интеграционные контракты — `ARCHITECTURE.md`.
5. Этот документ определяет правила реализации и проверки. Он не может
   переопределять документы выше.

При конфликте, пробеле или необходимости изменить инвариант разработчик сначала
создаёт явный proposal и обновляет нужный источник истины вместе с кодом. Нельзя
«уточнять» доменную модель неявным поведением реализации.

## 2. Общие инженерные правила

- **Обязательно:** изменение mutable state проходит именованную command через
  Core; прямые изменения canonical PostgreSQL state запрещены.
- **Обязательно:** команда проверяет actor, capability, idempotency key и
  expected revision там, где это требует публичный contract.
- **Обязательно:** state change, immutable Event и outbox record фиксируются
  одной транзакцией. Внешняя работа начинается только после commit.
- **Обязательно:** обработчики outbox/NATS считаются at-least-once и
  идемпотентны по стабильному идентификатору события.
- **По умолчанию:** сначала выбирается простое локальное решение. Новая
  асинхронность, отдельный процесс или shared abstraction добавляются только
  при доказанной необходимости.
- **Review trigger:** изменение доменного инварианта, границы trust или модели
  отказа требует отдельного описания причины и обновления документации.

## 3. Workspace и зависимости

Workspace следует утверждённой структуре из `ARCHITECTURE.md`:
`forge-domain`, `forge-application`, `forge-storage`, `forge-protocol`,
`forge-core`, `forge-supervisor`, `forge-provider-*`, `forge-cli` и
`forge-testkit`.

- **Обязательно:** зависимости направлены вниз: domain не импортирует HTTP,
  SQLx, Podman или SDK провайдера; Supervisor не получает storage repository;
  CLI не вызывает Core как библиотеку.
- **Обязательно:** shared crate появляется после доказанной общей потребности,
  а не для предполагаемого переиспользования.
- **Обязательно:** каждый новый crate, dependency и feature flag имеет
  обоснование в изменении. Dependency добавляется только через Cargo с
  обновлённым `Cargo.lock`.
- **По умолчанию:** используются стабильные, хорошо поддерживаемые зависимости;
  beta, git-зависимость или новая runtime-платформа требуют явного review.
- **Обязательно:** `Cargo.lock` коммитится для исполняемого продукта.

## 4. Код и границы модулей

- **Обязательно:** transport handlers, SQLx repositories, Podman calls и
  provider-specific parsing остаются adapter-слоями. Они не содержат доменную
  policy.
- **Обязательно:** domain types делают недопустимые состояния трудно или
  невозможно выразимыми; строковые статусы и невалидируемый JSON не заменяют
  typed value/enum на canonical boundary.
- **Обязательно:** внешние HTTP, CLI, MCP, NATS и provider payloads валидируются
  при входе. Недоверенные данные не становятся domain event без проверки.
- **Обязательно:** `Result` используется для ожидаемых отказов; `panic!`,
  `unwrap()` и `expect()` запрещены в production path. `thiserror` используется
  для library/domain errors, `anyhow` допустим только на binary composition
  boundary.
- **Обязательно:** `unsafe` запрещён, кроме изолированного FFI boundary с
  documented safety invariant, тестом и явным review.
- **По умолчанию:** передача заимствованием предпочтительна ненужному `clone`;
  изменение производительности начинается с измерения, а не с предположения.
- **Review trigger:** файл превышает 500 эффективных строк кода (без пустых
  строк и комментариев). Это повод проверить смешение ответственностей, а не
  жёсткий запрет. Ограничения на размер функции нет.
- **Обязательно:** комментарий объясняет причину, ограничение или workaround, а
  не пересказывает код. Каждый `TODO` ссылается на Task или issue.

## 5. Правила по системным частям

### 5.1 Core и application

- Только Core применяет lifecycle Task, Pipeline stage, QueueEntry, Lease, Run,
  Artifact, Handoff, escalation и policy state.
- Scheduler не обходит transaction boundary и не создаёт duplicate write-Run.
- Queue/Lease/Run activity не маскируется под lifecycle Task.
- Recovery не считает interrupted или pre-boot Run успешным без accepted
  evidence и валидного Pipeline transition.
- Summarizer, projection и metric failure не откатывают accepted command и не
  меняют Task/Artifact canonical records.

### 5.2 Supervisor и RunEnvironment

- Supervisor сообщает observed state, но не пишет доменные таблицы и не
  применяет lifecycle/Pipeline transition.
- `sandboxed_local` запускается только в rootless Podman. При невозможности
  provision Core возвращает понятную ошибку; тихий fallback в `host_process`
  запрещён.
- Run не получает host home, container socket, чужой WorkSurface, произвольную
  сеть или секреты по умолчанию.
- Shell внутри собственной выданной среды и WorkSurface допустим. Доступ к
  сети, MCP, чужим Task, проектной памяти, интеграциям и secrets проходит через
  Tool Gateway.
- Каждый RunEvent проходит проверку `run_id`, lease fencing token,
  `environment_epoch` и sequence до применения.

### 5.3 Providers, MCP и credentials

- `ExecutionProfile` pin-ится в `RunSpec`; fallback не меняет provider, model,
  credential mode или capability set молча.
- Provider adapter не создаёт Task, не принимает собственный текстовый verdict
  как domain truth и не пишет canonical storage.
- MCP — adapter Tool Gateway, а не обход его policy. Отсутствующий у provider
  MCP не меняет logical Tool Catalog.
- Raw credential не попадает в source code, `.env.example`, Task, Artifact,
  Event, ContextSnapshot, prompt, logs, traces или metrics.
- `proxy_only` — default для API lane. `isolated_runtime_secret` и
  `trusted_host` требуют explicit profile, audit и теста соответствующей policy.

### 5.4 Storage, память и artifacts

- PostgreSQL — canonical state; NATS — delivery; Redis — cache/short-lived
  coordination; MinIO — body artifact/raw log; AgentMemory — retrieval
  projection. Один слой не подменяет другой.
- Artifact acceptance, domain audit и raw technical log хранятся раздельно.
- Перед добавлением retrieved memory в ContextSnapshot Core повторно проверяет
  scope, visibility и canonical revision.
- Summarizer создаёт только derived entries. Он не меняет Task, Pipeline,
  Artifact или authoritative knowledge.

### 5.5 API, CLI и observability

- Публичные мутации — HTTP/JSON named commands; каждое изменение несёт
  idempotency key и expected revision по contract. SSE — read-only projection.
- Core ↔ Supervisor использует только authenticated local UDS gRPC/Protobuf.
- Process diagnostics используют structured `tracing`; correlation identifiers
  допустимы в log/trace, но не как Prometheus labels.
- Prometheus metrics имеют bounded cardinality. Запрещены labels с task/run/
  employee ids, путём, URL, prompt, секретом или произвольным provider text.
- Diagnostics по умолчанию не содержат prompt, provider response, tool body,
  OAuth material, secret или query parameters URL.

## 6. Configuration и secrets

- **Обязательно:** config и secret разделены. Несекретные sample values живут в
  documented config/example; реальные значения не коммитятся.
- **Обязательно:** новые environment variables документируются с назначением,
  owner, default и влиянием на безопасность. Secret никогда не имеет default.
- **Обязательно:** Forge Secret Store использует утверждённый AEAD path и
  owner-only fallback master-key file; самописная криптография запрещена.
- **Обязательно:** тесты и логи используют synthetic credentials и маскируют
  sensitive values до записи.

## 7. Проверки и тестирование

- **Обязательно:** новая domain logic, command, transition, retry/recovery rule
  или исправление bug получает regression test на наблюдаемое поведение.
- **Обязательно:** все critical paths имеют unit и integration coverage: command
  transaction/outbox, idempotency, fencing, lifecycle/Pipeline validation,
  single write-Run, stop/recovery и credential/tool policy.
- **Обязательно:** provider adapters получают contract tests на pinned runtime
  version. Поддержка provider не объявляется готовой только по наличию adapter.
- **По умолчанию:** domain/application tests используют fake clock, fake
  Supervisor и deterministic provider evidence. Реальные containers нужны для
  storage, NATS, Podman и integration boundaries.
- **По умолчанию:** coverage оценивается по риску, без глобального процента.
  Пропуск теста для persistent или security-sensitive поведения требует review
  justification.
- **Обязательно перед handoff:** `cargo fmt --check`,
  `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
  и релевантные `cargo test` проходят. Для изменённых integration boundaries
  запускается соответствующий integration/smoke scenario.

## 8. Git, документация и definition of done

- **Обязательно:** commit атомарен: одна логическая причина изменения, ясное
  сообщение и только относящиеся файлы. Не смешивать refactor, dependency bump
  и поведенческое изменение без необходимости.
- **Обязательно:** перед handoff выполняется `git diff --check` и фиксируются
  фактически запущенные проверки с результатом.
- **Обязательно:** изменение доменного contract, API/protocol schema, storage
  model, provider capability, security boundary, deployment или observability
  обновляет соответствующий документ и тесты в той же работе.
- **Обязательно:** implementation Task завершена, только когда код, migration
  (если нужна), тесты, documentation и проверочные команды готовы, а известные
  ограничения/непроверенные предположения явно указаны.
- **По умолчанию:** незавершённую работу не маскировать как done. Передавать
  следующему исполнителю canonical handoff: что изменено, что проверено, что
  осталось и какие есть риски.

## 9. Принятые пользовательские настройки

- Review trigger для размера файла: **500 эффективных строк кода**, без
  комментариев и пустых строк.
- Ограничение на размер функции не устанавливается.
- Strict rules действуют для canonical state, security, secrets, isolation,
  provider boundaries и recovery; в остальных местах применяются risk-based
  testing и review triggers.
