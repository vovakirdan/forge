# FRONTEND-008 — Настоящий список и карточка Task

**Epic:** [UI1.3](../../docs/epics/ui1-e3-task-board-management.md), ранний read-only срез;
[UI0.2](../../docs/epics/ui0-e2-browser-api.md), расширение gateway.
**Статус:** done
**Приоритет:** P0
**Дата:** 17 сентября 2026
**Baseline:** `ba920f7` — FRONTEND-007 зафиксирована отдельно, без push.
**Зависимости:** FRONTEND-002/005/007; ранний срез до полного закрытия UI0
согласован пользователем.

**Goal:** читать реальные задачи выбранного Project и их закреплённые стадии
в отдельном live UI, без изменения канонических данных.
**Approach:** ограниченные scoped GET через owner gateway, существующие
Task/Pipeline contracts и presentation, настоящий Core fixture без провайдера.
**Skills:** coding, rust-best-practices, react-best-practices,
typescript-best-practices, playwright-best-practices, subagent-task-execution.

## Контракт

1. Gateway разрешает только три новых GET:
   `/api/projects/{project_id}/tasks`, `/tasks/{task_id}` и
   `/pipelines/{pipeline_version_id}` внутри того же Project prefix.
   Новые IDs — UUIDv7. Только список принимает `limit` (1–100, default 20)
   и `cursor` (непустая opaque строка до 1024 UTF-8 bytes).
   Неизвестные/повторные параметры отклоняются; upstream query собирается заново.
2. Auth, exact Host/Origin, private owner UDS, отсутствие generic proxy,
   deadlines и concurrency bounds FRONTEND-007 сохраняются.
   Health/Project/list ограничены 64 KiB; Task detail/PipelineVersion — 1 MiB.
   Declared и streaming oversize дают безопасный 502/`response_too_large`,
   без обрезанного JSON. Core 409/`cursor_invalid` сохраняется как безопасный код.
3. Список показывает key, title, kind, lifecycle, raw stage ID, priority ID,
   updated_at. По 20 записей, Previous/Next/Refresh; без выдуманного total.
4. Карточка показывает Task fields, description, nullable Definition of Done,
   properties JSON, wait conditions и только ID/kind/title/date артефактов.
   Artifact body, ссылки и object refs автоматически не открываются.
   Inline body/metadata остаются частью канонического detail response и его
   общего лимита; запрет относится к отображению и загрузке ссылочных объектов.
   Любой пользовательский текст остаётся текстом, не HTML/исполняемым Markdown.
5. Pipeline читается только для выбранной карточки, по `pipeline_version_id`
   свежего detail, не старого summary. Старый pin остаётся валиден после новой
   default version и soft delete каталога. `no_stage` отличается от unavailable.
   Ошибка Pipeline не скрывает Task; list не порождает fan-out запросов Pipeline.
6. Loading/empty/not found/unavailable различаются. Неудачный refresh сохраняет
   явно помеченные stale данные. Invalid cursor предлагает первую страницу.
   Поздние ответы после смены Project/page/Task/session не восстанавливают
   старые данные; запросы отменяются. Ключи включают session generation и scope.
7. Cache ограничен текущими page/detail/pipeline: неактивные записи удаляются
   при навигации; история хранит только cursor strings. Project switch сбрасывает
   selection/navigation/cache, logout/expiry/401 — весь cache. Project/Task
   selection не сохраняется после reload. Клавиатура и узкий экран поддержаны.

## Порядок и проверки

- [x] Отдельный локальный commit FRONTEND-007; без push.
- [x] Gateway routes, query/response bounds, безопасные коды и Rust tests.
- [x] Live API/список/карточка, scoped cancellation и bounded cache; unit tests.
- [x] Named-command Core fixture: 21+ Tasks, другие/пустой Project, pin v1/v2,
  soft delete, свойства/ожидания/артефакты; без provider Runs.
- [x] Real browser proof списка/карточки/page20/Project scope/pinned stage/XSS;
  delayed real responses и отдельно обозначенные fault injections.
- [x] Offline/retry/stale data, cursor reset, 401/logout с pending reads,
  keyboard/narrow layout; прежние auth/CSP/secret/cleanup gates.
- [x] Последовательные frontend suites/builds/typecheck/lint, Rust fmt/clippy
  и целевые tests под MemoryMax=6G, MemorySwapMax=1G, CPUQuota=200%, Cargo jobs=2.
- [x] Независимое spec/security/quality review, исправление findings.
- [x] ADR/alignment/runbook/index и фактическое evidence.

## Evidence

Проверено 17 сентября 2026, Linux/rootless Podman, Bun 1.3.11, Node 24.14.0.
Тяжёлые проверки шли последовательно под `MemoryMax=6G`, `MemorySwapMax=1G`,
`CPUQuota=200%`, Cargo jobs ≤2. Provider credentials не читались, платные Runs
не запускались. Пользовательский Core/queues/repositories не использовались.

| Gate | Результат |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo test --locked -p forge-ui -p forge-cli -p forge-protocol --all-targets` | 78 PASS: 41 gateway, 25 protocol, 12 CLI, включая PTY tests |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | PASS |
| `bun run test:live-unit` | 29 PASS; в том числе 6 cache/QueryObserver regressions |
| `just ui-test-live` | PASS: live build, gateway/CLI/fixture, 2 egress policy tests, 1 physical sandbox test и browser suite |
| Live browser | 22 PASS: 7 прежних и 15 новых сценариев; повторный финальный прогон также 22 PASS |
| Intentional authenticated failure | Ровно 1 ожидаемая ошибка, 0 дополнительных; scan 50 issued code/token по stdout/artifacts/dist-live — PASS |
| Contracts / presentation | 47 / 18 PASS |
| Demo browser / static Node + browser | 7 / 2 + 13 PASS |
| `bun run typecheck` / `bun run lint` | PASS; 0 errors, прежние 10 fast-refresh warnings |
| Ordinary / static / live builds | PASS; live entry и demo остаются раздельными |
| Hygiene | `git diff --check`, shell syntax, ignore generated output, 182 локальные Markdown-ссылки — PASS |

### Что доказано настоящим Core

Fixture создаёт все данные named commands в private PostgreSQL schema с NATS.
Main Project содержит 23 Task: страницы 20 + 3. Вторая область содержит одну
свою analysis Task, третья пуста. Список включает waiting/ready/draft/cancelled;
Tasks без Git не получают искусственный workspace/assignee.

Featured Task имеет два stage-evidence артефакта, wait, описание и DoD. После
публикации default v2 и soft-delete каталога её v1 остаётся доступной, а карточка
показывает старое имя стадии. Межпроектные Task/Pipeline reads возвращают 404.
Кириллица и HTML-подобные title/description остаются текстом; URL/object_ref
в inline body не открываются и не появляются в карточке. Body/metadata при
этом входят в канонический TaskDetail response — UI не обещает их отсутствие
в browser memory. Draft сохраняет typed properties; настройка Project schema
не обходится прямой записью в БД.

Задержки реальных gateway responses проверяют отмену Task detail, Task list и
Pipeline read при смене selection/Project/logout. Отзыв session во второй
вкладке приводит к настоящему 401 и очистке UI. Offline refresh сохраняет
помеченные stale данные и выбранную карточку, retry возвращает реальные данные.
Проверены клавиатура, ширина 375px, отсутствие горизонтального overflow и
сброс selection при reload без потери действующей session.

### Что проверено искусственными отказами

Отдельно помеченные browser fault tests подставляют 503/404, oversized-response
error и cursor-invalid error, а также устаревший pin в summary. Они доказывают
UI recovery/сообщения, не поведение Core при этих отказах. Настоящий Core
отдельно подтвердил invalid cursor → 409; gateway unit tests проверяют finite
mapping, query/UUID validation, отсутствие browser authority headers,
границы response ровно 64 KiB/1 MiB, declared и chunked oversize для success
и error status. Raw upstream body не становится публичным сообщением.

Physical sandbox gate использует настоящий PodmanBackend/forge-runner с
synthetic executable, без провайдера; это не proof оплачиваемого execution.
Секреты тестовых owner sessions остаются в памяти/private control path. Trace,
HAR, screenshot/video и сохранённые auth fixtures выключены. Намеренный failure
не принимается по одному exit code: reporter проверяет его точную форму и
отсутствие дополнительных ошибок; diagnostics содержат только конечные категории.

### Findings и исправления

Независимое spec/security review нашло потерю выбранной Task при неуспешном
refresh того же Project и неточное утверждение о незагружаемых artifact bodies.
Оба замечания исправлены. Общий browser-прогон дополнительно выявил реальный
лишний запрос к предыдущему Project: удаление ещё наблюдаемой query до commit
React-перехода позволяло библиотеке создать её заново. Теперь запрос отменяется
сразу, а observed query удаляется после смены scope. QueryObserver regression
была RED до исправления и стала GREEN; browser отдельно требует отсутствие
запроса к прежнему Project. Аналогичный порядок применён к page navigation.

Повторное независимое review production и browser harness — без оставшихся
P0/P1/P2. Docs/scope/hygiene review также принято; уточнение «artifact metadata»
заменено конкретными ID/kind/title/date. Cache не полагается только на gcTime:
неактивные DTO удаляются, и поздний ответ не создаёт запись повторно.

Новые production endpoints/DTO Core, migrations и frontend dependencies не
добавлялись. Gateway явно использует уже присутствующий `percent-encoding 2.3.2`;
версии зависимостей не обновлялись. Ни один новый файл не превышает 500 даже
физических строк. Прежние demo Board/Team остаются mock; полный UI0/UI1 не закрыт.

Ручной запуск — в [live runbook](../../frontend/README.md#live-owner-ui-frontend-007).
FRONTEND-007 сохранена локально как `ba920f7`; FRONTEND-008 не закоммичена,
push не выполнялся.

После проверки остановлены только поднятые для этой работы PostgreSQL/NATS;
volumes и private test schemas сохранены для диагностики. `podman ps` пуст;
проверка `/proc` не нашла Core fixture, UI gateway или тестовый Chromium.
Повторный аварийный SIGTERM probe FRONTEND-007 в этой Task не запускался;
его process cleanup implementation не менялся, unit cleanup tests проходят.

## Вне Task

Mock Board/Team и demo services, mutating commands, drag-and-drop, SSE,
router/deep links, полный OpenAPI, новые Core DTO/endpoints/migrations,
provider Runs, remote control. UI0/UI1 целиком не закрываются.
FRONTEND-008 остаётся без commit и push до отдельного запроса.
