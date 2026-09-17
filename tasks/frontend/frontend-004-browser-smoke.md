# FRONTEND-004 — Воспроизводимый browser smoke

**Epic:** [UI0.3](../../docs/epics/ui0-e3-frontend-tooling.md)
**Статус:** done — browser-smoke срез; UI0.3 остаётся in_progress
**Приоритет:** P0
**Дата:** 17 сентября 2026
**Зависимости:** [FRONTEND-001](frontend-001-local-demo-baseline.md) — done
**Baseline:** `d16898c`; FRONTEND-003 опубликована отдельно до начала этой Task.

## Цель и границы

Автоматизировать проверку существующего mock-only Control Room в Chromium.
Тесты проверяют навигацию, диалоги и клавиатуру без Core, credentials и provider
Runs. Synthetic demo остаётся demo: это не evidence интеграции с backend.

Источники: [UI plan](../../docs/UI_IMPLEMENTATION_PLAN.md),
[STACK](../../docs/STACK.md), [правила](../../docs/PROJECT_RULES.md),
[frontend README](../../frontend/README.md).

## Реализация

- Единственная новая прямая dev dependency — `@playwright/test` ровно `1.63.0`;
  единственный lockfile — `bun.lock`. Bun `1.3.11`, Node `24.14.0` сохраняются.
- `just ui-browser-install` явно устанавливает Chromium закреплённой версии
  Playwright; `just ui-test-browser` запускает suite. Системный Chrome и
  произвольный старый browser cache не служат fallback. OS packages не
  устанавливаются автоматически с повышенными правами.
- Отдельный Vite на `127.0.0.1:4173`, strict port, `reuseExistingServer: false`.
  Обычный demo на `5173` не меняется. Playwright завершает собственную группу
  процессов через SIGTERM с ограниченным ожиданием, затем SIGKILL.
- Один worker, без retries, свежий context для каждого теста. Test timeout —
  30 s, server startup — 120 s, global timeout — 240 s; внешний предел — 300 s.
- Role/label locators и web-first assertions, без фиксированных sleeps.
  `pageerror` и `console.error` фиксируются с начала каждого теста и роняют его.
- Trace сохраняется при failure, screenshot — только при failure; HTML report
  не открывается автоматически. Reports и test output исключены из Git/lint.
- Browser config/tests включены в typecheck. Прежние 47 contract tests
  остаются отдельной suite.
- Допустимы только точечные исправления доступного названия и фокуса диалогов,
  подтверждённые regression tests; визуальный язык и demo services сохраняются.

## Acceptance

1. Overview → Board → Team → Knowledge: URL, видимые заголовки и reload.
2. TASK-142 открывается с доски клавиатурой; Escape закрывает drawer и
   возвращает фокус на исходную карточку.
3. Full page открывает страницу задачи, reload сохраняет её; навигация
   не возвращает фокус на покидаемую доску.
4. Command palette: Ctrl+K, поиск страницы/задачи, Enter, пустой результат,
   Escape и возврат фокуса.
5. При `390×844` диалоги помещаются в viewport, поиск и закрытие доступны.
   Это не проверка полной мобильной адаптации доски/sidebar.
6. Frozen install и два последовательных browser runs проходят. Занятый
   порт приводит к ошибке, чужой процесс остаётся жив. Собственный сервер
   завершается после успеха и ошибки.
7. Contract tests, typecheck, lint и build проходят без новых warnings;
   generated output не меняет tracked source неожиданно.
8. Независимые spec/quality reviews не оставляют открытых findings.

## Проверки и evidence

Проверки выполняются по очереди через `systemd-run --user --scope --quiet`
с `MemoryMax=6G`, `MemorySwapMax=1G`, `CPUQuota=200%` и `timeout 300s`.
Не запускать Rust builds и второй browser/dev процесс одновременно.

Проверено 17 сентября 2026: Linux, Bun `1.3.11`, Node `24.14.0`,
Playwright `1.63.0`, закреплённый Chromium `153.0.8010.12` (revision `1243`).
Системные библиотеки/браузер не устанавливались через sudo. Browser installer
проверил уже доступную закреплённую revision; старый произвольный browser path
не использовался.

| Проверка | Результат |
|---|---|
| `just ui-install` | PASS: frozen install, 419 installs across 479 packages, no changes |
| `just ui-browser-install` | PASS: Chromium закреплённой версии доступен |
| `just ui-test-browser`, root run 1 | PASS: 7/7, 34.1 s, без retries |
| `just ui-test-browser`, root run 2 | PASS: 7/7, 34.5 s, без retries |
| Занятый порт 4173 | PASS: запуск отклонён; контрольный HTTP server остался жив и вернул исходный ответ |
| Injected `console.error` / `pageerror` | PASS: обе негативные проверки завершились ожидаемым failure, collector сохранил сообщения |
| Зависший тест, global timeout 15 s | PASS: deadline сработал после начала теста, owned server завершён |
| `just ui-test-contracts` | PASS: 47/47, около 1.215 s |
| `just ui-typecheck` | PASS, включая browser config/tests |
| `just ui-lint` | PASS: 0 errors, те же 10 react-refresh warnings |
| `just ui-build` | PASS: client/SSR/Nitro cloudflare-module, прежний hosting target |
| Diff/Justfile formatting/ignore audit | PASS; browser reports, node_modules и build output ignored |
| Local Markdown links | PASS: 100 файлов назначения в изменённых документах существуют |
| Независимые spec/quality reviews | APPROVE, открытых findings по реализации нет |

Все проверки root выполнялись последовательно с лимитами выше. Port 4173
свободен после успешного запуска, injected failures и global timeout.
Обычный demo на 5173 не запускался и не использовался тестами. Backend, Core
services, Rust validation, credentials и provider Runs в проверках не участвовали.

SHA-256 нового `bun.lock` до/после frozen install и проверок:
`8b6a18718bad0635936d65b8ce39ba06a211be2595d3e7323cfb74a982aa0b6d`.
Lockfile получил только Playwright trio; существующие зависимости не обновлялись.
Tracked route manifest и другие generated sources не изменились.

### Regression evidence и ограничения

- Первый прогон на неизменённом UI: 5 failed / 2 passed. Тесты обнаружили
  отсутствие доступного названия command palette и потерю фокуса drawer.
  После первых исправлений: 6 passed / 1 failed — отдельно выявлена потеря
  фокуса palette. Добавлены скрытые заголовок/описания, label поля поиска и
  возврат фокуса при обычном закрытии; при навигации возврат подавляется.
- В первоначальном navigation smoke клик по SSR-разметке до готовности
  клиентских queries вызвал TanStack hydration errors. Ожидание загруженного
  названия проекта дало PASS на неизменённом UI; затем оно добавлено после
  каждого goto/reload. Это исправление синхронизации теста, не доказательство
  исправления раннего пользовательского клика. Router/framework не менялись;
  ранние SSR-взаимодействия остаются для проверки в оставшемся UI0.3.
- Первые red reports/traces сохранены вне repository:
  `/tmp/forge-frontend004-red.SmUUef/`. Root operational probes и logs:
  `/tmp/forge-frontend004-probes.nJSlpS/`. Это временная диагностика, не
  обязательные файлы для обычного повторного запуска suite.
- Негативные operational probes используют отдельный временный config/spec,
  импортирующий настоящий error collector и server config. Они не добавлены
  в обычные семь smoke tests, не изменяют UI и ожидаемо завершаются ненулевым кодом.
  Первоначальное ожидание текста timeout в probe было уточнено по фактическому
  сообщению Playwright; итоговый полный probe прошёл.
- Vite сохраняет прежнее предупреждение `vite-tsconfig-paths`. В окружении
  проверки Node также сообщает о совместных `NO_COLOR`/`FORCE_COLOR`.
  Bun может напечатать exit 143 для server script при штатном SIGTERM cleanup;
  итоговая успешная suite при этом возвращает 0. Browser errors не подавляются.

## Не входит и что остаётся

Component-unit runner, CI, screenshot golden baselines, live API/auth/SSE,
project-scoped query refactor, backend unavailable/offline/permission states,
полная мобильная адаптация и изменение PRD/доменов — отдельные задачи.
UI0.3 остаётся `in_progress` после завершения этого browser-smoke среза.
