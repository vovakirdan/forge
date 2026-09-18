# FRONTEND-012 — Приоритеты проекта в live Task

**Статус:** done
**Epic:** UI0.1 / UI1.1 / UI1.3; gateway UI0.2
**Приоритет:** P0
**Зависимости:** FRONTEND-008/011; ограниченный read-срез согласован отдельно.

## Результат

Live список и карточка Task показывают название приоритета из canonical
PriorityScheme проекта, сохраняя исходный stable ID. Каталог читается отдельно
от Project control view; его отказ не блокирует чтение Task и draft editor.

## Контракт и границы

- Core: `GET /v1/projects/{project_id}/priority-scheme`; gateway:
  `GET /api/projects/{project_id}/priority-scheme`, без query parameters.
- DTO: `project_id`, `project_revision`, `default_level_id`, `levels` с
  `id`, `display_name`, `rank`, `retired`. Один canonical Project snapshot;
  отдельная revision схемы не вводится.
- ProjectView не меняется. Read-only accessor перечисляет уровни, включая
  retired, в порядке stable ID; это не порядок планирования.
- Gateway сохраняет owner/session/Host/Origin правила GET и лимит 64 KiB.
  Oversize — ошибка, не усечённый или пустой каталог.
- Frontend проверяет Project scope, safe revision, unique stable IDs,
  непустую схему, активный default и signed i32 ranks. Отрицательные и
  одинаковые ranks допустимы. Label — непустой текст до 128 Unicode scalars.
- Один shared query на раздел Tasks; name и ID видны в списке/карточке.
  Retired не заменяется default. Unknown ID, loading, failure и stale
  различаются; ручное обновление каталога независимо от Task reads.
- Кэш ограничен Project/session scopes, поздние ответы после ухода не
  показываются. Отдельные Project и catalog revisions не обязаны совпадать.

Не входят: создание Task, изменение приоритета, редактор схемы, property и
cancellation catalogs, Board, SSE, новые зависимости или миграции. Порядок
Task и цвета не выводятся из rank. Полные epic/milestone gates сохраняются.

## Проверка

- Unit/contracts: точный DTO, 1/3/10 уровней, retired, unknown ID, Unicode,
  negative/tied ranks, duplicate IDs, invalid default/revision/scope.
- Настоящий Core/browser: стандартная схема, два Project scopes, названия и
  ID в списке/карточке, refresh, отсутствие изменений state от GET.
- Явно synthetic browser responses: нестандартные схемы, retired/unknown
  references, malformed/oversize/unavailable, XSS, stale/late responses.
- Regression существующих reads и FRONTEND-011; boundary/secret/sandbox gates,
  Rust fmt/clippy/tests, frontend types/lint/unit/build/browser checks.
- Тяжёлые проверки последовательно, MemoryMax 6 GiB, SwapMax 1 GiB,
  CPUQuota 200%, Cargo jobs 2; независимые spec и quality reviews.

CreateProject сейчас принимает только имя и создаёт стандартные три уровня.
Команды настройки схемы нет. Custom fixtures не доказывают её наличие и не
создаются обходом Core через прямую запись в canonical БД.

## Evidence

Локальная приёмка завершена 18 сентября 2026:

- `cargo test --locked -p forge-domain -p forge-core -p forge-ui --lib`:
  257 tests passed (117 domain, 80 Core, 60 gateway).
- PostgreSQL `priority_scheme_http_is_scoped_complete_and_read_only`:
  passed; два Project, разные revisions, точный DTO и неизменный canonical
  snapshot после повторных GET и отказов. Runs и queue entries не созданы.
- `cargo fmt --all -- --check` и workspace clippy с
  `--all-targets --all-features --locked -- -D warnings`: passed.
- Frontend typecheck и lint: passed; 10 прежних react-refresh warnings,
  ошибок нет. Contracts: 55, live-unit: 55, presentation: 23 passed.
- `just ui-test-live`: exit 0; live build, 2 egress tests, физическая Podman
  sandbox-проверка и 73 browser tests passed. Secret scan: 180 issued values,
  утечек нет. Отдельная намеренно падающая probe: 1 expected / 0 unexpected;
  финальный scan 184 issued values, утечек нет.
- Demo/static builds: passed; static — 2 Node + 13 browser tests passed;
  demo — 7 browser tests passed. Сообщение serve exit 143 — штатный SIGTERM
  принадлежащего harness dev server после тестов; итоговый exit 0.
- Независимые spec/security и quality reviews: findings нет. Финальная
  проверка согласованности документации также без замечаний.
- `git diff --check` и 149 локальных Markdown links: passed. Поднятые для
  проверки PostgreSQL/NATS остановлены, volumes сохранены; тестовые
  Core/gateway/browser процессы не остались.

Реальный Core/browser proof проверяет стандартную трёхуровневую схему,
scoping и отсутствие побочных эффектов чтения. Custom 1/10-level, retired
и fault cases — отдельные unit/synthetic browser evidence, не доказательство
реализованного конфигуратора. Provider credentials и платные Runs не использованы.

FRONTEND-011 опубликована отдельно как `42e135f`; на момент приёмки изменения
FRONTEND-012 остаются локальными, без commit/push до отдельного запроса.
