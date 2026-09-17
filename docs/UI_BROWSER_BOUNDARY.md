# ADR: локальная browser boundary

**Дата:** 17 сентября 2026
**Статус:** target принят; FRONTEND-007 — owner gateway, FRONTEND-008 — Task reads
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
React entry с TanStack Query и общими styles/primitives. Для login/Project/Task
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
Project scope. Health показывает доступность транспорта,
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
  имеет пустое тело и требует session/Origin. Будущие JSON commands потребуют
  отдельного schema/content-type contract;
  cross-origin preflight не получает разрешающих CORS headers. Проверки не
  полагаются на один только браузерный CORS. [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html).
- UDS path задаёт trusted owner config, не request. Проверяются type/owner/mode
  socket и parent directory; ошибка приводит к отказу без TCP fallback. Клиент
  соединяется только с этим endpoint, не следует redirects.
- Каждый route имеет request/response byte limits, connection/read deadlines
  и bounded concurrency. Auth/control JSON ≤1 KiB; headers ≤16 KiB; Core body
  ≤64 KiB для health/Project/Task list; Task detail/PipelineVersion ≤1 MiB,
  включая chunked/error responses. Connect ≤1 s, целый HTTP/control
  exchange ≤5 s, включая idle/body/upstream; до 32 HTTP connections,
  8 API requests и 4 control connections, без неограниченной очереди.
  Ошибка Core даёт явный unavailable/timeout, не mock response или retry loop.
- Secrets, Authorization, bootstrap body и raw upstream errors не попадают в
  logs/traces; auth и API responses используют `Cache-Control: no-store`.
  Service worker не кэширует authenticated content.

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
следом согласован ранний Task read срез UI1.3/FRONTEND-008.
полные dependencies и exit gates эпиков
сохраняются. Remote deployment, Tauri и installer остаются отдельными вехами.
