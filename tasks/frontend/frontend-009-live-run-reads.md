# FRONTEND-009 — Настоящий список и карточка Run

**Epic:** [UI2.1](../../docs/epics/ui2-e1-run-activity-evidence.md), ранний read-only срез;
[UI0.2](../../docs/epics/ui0-e2-browser-api.md), расширение gateway.
**Статус:** done
**Приоритет:** P0
**Дата:** 17 сентября 2026
**Baseline:** `a596ff6` — FRONTEND-008 опубликована в origin/main.
**Зависимости:** FRONTEND-003/005/008; ранний срез UI2.1 согласован пользователем.

**Goal:** читать реальные Runs выбранного Project, различая requested и observed
state, без управления исполнением и раскрытия технических bodies.
**Approach:** существующие Core Run reads, owner gateway, Run contracts и
presentation; отдельные Tasks/Runs в live entry, keyless acceptance.
**Skills:** coding, rust-best-practices, react-best-practices, react-dev,
typescript-best-practices, playwright-best-practices, subagent-task-execution.

## Контракт

1. Два новых GET: `/api/projects/{project_id}/runs?limit&cursor` и
   `/api/projects/{project_id}/runs/{run_id}`. UUIDv7; только список принимает
   строгие limit/cursor по правилам FRONTEND-008: default 20, диапазон 1–100,
   cursor до 1024 UTF-8 bytes. List cap 64 KiB, detail cap 1 MiB; oversized
   response — явная ошибка без частичного JSON, invalid cursor — безопасный 409.
2. Существующие auth, exact Host/Origin, private owner UDS, deadline/concurrency
   bounds сохраняются. Нет generic proxy, новых Core endpoints или миграций.
3. Переключатель Tasks/Runs внутри Project; default Tasks. Смена Project
   сбрасывает раздел. Смена раздела отменяет старые reads и удаляет cache после
   размонтирования; не сохраняет selection/cursor history. Reload также не
   сохраняет выбранный Project. Demo UI не меняется.
4. Список по 20, Previous/Next/Refresh, серверный порядок без выдуманного total.
   Показывает Run ID, purpose, nullable Task/Employee/stage ID, attempt и
   раздельные desired/observed states. Это Project-wide read, не история Task.
5. Карточка показывает typed Run facts и purpose-specific owner fields,
   fencing token, environment epoch, observation sequence и RunSpec version.
   Отсутствующие provider/model/timestamps/cost/duration/names не выдумываются.
6. Diagnostics показывает только наличие runtime report/handoff/usage/Git
   source, число загруженных incidents/evidence и stream completeness.
   Inline diagnostics входят в ответ и лимит detail, но bodies/raw JSON,
   URLs/object refs/paths не отображаются и не загружаются дополнительно.
7. Cache ограничен текущими page/detail и изолирован session generation,
   Project, cursor, Run. Поздние ответы не возвращают старую область. Loading,
   empty, not found, unavailable различаются; failed refresh сохраняет явно
   stale данные. Cursor-invalid предлагает первую страницу. Logout/401 очищает
   session cache. Только ручное обновление; без polling/SSE/commands.

## Проверки и порядок

- [x] Отдельный commit/push FRONTEND-008 и сверка remote HEAD.
- [x] Run gateway allowlist, bounds/query/error/security tests.
- [x] Live API, список/карточка, Tasks/Runs, scoped cleanup и unit tests.
- [x] Отдельные real Core fixture Projects: 23 + 1 Runs, пустой Project.
  Named commands и M0 fake runtime; без прямой записи в БД и платных моделей.
  Перед выдачей fixture дождаться также физического observed Stopped.
- [x] Real browser pagination/detail/isolation; synthetic fixtures всех пяти
  purposes, nullable owners, independent states и diagnostic availability.
- [x] Races/late replies, stale/retry, cursor reset, oversize, logout/401,
  keyboard/narrow layout, safe text и отсутствие body/link fetch.
- [x] Последовательные Rust/frontend/browser/static checks под MemoryMax=6G,
  MemorySwapMax=1G, CPUQuota=200%, Cargo jobs ≤2.
- [x] Независимое spec/security/quality review и исправление findings.
- [x] Документация, index и фактическое evidence.

## Evidence

Проверено 17 сентября 2026, Linux/rootless Podman, Bun 1.3.11, Node 24.14.0.
Тяжёлые проверки запускались последовательно с указанными выше ограничениями.
Provider credentials не читались; платные модели и пользовательский Core не
использовались. FRONTEND-008 опубликована как `a596ff6` в `origin/main`, вместе
с предыдущими локальными FRONTEND-006/007; remote HEAD сверён с полным SHA.

| Gate | Результат |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo test --locked -p forge-ui -p forge-cli -p forge-protocol --all-targets` | 80 PASS: 43 gateway, 12 CLI, 25 protocol |
| `cargo test --locked -p forge-core --lib` | 79 PASS, включая canary-тесты публичных Run projections и нормализованного runtime report |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | PASS |
| `bun run test:live-unit` | 36 PASS, включая 3 новых Run API и 4 cache/QueryObserver tests |
| `just ui-test-live` | PASS: live build, gateway/CLI/fixture, 2 egress tests, 1 physical sandbox test, browser и secret scan |
| Live browser | Два прогона по 35 PASS: прежние 22 и 13 новых Run-сценариев |
| Intentional authenticated failure | Ровно 1 ожидаемая ошибка, 0 дополнительных; scan 80 issued code/token по stdout/artifacts/dist-live — PASS |
| Contracts / presentation | 47 / 18 PASS |
| Demo browser / static Node + browser | 7 / 2 + 13 PASS |
| `bun run typecheck` / `bun run lint` | PASS; 0 errors, прежние 10 fast-refresh warnings |
| Ordinary / static / live builds | PASS; отдельные entry сохраняются |
| Hygiene | `git diff --check`, ignore generated output, 198 локальных Markdown-ссылок — PASS; изменённые code files не превышают 500 даже физических строк |

### Что проверено

Fixture создаёт отдельные Projects, Employees и Tasks через named commands в
private PostgreSQL schema с NATS. M0 fake runtime исполняет 23 + 1 Run; readiness
дожидается не только Task done, но и observed `stopped`. Реальные owner UDS/HTTP
reads подтверждают страницы 20 + 3, серверный порядок, detail, пустой Project,
foreign-Project 404 и безопасный cursor-invalid 409. Задержанные реальные ответы
используются для смены Run/Project/раздела, logout и серверного 401. Browser также
проверяет stale/retry, keyboard и 375px viewport.

Synthetic cases отдельно проверяют пять purposes, оба SystemJob kinds,
nullable Task/Employee/stage, `stop_requested` + `running` и
`force_stop_requested` + `unknown`. Diagnostic `{}` остаётся Available, `null` —
Unavailable; counts не объявляются доказательством успеха. HTML-текст stream
остаётся текстом; opaque body/URL/path/object-ref canaries не отображаются и
не вызывают сетевых запросов. Ошибки transport/oversize помечены как synthetic,
а фактические byte bounds проверены на UDS в Rust для declared/chunked responses.

Отсутствие private RunSpec/context/observation полей проверяется отдельно
canary-сериализацией Core, а не только пустой fake-диагностикой. Тест нормализации
runtime report доказывает отсутствие provider text/неизвестных auth-полей в
этом отчёте. Это не обещание универсальной очистки произвольного opaque JSON:
разрешённые inline diagnostics всё равно получает браузер.

Gateway allowlist regression сначала была RED (NotFound), затем GREEN после
добавления двух маршрутов. Для frontend unit tests отдельный RED-прогон до
реализации не выполнялся. Независимые Rust и frontend/security review приняты;
P2 исправлен: у Hook поля Task/stage подписаны как Context, поскольку владельцем
является invocation. Synthetic browser test закрепляет это отличие от TaskStage.
Оставшихся P0/P1/P2 в проверенном scope нет.

Новых Core endpoints, миграций и dependencies нет. Shared `Field` и
`ProjectReadScope` вынесены из Task UI без изменения его контракта. Полный
UI2.1 gate, mock Board/Team, commands/SSE и evidence/context viewer не закрыты.

Ручной запуск — в [live runbook](../../frontend/README.md#live-owner-ui-frontend-007).

После повторного browser-прогона остановлены только PostgreSQL/NATS, поднятые
для этой проверки. Volumes, private test schemas и безопасные diagnostics
сохранены; `podman ps` пуст. Проверка `/proc` не нашла Core fixture, UI gateway
или тестовый Chromium. На момент этой приёмки FRONTEND-009 оставалась локальной,
без commit/push. 17 сентября 2026 по отдельному запросу создан commit `3285f54`
(`feat(ui): add live run list and detail`); push не выполнялся.

## Ограничения

Core пока загружает все Runs Project перед пагинацией: bounded browser read
не доказывает масштабируемую серверную пагинацию. Diagnostics имеет count bounds,
но может превысить 1 MiB; UI сообщает об отказе, не обрезает содержимое.
All-purpose synthetic coverage не закрывает полный UI2.1 gate. Настоящие Core
reads с fake execution не являются provider/sandbox proof.
Отдельный запрос на публикацию commit `3285f54` получен 17 сентября 2026:
push выполняется вместе с FRONTEND-010, без изменения истории FRONTEND-009.
