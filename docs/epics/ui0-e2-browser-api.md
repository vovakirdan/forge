# Epic UI0.2 — Локальная browser boundary и API client

**Milestone:** UI0 — Модель интерфейса и локальная API-граница
**Статус:** planned; hosting/security decision перед implementation tasks
**Тип / приоритет:** integration / P0
**Источники:** [UI-план](../UI_IMPLEMENTATION_PLAN.md),
[матрица соответствия](../UI_BACKEND_ALIGNMENT.md)

## Цель

Дать browser безопасный доступ к существующему Core без переноса authority в UI
и без открытия operator Unix socket произвольным сайтам или сети.

## Исходная точка и design gate

Core обслуживает HTTP/JSON через UDS `0600`; browser не умеет обращаться к нему
напрямую. Импорт использует TanStack Start/Nitro и Lovable build wrapper.
До coding tasks фиксируется hosting ADR: остаётся ли локальный SSR host или
frontend собирается как static client. Выбор проверяется минимальным build и
health smoke, учитывает результат UI0.3 и описывается в STACK/ARCHITECTURE.
Это отдельный ограниченный design шаг, не скрытый выбор исполнителя.

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

**Зависимости:** UI0.1 и UI0.3. Toolchain preflight UI0.3 служит входом в
hosting ADR; финальная адаптация packaging и hosting proof входят в UI0.2.
Существующий CLI over UDS остаётся совместимым. Browser actor определяется
trusted server boundary, а не полем запроса. Валидация прав остаётся в Core.
Hosting/session ADR и conformance tests — обязательный выход до feature wiring.

## Направления будущей нарезки

1. Hosting/security ADR и local authentication handshake prototype.
2. OpenAPI completeness и typed read/command client.
3. Browser transport/session enforcement и health smoke.
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
