# ADR: локальная browser boundary

**Дата:** 17 сентября 2026
**Статус:** target принят; FRONTEND-006 проверяет только static hosting
**Область:** local-owner Control Room; remote control и Tauri отложены

## 1. Решение и уровень доказательства

Frontend поставляется как static React/TanStack client. Будущий тонкий
Rust/Axum gateway отдаёт assets и связывает browser с закрытым Core UDS:

`Browser → loopback Rust gateway → owner-local Core HTTP/JSON UDS`

В [FRONTEND-006](../tasks/frontend/frontend-006-static-hosting-boundary.md)
реализован только build/test path статического demo. Rust gateway, bootstrap,
sessions, CSP и Core reads ещё не реализованы. Node file server — тестовый
инструмент без доступа к Core, не production boundary и не auth prototype.
Успешный static smoke не доказывает безопасность будущего gateway.

| Вариант | Решение |
|---|---|
| Static client + отдельный Rust gateway | Принят: нет Node runtime в установленном продукте, доменная authority остаётся в Core |
| TanStack/Nitro SSR host | Сохраняется для прежнего demo/build workflow, но не выбран для установленного Forge |
| Management router Core на TCP | Отклонён: owner-local API нельзя открывать browser без новой auth boundary |

Static assets позже можно использовать в Tauri. WebView, IPC, desktop permissions
и packaging требуют отдельного решения; текущий ADR не обещает готовую desktop
интеграцию и не вводит её зависимости.

## 2. Build и runtime contract

- `just ui-build-static` использует отдельную Vite-конфигурацию с TanStack SPA
  mode, `nitro: false` и отключённым автоматическим внедрением environment.
  Версии, lockfile и обычный `just ui-build` не меняются.
- Runtime artifact — только `frontend/dist/client`: SPA shell, JS/CSS и public
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
  API errors будущего gateway тоже не должны скрываться за SPA fallback.

Build и smoke выполняются последовательно с ограничениями памяти. `vite preview`
не служит static proof: установленный Start preview plugin загружает server
bundle. Обычный demo на 5173 и прежний browser suite на 4173 остаются отдельными.

## 3. Будущий Rust gateway

Gateway — отдельный native adapter, не новый домен и не второй Core. Он получает
только static assets, свой session state и фиксированный owner-local UDS path.
Не получает DB, NATS, provider keys, container socket, `CoreService` или право
запускать CLI/shell. Не импортирует management router Core в TCP listener.

Первый transport-срез разрешает только authenticated read: health и один
существующий Project по известному ID. Health показывает доступность транспорта,
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
  Любой Core read, включая health, требует действующей session. Host/Origin и
  Fetch Metadata — защита browser boundary, но не аутентификация. Native local
  процесс умеет подделать headers; authority даёт possession секрета.
- Bootstrap/mutation принимают только ожидаемый JSON content type и schema;
  cross-origin preflight не получает разрешающих CORS headers. Проверки не
  полагаются на один только браузерный CORS. [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html).
- UDS path задаёт trusted owner config, не request. Проверяются type/owner/mode
  socket и parent directory; ошибка приводит к отказу без TCP fallback. Клиент
  соединяется только с этим endpoint, не следует redirects.
- Каждый route имеет request/response byte limits, connection/read deadlines
  и bounded concurrency. Ошибка Core даёт явный unavailable/timeout, не mock
  response и не бесконечный retry. Точные численные лимиты фиксирует следующая
  implementation Task вместе с boundary tests.
- Secrets, Authorization, bootstrap body и raw upstream errors не попадают в
  logs/traces; auth и API responses используют `Cache-Control: no-store`.
  Service worker не кэширует authenticated content.

## 4. Owner bootstrap и session — target contract

1. Gateway сначала успешно bind-ит listener и фиксирует origin. После этого
   генерируется single-use CSPRNG код с 256 битами энтропии и TTL **5 минут**.
   Код показывается только в owner terminal; пользователь вставляет его в UI.
   Он не попадает в URL/history, argv, env, provider context, telemetry или logs.
2. Browser отправляет код JSON POST на same-origin exchange. Gateway проверяет
   Host/Origin, размер/формат, TTL и атомарно consumes код. При двух одновременных
   exchange ровно один может выдать session. Invalid/expired/replayed код не
   раскрывается ответом; число попыток и concurrency ограничены.
3. Только явная owner CLI операция может выдать новый код; старый инвалидируется.
   Неаутентифицированного HTTP mint/reset endpoint нет. Интерактивный terminal
   вывод отделён от daemon journal. При отсутствии controlling owner TTY
   выдача отказывает безопасно: код не печатается в redirected stdout/journal.
   Доставка кода CLI после запуска service —
   часть следующего local-owner implementation contract.
4. Успех возвращает новый opaque CSPRNG bearer token (256 бит) и absolute expiry
   **8 часов**. Сервер держит volatile session state; restart инвалидирует все
   sessions. Токен привязан к экземпляру gateway/owner authority, не является
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
Нужные build-time hashes для shell scripts проверяются на реальном static build;
попутный `unsafe-inline` для scripts не служит решением. Это обязательный gate,
а не утверждение о текущем demo. [OWASP HTML5 Security](https://cheatsheetseries.owasp.org/cheatsheets/HTML5_Security_Cheat_Sheet.html).

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

## 6. Следующий implementation gate

Отдельная Task реализует gateway/session и keyless Core read smoke. До её
закрытия нельзя подключать реальные экраны к UDS через временный открытый proxy.

Негативные тесты должны доказать rejection до Core effect для чужого Host/Origin
(включая `null`), отсутствующего/expired/revoked token, code replay/race,
недопустимого method/path, oversized body/response, неправильного UDS owner/mode
и sandbox ingress. Нужны lost-response/idempotency tests при добавлении commands,
а также реальный CSP/XSS smoke, credential scan assets/logs, logout copied-token
test и restart expiry. Static traversal tests FRONTEND-006 не заменяют их.

UI0.1/UI0.3 остаются открытыми. Пользователь согласовал ранний preparatory-срез
UI0.2 только для этого ADR/static proof; полные dependencies и exit gates эпиков
сохраняются. Remote deployment, Tauri и installer остаются отдельными вехами.
