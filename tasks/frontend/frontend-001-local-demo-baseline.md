# FRONTEND-001 — Воспроизводимый локальный запуск UI

**Epic:** [UI0.3](../../docs/epics/ui0-e3-frontend-tooling.md)
**Статус:** done — local demo baseline; UI0.3 остаётся in_progress
**Приоритет:** P0
**Дата:** 17 сентября 2026

## Цель и контекст

Дать разработчику повторяемую установку, сборку и локальный запуск импортированного
Control Room без Lovable account, Core, секретов и provider Runs. Исходный
frontend в `c2dbca2` использует mock services и ещё не согласован с моделью Core.
Baseline `f10f584` содержит UI-roadmap; build/typecheck тогда не были проверены.

Это первый шаг UI0.3, не закрытие всего эпика и не готовый операторский интерфейс.
Он даёт проверяемую среду для UI0.1 и входные сведения для hosting-решения UI0.2.

## Источники

- [UI plan](../../docs/UI_IMPLEMENTATION_PLAN.md): UI0, зависимости и evidence.
- [Alignment](../../docs/UI_BACKEND_ALIGNMENT.md): mock baseline, отсутствие live API.
- [PRD](../../forge-prd-v0.1.md): локальный Control Room до installer M4.
- [STACK](../../docs/STACK.md), [ARCHITECTURE](../../docs/ARCHITECTURE.md):
  frontend hosting ещё не выбран; Core сохраняет authority.
- [PROJECT_RULES](../../docs/PROJECT_RULES.md): минимальные изменения, секреты,
  truthful evidence, безопасная проверка и отсутствие неожиданных generated diffs.

## Scope

- Сохранить текущие TanStack/Vite, Lovable wrapper и `bun.lock`; проверить
  frozen install без ручного изменения дерева зависимостей.
- Зафиксировать Bun `1.3.11`, Node `24.14.0`, добавить `typecheck`.
- Добавить root entrypoints `just ui-install`, `ui-dev`, `ui-build`,
  `ui-typecheck`, `ui-lint`.
- Dev: только `127.0.0.1:5173`, ошибка при занятом порте вместо смены адреса.
- Исправить только конкретные блокеры установки/сборки/запуска.
- Документировать команды, demo-only смысл, artifact target и границы проверки.
- Зафиксировать оставшиеся type/lint findings без ослабления правил.

## Non-goals

Backend API, credentials, auth, model/fixtures redesign, production hosting,
новый framework, массовое обновление зависимостей/форматирование, полноценный
browser-test harness или CI, Rust validation и оплачиваемые provider Runs.

## Dependencies и выход

Task dependencies отсутствуют. Нужны исходный frontend, Linux, user systemd
для ограниченных проверок, Bun/Node указанных версий и доступ к package registry.
FRONTEND-001 не зависит от UI0.1 и не принимает hosting ADR за UI0.2.
Она снимает неопределённость baseline для оставшегося UI0.3 и подключения UI.

## Реализация и области изменений

`frontend/package.json`, runtime version files, `Justfile`, README; Vite/ignore
config только при доказанной необходимости. Новые launcher-frameworks не нужны.
Перед сменой framework или массовым dependency upgrade работа приостанавливается
для отдельного решения. Не переписывать опубликованную Git history.

Проверки выполняются последовательно в отдельном systemd scope:
`MemoryMax=6G`, `MemorySwapMax=1G`, `CPUQuota=200%`. Превышение лимита — failed
check, не повод снять лимит. Нельзя параллельно запускать Rust-сборку или несколько
browser sessions. Для smoke допускается один браузер вместе с dev-сервером.

## Acceptance

- Чистая и повторная frozen install проходят; `bun.lock` не изменяется.
- Build завершается; документирован фактический target полученного артефакта.
- Overview, Board, Task detail, Team, Knowledge открываются в браузере;
  работают навигация и reload без Core или provider credentials.
- Dev слушает только утверждённый loopback-адрес; второй запуск на занятом
  порте завершается ошибкой и не переходит на 5174.
- Typecheck/lint запускаются; не мешающие demo baseline findings записаны
  отдельно и не объявлены исправленными. Это не полный quality gate UI0.3.
- Generated output, dependencies, logs не попадают в Git; tracked source
  не переписывается неожиданно при build/dev.
- Независимое review не оставляет неисправленных findings в изменённом scope.

## Проверка и evidence

Проверено 17 сентября 2026 на Linux, Bun `1.3.11`, Node `24.14.0`.
Install/build/typecheck/lint выполнялись последовательно в ограниченных scopes,
без Core, базы данных, provider credentials и inference.

| Проверка | Результат |
|---|---|
| Чистый `bun install --frozen-lockfile` | PASS; установлено 413 packages |
| Повторная frozen install | PASS; no changes |
| `just ui-build` | PASS; client + SSR + Nitro cloudflare-module |
| `just ui-typecheck` | PASS; `tsc --noEmit`, exit 0 |
| `just ui-lint` | PASS; 0 errors, 10 warnings |
| Dev listener | Только `127.0.0.1:5173`; дополнительных listeners Vite нет |
| Второй `just ui-dev` | Exit 1, `Port 5173 is already in use`; 5174 не открыт |
| Browser navigation/reload | PASS; матрица ниже |
| Source/lock preservation | `frontend/src`, Vite/TS/ESLint config и `bun.lock` без изменений |
| Ignore/diff hygiene | node_modules/.output/.wrangler ignored; `git diff --check` PASS |
| Независимые reviews | Spec compliance и quality/safety: findings не найдены |

SHA-256 `bun.lock` до и после:
`fc00072f27cf820a6b27ab8c9de583f0c508ff16aaa40ddb403271cc65991935`.
Build создаёт `.output/public` и `.output/server/index.mjs` с `wrangler.json`;
это Cloudflare-module artifact, не доказательство Node/Forge production hosting.

Browser smoke выполнен через Playwright CLI с установленным Chromium
`145.0.7632.6`, в отдельном временном profile и ограниченном scope.
Проверялись видимые заголовки и содержимое после загрузки, не только HTTP status.

| Экран | Переход | Reload |
|---|---|---|
| Overview `/` | Sidebar → `Control Room` | PASS |
| Board `/board` | Sidebar → `Engineering Board` | PASS |
| Task drawer | Карточка TASK-142 → загруженный dialog | Не отдельный маршрут |
| Task `/tasks/TASK-142` | Drawer → `Full page` | PASS |
| Team `/team` | Sidebar → `Team` | PASS |
| Knowledge `/knowledge` | Sidebar → `Knowledge` | PASS |

Console после smoke: 0 errors, 0 warnings; page-error listener: 0.
Временные browser/dev scopes остановлены; PID завершены, порты 5173/5174 свободны.
Snapshots/console logs сохранены вне репозитория в
`/tmp/forge-frontend001-browser.3aNAak/`; это временные диагностические файлы,
которые не требуются для повторного запуска по [README](../../frontend/README.md).

### Известные ограничения

- 10 прежних `react-refresh/only-export-components` warnings: `Bits.tsx`
  (3), `project-context.tsx`, `badge.tsx`, `button.tsx`, `form.tsx`,
  `navigation-menu.tsx`, `sidebar.tsx`, `toggle.tsx`. Ни правило, ни source
  не менялись. Разбор export boundaries остаётся в static/component checks UI0.3.
- Build warnings о `vite-tsconfig-paths` и ignored `inlineDynamicImports`
  не блокируют сборку; wrapper/plugin cleanup не выполнялся в этой Task.
- Первый browser launch не нашёл системный Chrome; Firefox cache тоже не
  стартовал. Использован уже установленный Chromium через временный config;
  системные браузеры и зависимости проекта не устанавливались заново.
- Один предварительный smoke locator ожидал неполный exact title и дал timeout.
  Исправлен только проверочный locator; финальная матрица выше прошла без правок UI.
- Это agent-driven browser smoke, не новый persistent test suite. Harness/CI,
  full UI0.3 gate и live Core/provider integration не проверены и не закрыты.
- Rust validation не запускалась: backend не менялся, scope задачи её исключает.

## Делегирование и риски

Один исполнитель владеет config/entrypoints, отдельный reviewer проверяет scope
и evidence. Browser smoke запускается после освобождения install/build процесса.
README с командами не доказывает работоспособность; Cloudflare-target artifact
не доказывает готовность локального production host. Runtime/type/lint findings
остаются видимыми; mock UI не выдаётся за интеграцию с Forge Core.
