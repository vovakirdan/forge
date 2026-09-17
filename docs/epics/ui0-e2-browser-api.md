# Epic UI0.2 — Локальная browser boundary и API client

**Milestone:** UI0 — Модель интерфейса и локальная API-граница
**Статус:** in_progress — static proof и owner gateway/Project read срез; полная приёмка эпика впереди
**Тип / приоритет:** integration / P0
**Источники:** [UI-план](../UI_IMPLEMENTATION_PLAN.md),
[матрица соответствия](../UI_BACKEND_ALIGNMENT.md)

## Цель

Дать browser безопасный доступ к существующему Core без переноса authority в UI
и без открытия operator Unix socket произвольным сайтам или сети.

## Исходная точка и design gate

Core обслуживает HTTP/JSON через UDS `0600`; browser не умеет обращаться к нему
напрямую. [FRONTEND-006](../../tasks/frontend/frontend-006-static-hosting-boundary.md)
фиксирует [ADR](../UI_BROWSER_BOUNDARY.md): static client и отдельный Rust/Axum
gateway. Historical static demo использует Lovable wrapper и build-time SSR
prerender. FRONTEND-007 добавляет отдельный plain React/Vite live entry без SSR;
в runtime gateway не требуется Node/Nitro.

FRONTEND-006 проверяет static build через независимый file server, а не через
SSR preview. Это ограниченный preparatory-срез, не закрытие epic или
доказательство безопасного browser ingress.

Static-срез проверен 2 Node и 13 browser tests, прежними 7 dev-browser tests,
47 contract и 18 presentation tests, typecheck/lint/обеими сборками и
независимыми spec/quality reviews. Evidence и negative probes — в FRONTEND-006.

[FRONTEND-007](../../tasks/frontend/frontend-007-live-owner-gateway.md) реализует
`forge-ui`: exact loopback origin, private owner control UDS/TTY login, opaque
sessions, CSP, bounded assets/transport и allowlisted Core reads. Отдельный
`frontend/dist-live` показывает health и Project по ID через Zod contract,
без mock layout/services. Commands, SSE и остальные экраны сюда не входят.
Результаты unit/browser/Core/sandbox приёмки фиксируются в Task; наличие кода
или historical static tests не означает, что эти gates уже пройдены.

[FRONTEND-008](../../tasks/frontend/frontend-008-live-task-reads.md) расширяет
allowlist scoped Task list/detail и PipelineVersion reads. Новый read-only экран
использует существующие DTO; response bounds, cursor recovery, scope isolation
и поздние ответы проверяются отдельно. Commands/SSE по-прежнему не подключены.

Независимо от hosting, boundary имеет browser-facing loopback origin и доступ
к owner-local Core API; она не получает собственный scheduler или DB-write слой.

## В границах

- Local-owner bootstrap/session contract, exact Host/Origin checks, защита
  от CSRF/DNS rebinding, expiry/logout и запрет доступа из sandbox Run.
  Проверка `localhost` сама по себе не считается аутентификацией.
- HTTP/JSON и SSE, authenticated same-origin browser reads/commands;
  небезопасный wildcard CORS/public bind не является default.
- Полное OpenAPI описание существующего M0–M3 surface, включая knowledge,
  memory и SystemJob. Недостающие feature reads добавляются в своих эпиках.
- Typed client: project scope, validated responses, pagination/cursor,
  idempotency/revision/conflict, request cancellation и error states.
- SSE конечными пачками: cursor resume, dedup, bounded reconnect/backoff,
  явное stale/offline состояние. Событие обновляет projection, не запускает command.
- Session/bootstrap secret не попадает в browser URL/history/logs или provider
  context. Server credentials не выдаются UI; proxy не исполняет shell/file paths.

## Не в границах

Remote control, accounts/RBAC, OAuth service, WebSocket, API gateway platform,
отдельная БД frontend, generic filesystem proxy и массовое расширение domain API.

## Контракты и зависимости

**Зависимости:** UI0.1 и UI0.3. Пользователь разрешил FRONTEND-006 (ADR/static
proof), затем FRONTEND-007 (owner login/Project read) до их полного закрытия на
основе FRONTEND-001–005. Следом согласован FRONTEND-008 — Task read срез UI1.3
поверх этой границы. Это исключение для ограниченных срезов; остальные
gates сохраняются. Финальное packaging и полный typed transport входят в UI0.2.
Существующий CLI over UDS остаётся совместимым. Browser actor определяется
trusted server boundary, а не полем запроса. Валидация прав остаётся в Core.
Hosting/session ADR и conformance tests — обязательный выход до feature wiring.

## Направления будущей нарезки

1. Hosting/security ADR и static proof — FRONTEND-006; owner gateway,
   authentication и keyless health/Project read acceptance — FRONTEND-007.
2. OpenAPI completeness и typed read/command client.
3. Расширение защищённого transport на feature routes с policy и contract tests.
4. Scoped live updates, error/replay/conflict integration tests.

## Exit gate и проверка

- Посторонний origin/Host, отсутствующая/просроченная session и запрос из Run
  отвергаются до management effect; rejected attempts не раскрывают secrets.
- Потерянный command response и retry дают один receipt/effect; revision
  conflict не вызывает скрытого применения новой команды.
- Две browser tabs/Projects не смешивают scopes; cursor reconnect показывает
  каждое событие один раз и обрабатывает unavailable без busy loop.
- Реальный local health/read smoke проходит, CLI UDS tests не регрессируют.
- Проверено, что published client assets, logs и ошибки не содержат credentials.

## Риски

Browser trust boundary расширяет attack surface. Security design и negative
tests нельзя отложить до UI4. SSR adapter не должен обходить те же Core commands.
