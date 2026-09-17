# FRONTEND-007 — Local-owner gateway и первое чтение Core

**Epic:** [UI0.2](../../docs/epics/ui0-e2-browser-api.md)
**Статус:** done
**Приоритет:** P0
**Дата:** 17 сентября 2026
**Baseline:** `2c42c16` — FRONTEND-006 зафиксирована отдельно, без push
**Зависимости:** FRONTEND-001–006; ранний live-read срез до полного UI0.1/UI0.3
согласован пользователем.

**Goal:** защищённый owner login и чтение настоящего Project в небольшом browser UI.
**Approach:** отдельный Rust gateway обслуживает static React entry и только
allowlisted Core GET по owner-local UDS. Canonical authority остаётся в Core.
**Skills:** coding, rust-best-practices, react-best-practices,
typescript-best-practices, playwright-best-practices, subagent-task-execution.
**Tech Details:** Rust/Axum/Hyper, React/Vite/TanStack Query, owner UDS,
Playwright; testkit с private PostgreSQL schema и NATS, без provider Runs.

## Контракт реализации

1. `forge-ui serve --assets-dir … --core-socket … --control-socket …` слушает
   только `127.0.0.1:0`. CLI `forge-cli ui login --control-socket …` получает код по
   отдельному owner-only control UDS и показывает его только controlling TTY.
   Код не передаётся через URL, env, argv, redirected stdout или daemon logs.
2. Код — CSPRNG 256 бит, 5 минут, один pending code, максимум 5 неверных попыток;
   повторная выдача инвалидирует старый, exchange атомарный. Opaque session —
   CSPRNG 256 бит, absolute TTL 8 часов, максимум 16 активных sessions, volatile
   storage без refresh; restart инвалидирует sessions.
3. HTTP: `POST /api/auth/exchange` (`{code}` → `{token, expires_at}` RFC3339),
   `POST /api/auth/logout` (204), authenticated `GET /api/health` и
   `GET /api/projects/{uuid}`. Exact Host везде, exact non-null Origin для POST;
   присутствующий Origin у GET также проверяется. API/auth используют no-store.
4. Gateway не имеет DB/CoreService/provider/shell зависимости. Core UDS path
   задаёт owner config; проверяются type/owner/mode и UID peer. Никакого TCP
   fallback, redirects, generic proxy, переноса browser actor/Authorization.
5. Request/control body ≤1 KiB, headers ≤16 KiB, Core response ≤64 KiB;
   connect deadline 1 s, весь request с body/upstream ≤5 s; до 32 connections
   и 8 API requests, bounded idle/control. Ошибки явные и без raw secret bodies.
6. Live entry не импортирует mock root/layout/services. `just ui-build-live`
   создаёт отдельный `frontend/dist-live` и manifest разрешённых файлов с hashes.
   Gateway держит immutable snapshot: ≤8 MiB/file, ≤32 MiB total. Старый demo
   и `dist/client` static proof остаются отдельными и работоспособными.
7. CSP без inline/eval/third-party scripts; только собственные assets/connects,
   запрет base/object/framing; nosniff, no-referrer. Project name — plain text.
8. UI: password code input, connection state, Project ID и настоящие
   id/name/revision/execution_gate через ProjectViewSchema. Token только в
   sessionStorage; storage failure без fallback. Logout/401/expiry очищают
   session, запросы и cache. Network error не выдаёт ложного logout/успеха.
   Запоздавшие ответы старой session/Project не показываются.

## Порядок работы и приёмка

- Реализовать gateway/auth, control/CLI и live entry отдельными ответственностями.
- Покрыть security negatives, bounded transport, assets, session races и cleanup
  unit/integration tests до полного browser smoke.
- Проверить реальный sandbox profile без провайдера: UI listener/Core/control
  socket и секреты недоступны. Mock UDS не заменяет эту проверку.
- Реальный testkit создаёт Project через named command в private schema;
  browser проходит login → gateway → Core Project GET. Health не заменяет это.
- Проверить XSS/CSP, offline/401/storage failure/stale responses/copied logout.
  Trace/HAR/video/screenshots и storage snapshots выключены. Намеренный failure
  должен пройти scan всех artifacts/stdout по конкретным issued code/token.
- Последовательно выполнить старые frontend suites/builds/typecheck/lint,
  Rust fmt/clippy и целевые tests. Limits: MemoryMax=6G, MemorySwapMax=1G,
  CPUQuota=200%, CARGO_BUILD_JOBS=2. Не запускать Rust и frontend builds вместе.
- Независимые spec/security/quality reviews; исправить findings и повторить gates.
- Обновить ADR/архитектуру/стек/runbook/index с фактическим evidence.

## Evidence

Проверено 17 сентября 2026 в этой рабочей копии, Linux/rootless Podman,
Bun 1.3.11, Node 24.14.0. Сборки и browser suites выполнялись последовательно
под `MemoryMax=6G`, `MemorySwapMax=1G`, `CPUQuota=200%`; Cargo — два jobs.

| Gate | Результат |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | PASS |
| `cargo test --locked -p forge-ui -p forge-cli -p forge-protocol --all-targets` | 66 PASS: 29 gateway, 25 protocol, 12 CLI, включая 3 реальных PTY/redirect tests |
| `cd frontend && bun run test:live-unit` | 20 PASS: API/session races, manifest/config, failure classification и остановка процессов |
| `just ui-test-live` | PASS: live build, Rust binaries/fixture, 2 egress policy tests, 1 physical sandbox test и browser acceptance |
| Real Core browser suite | 7 PASS; повторный проход также PASS. Owner CLI → exchange → health → настоящий Project с revision 2 и execution gate open |
| Intentional authenticated failure | Ровно 1 ожидаемая ошибка, 0 дополнительных; scan 16 выданных code/token по stdout/artifacts/dist-live — без утечек |
| `bun run typecheck` / `bun run lint` | PASS; 0 lint errors, прежние 10 fast-refresh warnings сохранены |
| Contracts / presentation | 47 / 18 PASS |
| Demo browser / static Node + browser | 7 / 2 + 13 PASS |
| `bun run build` / `bun run build:static` | PASS; отдельный dist-live сохранён |
| Shell syntax, diff, ignore, docs | `bash -n` двух launchers, `git diff --check`, ignore build/reports/secrets и локальные ссылки — PASS |

Реальный Core fixture создаёт Project через named commands в собственной
PostgreSQL schema и использует NATS. UI проверяет именно данные Project, не
только health; HTML-подобное имя остаётся текстом, injected inline script
блокируется production CSP. Проверены unknown Project без mock fallback,
offline/retry, sessionStorage denial, logout с копией токена в другой вкладке,
offline logout и отмена задержанного настоящего ответа предыдущего Project.
Старую session generation и expiry отдельно покрывают pure tests.

Physical sandbox gate использует настоящий `PodmanBackend`/`forge-runner` и
synthetic executable, без оплачиваемого провайдера. Проверены недоступность
живого host loopback listener через local/host aliases и отсутствие owner
Core/control sockets и synthetic session files в Run. Проверки egress policy
отдельно запрещают private/local destinations. Это не проверка всех возможных
сетевых конфигураций или защита от захваченного host-owner процесса.

Отдельный SIGTERM probe дождался одновременного появления Core fixture,
gateway и Chromium, остановил parent live runner и получил exit 1 (не ложный
PASS). До выхода внешнего cgroup `/proc` подтвердил отсутствие всех трёх
процессов. Unit test также проверяет остановку TERM-resistant потомка после
выхода его родителя. Process-group cleanup не объявляется защитой от любого
отделившегося процесса; для аварийного ограничения suite используется внешний
systemd scope. Force-kill всей машины не тестировался.

Намеренный failure не превращается в общий PASS по одному exit code: отдельный
reporter требует ровно ожидаемую ошибку и отклоняет дополнительные fixture/
cleanup ошибки. Токены не сохраняются как auth fixtures; trace/HAR/screenshots/
video выключены. Обычный demo harness не используется для real-owner login.

В процессе исправлены: приём лишних полей control command, завершение control
connections до освобождения lifetime lock, offline pause Query, неверный root
тестового fixture, гонка проверки logout и возможность маскировать дополнительную
ошибку ожидаемым failure. Независимые production security/spec и protocol/CLI
reviews — без оставшихся findings; замечания к тестовому harness исправлены
и покрыты повторными проверками. Ни один новый файл не превышает даже 500
физических строк. Frontend dependencies/lockfile и generated routes не изменены.

Ручной запуск описан в [frontend README](../../frontend/README.md#live-owner-ui-frontend-007).
Для этой приёмки provider credentials не читались, платные Runs не запускались.
PostgreSQL test schemas сохраняются для диагностики; пользовательские данные
и старые Run-контейнеры не удалялись. Поднятые для проверки PostgreSQL/NATS
остановлены после приёмки, volumes сохранены; активных тестовых процессов
и контейнеров после cleanup нет.

## Вне Task

Board/Team/Task feature wiring, management commands, SSE, provider runs,
remote control, installer/Tauri и миграции БД. UI0.1/UI0.2/UI0.3 целиком не
закрываются. Новый commit FRONTEND-007 и push требуют отдельного запроса.
