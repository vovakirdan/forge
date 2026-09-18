# ADR: локальная browser boundary

**Дата:** 18 сентября 2026
**Статус:** target принят; FRONTEND-007 — owner gateway, FRONTEND-008–010/012 — reads; FRONTEND-011/013/014 — draft text/priority/create commands (приёмка в Task)
**Область:** local-owner Control Room; remote control и Tauri отложены

## 1. Решение и уровень доказательства

Frontend поставляется как static React client. Тонкий
Rust/Axum gateway отдаёт assets и связывает browser с закрытым Core UDS:

`Browser → loopback Rust gateway → owner-local Core HTTP/JSON UDS`

В [FRONTEND-006](../tasks/frontend/frontend-006-static-hosting-boundary.md)
реализован только build/test path статического demo. Отдельная
[FRONTEND-007](../tasks/frontend/frontend-007-live-owner-gateway.md) добавляет
Rust gateway, bootstrap, sessions, CSP и первое Core Project read; фактические
результаты приёмки фиксируются в Task. Node file server FRONTEND-006 — тестовый
инструмент без доступа к Core, не production boundary и не auth prototype.
Успешный static smoke сам по себе не доказывает безопасность gateway.

[FRONTEND-008](../tasks/frontend/frontend-008-live-task-reads.md) расширяет этот
live entry read-only списком и карточкой Task, а allowlist — scoped Task и
PipelineVersion reads. Доказательство этого среза записывается отдельно в Task.
[FRONTEND-009](../tasks/frontend/frontend-009-live-run-reads.md) добавляет
Project-wide список/карточку Run с ограниченной диагностикой. Его проверки
также фиксируются отдельно; полные UI0.2/UI2.1 gates остаются открытыми.
[FRONTEND-010](../tasks/frontend/frontend-010-live-pipeline-reads.md) добавляет
Pipeline version list и read-only stage inspector. Это ранний срез UI1.2,
не editor или закрытие gate; его evidence фиксируется в отдельной Task.
[FRONTEND-011](../tasks/frontend/frontend-011-draft-task-edit.md) добавляет
первую мутацию — title/description draft Task через `amend_draft`.
[FRONTEND-013](../tasks/frontend/frontend-013-task-priority-edit.md) добавляет
`set_task_priority`, а [FRONTEND-014](../tasks/frontend/frontend-014-create-draft-task.md) —
создание draft через `create_task`. Остальные команды и SSE остаются закрытыми;
эти срезы не закрывают весь UI0.2/UI1.3.

| Вариант | Решение |
|---|---|
| Static client + отдельный Rust gateway | Принят: нет Node runtime в установленном продукте, доменная authority остаётся в Core |
| TanStack/Nitro SSR host | Сохраняется для прежнего demo/build workflow, но не выбран для установленного Forge |
| Management router Core на TCP | Отклонён: owner-local API нельзя открывать browser без новой auth boundary |

Static assets позже можно использовать в Tauri. WebView, IPC, desktop permissions
и packaging требуют отдельного решения; текущий ADR не обещает готовую desktop
интеграцию и не вводит её зависимости.

## 2. Build и runtime contract

В FRONTEND-007 runtime artifact — **`frontend/dist-live`**, отдельный plain-Vite
React entry с TanStack Query и общими styles/primitives. Для login/Project/Task/Run/Pipeline
не нужен router. Он не загружает mock root, Sidebar/TopBar, ProjectProvider,
service barrel или Lovable error reporter. Новый `just ui-build-live` не меняет
прежние demo/build/static workflows и не требует новых frontend dependencies.

`forge-live-manifest.json` содержит `format: "forge-live-v1"` и `files` с
`path`/SHA-256. Build gate запрещает inline scripts/handlers и remote scripts.
Gateway проверяет hashes, типы файлов и paths, затем хранит immutable snapshot
разрешённых assets: ≤8 MiB/file, ≤32 MiB total, manifest ≤64 KiB/256 files.
Запросы не открывают filesystem paths; unknown assets/API дают 404, не shell.
Manifest — allowlist/integrity check trusted owner artifact, не подпись издателя.

Исторический static proof FRONTEND-006 сохраняется отдельно:

- `just ui-build-static` использует отдельную Vite-конфигурацию с TanStack SPA
  mode, `nitro: false` и отключённым автоматическим внедрением environment.
  Версии, lockfile и обычный `just ui-build` не меняются.
- Artifact static demo — только `frontend/dist/client`: SPA shell, JS/CSS и public
  assets. SSR bundle, который toolchain создаёт для prerender на этапе сборки,
  не поставляется и не исполняется при обслуживании browser.
- TanStack SPA mode создаёт shell, но само по себе не запрещает server functions.
  Такие функции/routes нельзя добавлять как скрытую runtime-зависимость static
  клиента. [Документация TanStack](https://tanstack.com/start/latest/docs/framework/react/guide/spa-mode).
- Browser не получает server secrets через build, public assets или runtime
  config. Выбор публичных несекретных параметров в будущем будет явным;
  автоматический экспорт `VITE_*` для static target выключен.
- `just ui-test-static` использует отдельный file server на `127.0.0.1:4174`.
  Он читает только client output, без SSR import, proxy, shell или Core socket.
  Playwright владеет его запуском и остановкой; занятый порт означает отказ.
- Shell fallback допустим для UI navigation. Missing assets, traversal/root
  escape, `/api`, `/v1` и `/_serverFn` не превращаются в успешную HTML-страницу.
  API errors gateway тоже не должны скрываться за SPA fallback.

Build и smoke выполняются последовательно с ограничениями памяти. `vite preview`
не служит static proof: установленный Start preview plugin загружает server
bundle. Обычный demo на 5173 и прежний browser suite на 4173 остаются отдельными.

## 3. Rust gateway

`forge-ui` — отдельный native adapter, не новый домен и не второй Core. Он получает
только static assets, свой session state и фиксированный owner-local UDS path.
Не получает DB, NATS, provider keys, container socket, `CoreService` или право
запускать CLI/shell. Не импортирует management router Core в TCP listener.

Первый transport-срез разрешил authenticated health и Project read по известному
ID. FRONTEND-008 добавляет список/карточку Task и точную PipelineVersion внутри
Project scope; FRONTEND-009 — список/карточку Run, FRONTEND-010 — список версий
Pipeline. Health показывает доступность транспорта,
но не доказывает чтение DB или готовность Project. Для этого нужен отдельный
Project GET. В дальнейшем каждый method/path добавляется в allowlist явно;
универсальные proxy, URL, произвольные headers и filesystem paths запрещены.

Обязательные ограничения перед первым Core connection:

- Bind `127.0.0.1:0`: ОС выбирает свободный ephemeral port, затем gateway
  фиксирует и показывает фактический origin. Это не тестовый fixed port 4174.
  Wildcard bind,
  wildcard CORS, доверие `Forwarded`/`X-Forwarded-*` и автоматическое открытие
  сети отсутствуют. IPv6 и alias `localhost` не разрешаются неявно.
- Exact `Host` проверяется для всех запросов. Exact non-null `Origin` обязателен
  для bootstrap exchange и mutations, включая logout. Если Origin присутствует
  у read, он также должен совпасть. На same-origin GET browser может его не
  отправить: отсутствие Origin само по себе не заменяет проверку bearer token.
- Public surface — только login shell/assets и строго ограниченный exchange.
  Любой Core read, включая health, требует действующей session. Host/Origin —
  защита browser boundary, но не аутентификация. Native local
  процесс умеет подделать headers; authority даёт possession секрета.
- Bootstrap принимает только ожидаемый JSON content type и schema; logout
  имеет пустое тело и требует session/Origin. Draft command принимает только
  свой строгий JSON schema/content-type contract;
  cross-origin preflight не получает разрешающих CORS headers. Проверки не
  полагаются на один только браузерный CORS. [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html).
- UDS path задаёт trusted owner config, не request. Проверяются type/owner/mode
  socket и parent directory; ошибка приводит к отказу без TCP fallback. Клиент
  соединяется только с этим endpoint, не следует redirects.
- Каждый route имеет request/response byte limits, connection/read deadlines
  и bounded concurrency. Auth/control JSON ≤1 KiB; headers ≤16 KiB; Core body
  ≤64 KiB для health/Project/Task list/Run list; Task/Run detail и Pipeline version
  list/detail ≤1 MiB,
  включая chunked/error responses. Connect ≤1 s, целый HTTP/control
  exchange ≤5 s, включая idle/body/upstream; до 32 HTTP connections,
  8 API requests и 4 control connections, без неограниченной очереди.
  Ошибка Core даёт явный unavailable/timeout, не mock response или retry loop.
- Secrets, Authorization, bootstrap body и raw upstream errors не попадают в
  logs/traces; auth и API responses используют `Cache-Control: no-store`.
  Service worker не кэширует authenticated content.

### Draft command (FRONTEND-011)

Первый mutation path: `POST /api/commands/amend_draft`, без query.
Gateway отправляет исходное проверенное тело на фиксированный Core path
`/v1/commands/amend_draft`; браузер не выбирает URL, actor или произвольные
forwarded headers. Exact Origin и session обязательны до Core connection.

Envelope: `project_id`, `expected_revision`, `payload` с `task_id`,
`expected_task_revision`, `patch`. Patch содержит хотя бы одно поле `title`
или `description`, только строки; null и неизвестные поля отклоняются.
IDs — UUIDv7; положительные revisions должны оставлять место для безопасного
JS-integer increment. `Idempotency-Key` — один непустой header до 128 bytes;
live client создаёт UUIDv4 для каждой новой попытки Save.

Request cap — 512 KiB, receipt/error cap — 64 KiB. Auth/read caps не меняются.
Receipt проверяется на applied/replayed, Task resource identity, UUIDv7 command
и event IDs, непустой event list и исходную Project revision + 1. Browser
проверяет контракт повторно. Все gateway ошибки имеют finite `code` и fixed
`error`; raw upstream text не передаётся.

Один Save фиксирует body/key/revisions. При неизвестном результате повтор
использует те же bytes и key: replay Core проверяется до stale revision.
Нельзя обновить ревизию под тем же key. Stale Project остаётся
409/stale_revision, stale Task — 400/invalid_request; error string не служит
классификатором. После отказа пользователь явно перечитывает baseline и
сохраняет заново; UI удерживает только изменённые пользователем поля.

Успех показывается по receipt, а актуальные Project/Task/list перечитываются
отдельно. Ошибка этих reads не отменяет receipt. Draft и retry key находятся
только в памяти формы; уход требует предупреждения, поздний ответ не меняет
другой Project/session. Abort, logout и уход со страницы не отменяют Core commit.
После утраты локальной попытки нужно перечитать Task; durable offline replay
не входит в этот срез.

Сам AmendDraft сохраняет Task в draft; общая postcommit-dispatch фаза Core
не изменяется и может обработать другую готовую работу открытого Project.
Keyless acceptance использует отдельный stopped Project без Employees/ready
работы. Lost-response test теряет ответ после настоящего commit, а не заменяет
результат mock-success. Полные результаты и ограничения фиксируются в Task.

### Scoped Task reads (FRONTEND-008)

Новые paths: `GET /api/projects/{project_id}/tasks`,
`GET /api/projects/{project_id}/tasks/{task_id}` и
`GET /api/projects/{project_id}/pipelines/{pipeline_version_id}`. IDs — UUIDv7.
Только список принимает `limit` (1–100, default 20) и непустой opaque `cursor`
до 1024 UTF-8 bytes. Неизвестные/повторные query keys и некорректное encoding
отклоняются; gateway собирает upstream path/query заново, без raw forwarding.

Превышение declared или streamed response limit возвращает безопасный
502 с `code: response_too_large`; данные не обрезаются, Task не объявляется
повреждённой. Core list 409 с `error.code: cursor_invalid` становится безопасным
409 с `code: cursor_invalid`. Остальные raw upstream errors не раскрываются.

UI хранит текущие page/detail/pipeline, удаляя неактивные query records при
навигации; cursor history хранит только строки. Query keys включают session
generation и Project scope, detail/Pipeline IDs. Project switch, logout и 401
отменяют старые reads и исключают их поздние ответы. Refresh failure оставляет
явно stale данные. Invalid cursor предлагает первую страницу.

Pipeline читается только для выбранной карточки по pin свежего Task detail.
List показывает raw stage ID без fan-out. Неизвестная стадия и отсутствие
стадии различаются. Артефакты показаны только как ID/kind/title/date; inline body
и metadata приходят внутри канонического detail, но не рендерятся. Object refs
и URL не открываются и не загружаются автоматически. UI не выводит
Run activity, Employee ownership или priority labels из отсутствующих данных.

### Draft creation command (FRONTEND-014)

Третий mutation path — `POST /api/commands/create_task`, без query. Envelope:
`project_id`, `expected_revision`; payload: `title`, `description`, optional
`definition_of_done`, `kind`, `pipeline_version_id`, `priority`, `properties:{}`.
Принимаются только delivery/analysis и точная версия. Browser-срез не разрешает
альтернативный pipeline_id, непустые properties, actor, Task ID или lifecycle.
Core по-прежнему владеет policy и созданием ID/key. Существующие guards и limits
сохраняются. Title/description/DoD проверяются по ограничениям Core.

Create receipt содержит новый UUIDv7 Task; для amend/priority сохраняется
сравнение с исходным Task ID. UUIDv7 command/events, applied/replayed и исходная
Project revision + 1 проверяются для всех трёх commands. Unknown outcome
сохраняет body/key; replay не превращается в повторное создание.

Форма получает fresh PriorityScheme для revision и active/default priority.
Pipeline выбирается явно из страниц по 20, без скрытой загрузки всех страниц
или автоматического latest/default. Выбранный detail перечитывается отдельно;
удаление и несовместимый kind блокируют создание. Project scope задаётся endpoint
и окончательно проверяется Core. Conflict сохраняет все поля; refresh baseline
и следующая отправка — отдельные действия.

После receipt UI знает созданный ID, обновляет Project/list и читает новую Task.
Ошибка чтения не отменяет создание и предлагает только повтор reads. После
успешного readback открывается карточка. Ввод и retry key живут в памяти формы;
уход предупреждает об их потере, logout/scope change отбрасывают поздние ответы.
После потери локальной попытки сначала проверяют canonical список Task; новый
key не используется как «восстановление» неизвестного результата.

Draft не получает approval/очередь/Run. DoD и обязательные свойства могут быть
заполнены позже; этот срез не обещает готовности к исполнению. Общий postcommit
dispatch не меняется; keyless proof работает на stopped Project без Employees.

### Priority command (FRONTEND-013)

Второй mutation path — `POST /api/commands/set_task_priority`, без query.
Он использует те же guards, bounded transport и receipt validation, но отдельный
strict request: `project_id`, `expected_revision`, payload с `task_id`,
`expected_task_revision`, `priority` (stable level ID). Произвольные команды,
actor, headers и Core URLs не допускаются.

Редактор загружает свежие Task и PriorityScheme; envelope revision берётся из
`scheme.project_revision`. Одновременно открыт только draft или priority editor.
Выбор ограничен active levels для non-terminal Task; порядок каталога сохраняется,
unknown/retired ID не заменяется default. Неизменённый выбор не отправляется.

Conflict сохраняет выбранный ID; отдельный refresh baseline не является Save.
Недопустимый после refresh выбор остаётся видимым, но блокирует Save. Unknown
outcome повторяет только исходные bytes/key. Receipt подтверждает запись;
последующие reads обновляют Project/Task/list/catalog независимо от этого факта.
Scope/leave guards и отсутствие durable offline replay сохраняются.

Доменная команда не меняет lifecycle/stage и не прерывает активный Run/lease.
Core может обновить ожидающую работу; обычный post-commit dispatch сохраняется.
Browser proof использует отдельный stopped Project, без Employees/платных Runs.

### Project priority reads (FRONTEND-012)

`GET /api/projects/{project_id}/priority-scheme` обращается к новому Core
`GET /v1/projects/{project_id}/priority-scheme`. Это отдельный allowlisted GET,
без query, с UUIDv7, прежними owner/session/Host/Origin guards, timeout и
лимитом 64 KiB. Превышение лимита не превращается в частичный каталог.

Явный DTO содержит `project_id`, `project_revision`, `default_level_id` и
`levels` (`id`, `display_name`, `rank`, `retired`) из одного Project snapshot.
ProjectView не расширяется, storage representation наружу не передаётся.
Default должен существовать и быть активным; уровни уникальны, ranks — signed
i32, отрицательные и одинаковые значения допустимы. ID ответа проверяется
против запроса. Revision схемы — Project revision этого чтения, не новая сущность;
она не обязана совпадать с отдельно загруженным ProjectView.

Один scoped query обслуживает список и detail Task. Names выводятся как текст,
рядом с исходным ID; retired отмечается явно. Ошибка или unknown ID не подставляет
default. При failed refresh прежние names отмечаются stale, доступно ручное
обновление. Ошибка каталога не закрывает Task/draft editor. Scope cleanup и
generation fencing прежние; нет per-Task запросов, сортировки или invented colors.

Это priority-часть `project-task-catalogs`, не конфигуратор схемы. Настоящий Core
использует default-three, custom/retired cases проверяются отдельно на synthetic
responses. Evidence — в [Task](../tasks/frontend/frontend-012-project-priorities.md).

### Scoped Run reads (FRONTEND-009)

Добавлены `GET /api/projects/{project_id}/runs` и
`GET /api/projects/{project_id}/runs/{run_id}`. Это существующие Core GET через
owner UDS; новых Core endpoints, DTO schemas и миграций нет. IDs — UUIDv7;
только список принимает те же строгие limit/cursor, что Task list. Detail не
принимает query. List cap — 64 KiB, detail — 1 MiB. Oversize и cursor-invalid
обрабатываются так же, без partial JSON или raw upstream errors.

Раздел Runs показывает все Runs выбранного Project, не историю отдельной Task.
Purpose/owner, desired/observed states остаются независимыми; отсутствующие
Employee/Task IDs не подменяются. Diagnostics показывают наличие четырёх nullable
reports, loaded counts и stream completeness. `null` отличается от `{}`; ни
наличие report, ни process exit не подтверждают принятие результата Task.
Inline diagnostics целиком поступают в bounded detail response. UI не рендерит
bodies/raw JSON, URLs/object refs/paths и не делает дополнительных загрузок.
Auth/raw prompts исключает Core projection, а не скрытие полей в DOM.

Переключатель Tasks/Runs по умолчанию открывает Tasks. Смена Project сбрасывает
раздел; смена раздела не сохраняет selection и cursor history. Navigation сразу
отменяет reads, но сохраняет наблюдаемые query до commit смены области; cleanup
удаляет старые records после размонтирования. Это предотвращает повторный запрос
старого Project в промежуточном render. Session/project/page/Run keys и отмена
не дают поздним ответам восстановить старые данные. Refresh ручной; ошибка
сохраняет явно stale snapshot. Commands, polling/SSE и viewer не добавляются.

Run browser fixture использует отдельные Projects и M0 fake execution через
named commands, а не прямые DB writes или модели. Synthetic all-purpose cases
отдельны от real Core reads. Core всё ещё загружает все Runs Project до
пагинации; count bounds diagnostics могут дать ответ больше 1 MiB. Эти gaps
не скрываются усечением или фоновым скачиванием всей истории.

### Scoped Pipeline version reads (FRONTEND-010)

Добавлен `GET /api/projects/{project_id}/pipelines`; существующий detail path
`/pipelines/{pipeline_version_id}` сохраняется. Это allowlisted Core GET, без
новых Core endpoints, dependencies или миграций. IDs — UUIDv7; list принимает
те же строгие `limit` (1–100, default 20) и opaque `cursor` (непустой, ≤1024
UTF-8 bytes). Detail не принимает query. Owner auth, Host/Origin, UDS guards,
deadlines и concurrency limits остаются прежними.

List возвращает полные version DTO, поэтому его cap — **1 MiB**, как detail;
Task/Run lists по-прежнему ограничены **64 KiB**. Declared/streamed oversize
даёт безопасный `502/response_too_large`, cursor-invalid — `409/cursor_invalid`.
Нет partial JSON, скрытого уменьшения limit или adaptive retry. Core пока
загружает все версии до пагинации и читает catalog для каждой версии (N+1).
Даже допустимый page может превысить cap; это ограничение интерфейса, не
повреждение definition. Initial failure не подменяется empty list; неудачный
refresh сохраняет явно stale snapshot.

Третий раздел **Pipeline versions** показывает версии, не полный каталог
уникальных Pipeline. Нет total, pinned Task usage, editor или mutation buttons.
Выбор версии читает fresh detail: name, catalog revision, default/latest и
deleted_at изменяемы, immutable только definition. Default не обязан совпадать
с latest; soft deletion не делает историю недоступной. Весь read DTO нельзя
кэшировать навсегда как immutable version.

Detail показывает Task kinds, entry stage, max_stage_visits, stages/transitions
и artifact requirements. Read-only inspector по умолчанию выбирает entry stage;
unresolved reference остаётся явным ID, без поиска по имени. Executor/outcomes,
workspace/acceptance/system action показываются как typed configuration;
instructions — plain text, не HTML или executable Markdown. Неизвестные поля,
raw JSON и private hook runtime не становятся viewer или действием.
`null` означает «не настроено», не отсутствие WorkSurface, автоматическую
acceptance или гарантию неограниченного исполнения.

Tasks остаётся разделом по умолчанию. Навигация между Tasks/Runs/Pipeline versions
отменяет reads, сбрасывает selection/cursors и после unmount удаляет старые
query records. Keys содержат session, Project, page/version scope и независимы
от Task-specific Pipeline pin read. Late replies, logout/401 и reload следуют
правилам остальных reads; refresh только ручной. Fixture создаёт 23 + 1 версии
и empty Project через named commands, без provider Runs; synthetic policy и
failure cases помечаются отдельно от real Core reads.

## 4. Owner bootstrap и session — target contract

1. Gateway сначала успешно bind-ит listener и фиксирует origin. Затем owner
   вызывает `forge-cli ui login --control-socket …`: через отдельный private UDS
   генерируется single-use CSPRNG код с 256 битами энтропии и TTL **5 минут**.
   Код показывается только в owner terminal; пользователь вставляет его в UI.
   Он не попадает в URL/history, argv, env, provider context, telemetry или logs.
2. Browser отправляет код JSON POST на same-origin exchange. Gateway проверяет
   Host/Origin, размер/формат, TTL и атомарно consumes код. При двух одновременных
   exchange ровно один может выдать session. Invalid/expired/replayed код не
   раскрывается ответом; после пяти неверных попыток pending code инвалидируется.
3. Только явная owner CLI операция может выдать новый код; старый инвалидируется.
   Неаутентифицированного HTTP mint/reset endpoint нет. Интерактивный terminal
   вывод отделён от daemon journal. При отсутствии controlling owner TTY
   выдача отказывает безопасно: код не печатается в redirected stdout/journal.
   Control UDS: owner directory 0700, socket/lock 0600, peer UID проверяется
   с обеих сторон. Lifetime lock и проверка inode защищают от двух владельцев
   и очистки чужого socket. Протокол — 4-byte big-endian length + JSON ≤1 KiB,
   одна команда `issue_login_code` на соединение. Initial/reissued code идут
   одинаковым путём; daemon никогда сам не печатает код.
4. Успех возвращает новый opaque CSPRNG bearer token (256 бит) и absolute expiry
   **8 часов**. Сервер держит volatile session state; restart инвалидирует все
   sessions; максимум 16 активных sessions. Токен привязан к экземпляру gateway/owner authority, не является
   provider credential. Автоматического refresh нет.
5. Browser хранит token в `sessionStorage` и передаёт только в
   `Authorization: Bearer …` same-origin запросов. Не использует cookie,
   `localStorage`, URL/query, HTML или сохраняемый auth fixture. При storage
   error login явно неуспешен, без тихой смены механизма хранения.
6. Logout инвалидирует session на gateway и очищает local token. После 401,
   expiry или restart UI очищает auth state и показывает явный login; mutations
   не переотправляются автоматически. Потеря сети — unavailable, не новый login
   loop и не подтверждение успеха предыдущей команды.

Cookies не изолированы по port. Для нескольких сервисов на loopback cookie
может попасть другому local listener; поэтому здесь выбран explicit bearer
header. [RFC 6265 §8.5](https://www.rfc-editor.org/rfc/rfc6265#section-8.5).

`sessionStorage` отделяет origin и page session, но не гарантирует уникальный
   server session на tab: новая вкладка с opener может скопировать token. Копии
представляют одну server session, и logout инвалидирует их все. Открытие новых
окон использует `noopener`; отдельный login создаёт отдельную session.
[MDN sessionStorage](https://developer.mozilla.org/en-US/docs/Web/API/Window/sessionStorage).

Bearer в JS storage уязвим при XSS. До подключения Core нужны tested CSP без
неограниченных script sources/eval, безопасный text/Markdown/URL rendering,
отсутствие выполнения HTML из Task/log/Artifact, `frame-ancestors 'none'`,
`base-uri 'none'`, `object-src 'none'`, `nosniff` и отказ от сторонних scripts.
Plain-Vite live shell не содержит inline JS: применяется `script-src 'self'`
без nonce/hash exceptions или `unsafe-inline`. CSP проверяется на production
live build, а не на demo/HMR. [OWASP HTML5 Security](https://cheatsheetseries.owasp.org/cheatsheets/HTML5_Security_Cheat_Sheet.html).

## 5. Authority, sandbox и streaming

Core сохраняет actor, authorization, Project scope, named commands,
idempotency и expected revision. Browser не задаёт роль/actor через payload
или headers. Local-owner session не создаёт нового Human actor на каждый login:
она используется с существующей стабильной local-human authority Core. Receipt,
policy failure и revision conflict возвращаются без выдуманного успеха.

UDS, session/bootstrap material и assets с credentials не монтируются в
RunEnvironment. Provider/tool egress не позволяет Run обращаться к UI listener
или local host network. Перед первым Core connection это проверяется отдельно;
отсутствие mount само по себе не доказывает сетевую изоляцию. Процесс с правами
самого host owner и скомпрометированный browser extension вне этой sandbox
границы: loopback auth не защищает от полностью захваченной owner session.

Будущий SSE client использует fetch-stream, чтобы передавать Authorization
header; native EventSource с токеном в URL запрещён. Текущий Core выдаёт конечные
пачки: нужны cursor resume/dedup, bounded reconnect/backoff, request cancellation
при смене Project и offline/stale state. Event остаётся projection, не command.

## 6. Implementation gate FRONTEND-007

FRONTEND-007 реализует gateway/session и keyless Core read smoke. До её
закрытия нельзя подключать реальные экраны к UDS через временный открытый proxy.

Негативные тесты должны доказать rejection до Core effect для чужого Host/Origin
(включая `null`), отсутствующего/expired/revoked token, code replay/race,
недопустимого method/path, oversized body/response, неправильного UDS owner/mode
и sandbox ingress. Нужны lost-response/idempotency tests при добавлении commands,
а также реальный CSP/XSS smoke, credential scan assets/logs, logout copied-token
test и restart expiry. Static traversal tests FRONTEND-006 не заменяют их.

UI0.1/UI0.3 остаются открытыми. Пользователь согласовал ранний preparatory-срез
UI0.2 для ADR/static proof, затем отдельный login/Project read FRONTEND-007;
следом согласованы ранние Task read срез UI1.3/FRONTEND-008 и Run read срез
UI2.1/FRONTEND-009, затем Pipeline read срез UI1.2/FRONTEND-010. Полные dependencies и exit gates эпиков
сохраняются. Remote deployment, Tauri и installer остаются отдельными вехами.
