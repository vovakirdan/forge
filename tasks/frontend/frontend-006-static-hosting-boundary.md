# FRONTEND-006 — Static hosting proof и browser boundary ADR

**Epic:** [UI0.2](../../docs/epics/ui0-e2-browser-api.md)
**Статус:** done — только ADR/static proof; полный UI0.2 остаётся открытым
**Приоритет:** P0
**Дата:** 17 сентября 2026
**Зависимости:** FRONTEND-001–005; preparatory-срез до полных UI0.1/UI0.3
согласован пользователем
**Baseline:** `96fee98`; FRONTEND-005 опубликована отдельно

## Цель и границы

Проверить, что существующий React/TanStack UI работает из статических assets
без SSR runtime, и зафиксировать будущую безопасную browser → Core boundary.
Node/Bun допустимы при сборке и тестировании, но не обязательны в установленном
Forge. Gateway/auth сейчас не реализуются, demo services остаются mock-only.

Источники: [ADR](../../docs/UI_BROWSER_BOUNDARY.md), [стек](../../docs/STACK.md),
[архитектура](../../docs/ARCHITECTURE.md), [UI plan](../../docs/UI_IMPLEMENTATION_PLAN.md),
[правила](../../docs/PROJECT_RULES.md), [runbook](../../frontend/README.md).

## Согласованный результат

1. Отдельный static Vite target использует установленный Lovable wrapper и
   TanStack SPA, выключает Nitro и автоматический environment export.
   `just ui-build-static` выдаёт runtime artifact `frontend/dist/client`.
   Build-time server bundle для shell prerender не попадает в runtime.
2. `just ui-test-static` запускает собственный Node built-in file server на
   `127.0.0.1:4174`, обслуживающий только client output. Нет SSR import,
   Vite preview, proxy, backend access или reuse существующего сервера.
3. Playwright повторно использует семь demo-сценариев и проверяет прямые URLs,
   reload, missing assets, traversal/root escape и отказ SPA fallback для
   `/api`, `/v1`, `/_serverFn`. Занятый порт и timeout не оставляют процессы.
4. ADR разделяет принятый target и фактический proof: отдельный Rust gateway,
   owner terminal code, explicit bearer session, Core authority и security
   negatives до первого Core connection. Tauri/remote/installer отложены.
5. Прежние demo/dev/browser/build commands, dependency versions и lockfile не
   меняются. Сбой static target не обходится SSR fallback или upgrade framework.
6. Документы не закрывают UI0.1/UI0.3 или весь UI0.2. Preparatory-исключение из
   порядка эпиков явно отражено в roadmap/index.

## Проверка

Все тяжёлые команды выполняются по очереди с wrapper:

```sh
systemd-run --user --scope --quiet \
  -p MemoryMax=6G -p MemorySwapMax=1G -p CPUQuota=200% \
  timeout 300s just ui-build-static
```

Последовательно: static build/smoke, 18 presentation и 47 contract tests,
прежний browser suite, typecheck, lint и обычный build. Проверить diff/ignore,
отсутствие изменений generated routes и lockfile. Независимые spec/quality
reviews обязательны; найденные дефекты исправляются с повторной проверкой.
Rust builds, Core/services и provider Runs не нужны и не запускаются.

## Evidence

Проверено 17 сентября 2026. Все проверки запускались последовательно с
указанными выше ограничениями; paid inference и Core services не использовались.

| Проверка | Результат |
|---|---|
| `just ui-build-static` | PASS: два прогона исполнителя и отдельный root run; `dist/client/_shell.html`, client assets; SSR только build-time |
| `just ui-test-static` | PASS: итоговый root run 2/2 Node tests (1.451 s) + 13/13 browser tests (26.3 s); перед ним отдельный прогон исполнителя 25.3 s |
| `just ui-test-presentation` | PASS: 18/18, 477 ms |
| `just ui-test-contracts` | PASS: 47/47, 1.194 s |
| `just ui-typecheck` | PASS после минимального исправления test-only narrowing |
| `just ui-lint` | PASS: 0 errors, прежние 10 react-refresh warnings |
| `just ui-test-browser` | PASS: прежние 7/7 на Vite, 34.4 s |
| `just ui-build` | PASS: прежний client/SSR/Nitro cloudflare-module target |
| Diff/Justfile formatting/ignore audit | PASS; assets, server bundles и browser reports игнорируются |
| Независимые spec/quality reviews | APPROVE; открытых findings нет |

13 static browser tests — это прежние 7 без копирования сценариев и ещё 6:
четыре direct-entry/reload для Board, Team, Knowledge и Task; запрет HTML fallback
для missing assets/API namespaces; raw traversal до URL normalization. Node
test дополнительно проверяет external symlink и prefix-sibling escape на
synthetic files, methods/content types и явный UI route allowlist.

Второй Node test разрешает реальную Vite config с synthetic `VITE_*` marker и
проверяет отсутствие автоматического env export, пустой envPrefix и loopback
prerender. Исполнитель также собрал client с двумя synthetic markers и проверил
40 HTML/JS/CSS/JSON файлов: значения не найдены. Это не blanket secret scan и
не обещание, что build tools не читают `.env`; соответствующий предел указан
в README/ADR.

Операционные negative probes исполнителя:

- При занятом 4174 Playwright завершился с exit 1 (`already used`); собственный
  sentinel listener продолжал отвечать. Чужие процессы не останавливались.
- Временный намеренно падающий test завершился с exit 1, после teardown порт
  4174 был свободен.
- Временный hanging test с global timeout 5000 ms завершился с exit 1; порт
  освободился. Были сообщения timeout suite и teardown, поэтому доказано
  освобождение порта, а не graceful завершение каждого handler.
- Временный `cleanup-probe.spec.ts` удалён перед итоговыми обычными прогонами.
  Negative traces/screenshots остались только в локальном
  `/tmp/forge-static-negative.G18f85KV/`, не в Git.

Первый root typecheck нашёл две ошибки одного test-only выражения: TypeScript
терял narrowing `server.address()` в nested callback. Исправление сохраняет
проверенный `port` в const до создания callback; casts и ослабления compiler
options не потребовались. Повторные typecheck/static tests прошли.

Предварительное ADR review уточнило ephemeral port будущего gateway и отказ
выдачи bootstrap code без owner TTY. Реализации auth это не добавляет.
Код после исправления отдельно одобрен quality reviewer; dependency source
audit подтвердил SPA shell/output и отдельно выявил необходимость loopback
bind для временного prerender preview.

`bun.lock` не изменился; SHA-256:
`8b6a18718bad0635936d65b8ce39ba06a211be2595d3e7323cfb74a982aa0b6d`.
Generated routes, весь `frontend/src`, прежние Vite/Playwright configs, backend
и PRD не менялись. Существующие warnings `vite-tsconfig-paths`, NO_COLOR и
SIGTERM exit 143 dev-server остаются видимыми. Ранний SSR-click риск из
FRONTEND-004 не объявляется исправленным или покрытым новым hosting smoke.

## Осталось за пределами Task

Реализация Rust gateway, session/bootstrap/CSP, keyless Core health/Project read,
typed HTTP/SSE client, complete API contract и feature wiring. Static smoke
проверяет только demo из client assets, не live Core и не защищённый ingress.
Component harness, CI и остальные UI0.1/UI0.3 gates остаются открытыми.
Новый коммит FRONTEND-006 требует отдельного запроса пользователя.
