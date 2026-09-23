# FRONTEND-019 — Выбор Project в live UI

**Статус:** done
**Epic:** UI1.1 / UI0.2
**Приоритет:** P0
**Зависимости:** FRONTEND-007, FRONTEND-018

## Результат

Owner видит страницу существующих Projects после входа и выбирает Project по
имени без ручного ввода UUID. Core выдаёт только safe control projection:
`id`, `name`, `revision`, `execution_gate`. Список и выбранная карточка приходят
из Core, без fallback к mock UI. Team, Project settings и создание Project
в этот срез не входят.

## Контракт

- `GET /v1/projects?limit=20&cursor=…` использует ID-порядок и существующую
  семантику list cursor; финальная страница не содержит `next_cursor`.
- Gateway разрешает только `GET /api/projects` после owner session/Host/Origin
  проверок, с лимитом ответа 64 KiB и ограниченным query.
- При переключении действует существующий leave guard; запросы предыдущего
  Project отменяются и их cache освобождается. Выбор не переживает reload/logout.
- Ошибки, пустой список и недействительный cursor отображаются явно; ручное
  обновление не создаёт Project и не обещает управление его execution gate.

## Приёмка

Проверить пустую/полную/вторую страницу, stale cursor, запрещённые методы и
query, защиту owner session, выбранный Project, две независимые области данных,
неотправленную команду при смене Project, клавиатуру и узкий экран. Выполнить
Rust/frontend tests, typecheck, lint, live build, browser/security gate и
`git diff --check`. Keyless Core fixture не является provider proof.

## Evidence

- `just ui-test-live`: 148/148 browser tests PASS. Включены реальный список
  более 20 Projects, выбор со второй страницы, возврат к другому Project,
  stale cursor, пустая synthetic страница, клавиатура и узкий экран.
- В том же gate прошли Core egress (2/2), sandbox boundary (1/1), скан
  секретов (494 значения без утечек) и ожидаемый failure probe (498 значений).
- `cargo test -p forge-ui --lib --locked`: 83 PASS;
  `cargo test -p forge-core --lib --locked`: 81 PASS. Frontend live unit,
  contract tests, typecheck, lint (0 ошибок, 10 прежних предупреждений),
  live build, Rust clippy/format и `git diff --check` прошли.
- Проверен локальный keyless Core fixture. Provider Run и внешний стенд
  не проверялись.
