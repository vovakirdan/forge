# FRONTEND-011 — Редактирование draft Task

**Статус:** done
**Epic:** UI0.2 / UI1.3
**Приоритет:** P0
**Зависимости:** FRONTEND-007–010; первый command-срез согласован отдельно.

## Результат

В существующей live-карточке Task можно изменить название и описание черновика
через Core `amend_draft`. Receipt, конфликт ревизии и неопределённый результат
сохранения различаются. Это не закрытие целого epic или milestone.

## Границы

- Только `title`/`description` существующей `draft` Task, changed-fields patch.
- Отдельный allowlisted `POST /api/commands/amend_draft`, не generic proxy.
- Core envelope, Project/Task revisions и `Idempotency-Key` без изменения
  существующего Core API, actor derivation или HTTP error contract.
- Запрос до 512 KiB, receipt до 64 KiB; прежние auth/read caps сохраняются.
- Обязательные exact Host/Origin/session перед Core connection; только
  необходимые server-built headers. Browser не задаёт actor.
- Форматирование текста сохраняется. Название не whitespace-only и не длиннее
  240 Unicode scalar values; описание допускает пустую строку, максимум 50 000.
- Успех только по валидному applied/replayed receipt для этой Task. После него
  перечитываются Project/Task/list; ошибка read не отменяет подтверждённый успех.
- Один Save фиксирует body/revisions/key. После потери ответа ручной retry
  использует ту же попытку. Нет автоматических повторов, offline-очереди,
  сохранения форм в storage или скрытого rebasing.
- Stale Project остаётся 409/stale_revision; stale Task — 400/invalid_request.
  После отказа ввод сохраняется, fresh baseline и новый Save требуют явных
  действий. Поздние ответы не пересекают Project/session scopes.
- При уходе предупреждение о потере локального состояния; abort HTTP не
  отменяет уже применённую команду Core.

Не входят: CreateTask, lifecycle/priority/properties/DoD edits, запуск работников,
SSE, Board wiring, новый Core API, миграции, зависимости и provider Runs.
Общая postcommit-dispatch механика Core не изменяется: no-Run proof относится
к stopped fixture без Employees/ready work, а не к любому открытому Project.

## Проверка

- Unit/contract tests: строгий wire, Unicode, изменённые поля, frozen retry,
  scoped callbacks, bounded transport и finite error mapping.
- Keyless browser + настоящий Core: изменение текста без lifecycle changes;
  потерянный applied response и replay с единственным effect; две вкладки;
  stale Task, foreign scope, non-draft и validation refusals.
- Негативные security POST probes, CSP/XSS, secret scan и sandbox ingress.
- Refresh failure после receipt, navigation/logout/late responses.
- Регрессии прежних reads, demo/static/live builds и tests, typecheck/lint,
  Rust tests/fmt/clippy; тяжёлые команды последовательно с resource limits.
- Независимые spec/quality reviews; findings устраняются до done.

## Evidence

Локальная приёмка 18 сентября 2026 года:

- Rust: `cargo fmt --all -- --check`, workspace clippy с `-D warnings`;
  `cargo test --locked -p forge-ui -p forge-cli -p forge-protocol` — 95 tests
  (58 UI, 12 CLI, 25 protocol), без failures.
- Frontend: typecheck; lint — 0 errors, 10 прежних react-refresh warnings;
  50 contract, 51 live-unit и 20 presentation tests проходят.
- Live, demo и static builds проходят; 2 static Node, 13 static browser и
  7 demo browser tests проходят. Импортированный demo не подключён к Core.
- Настоящий Core + browser: 61/61 tests проходят в двух последовательных
  прогонах после исправления тестового helper. Отдельный полный запуск
  `node frontend/scripts/run-live-tests.ts` завершился с exit 0, включая
  намеренно падающий failure probe (1 expected, 0 unexpected) и secret scan:
  152 issued values в основной suite, 156 после probe, без утечек.
- Физический rootless Podman sandbox ingress test и 2 Core egress tests
  проходят повторно. Fixture Project остановлен, без Employees/ready work;
  изменение draft не создаёт Run в этом fixture.
- Проверены реальный commit с потерянным ответом и replay без второго effect;
  конфликт двух вкладок; stale Task, foreign scope, non-draft, invalid patch;
  Host/Origin/session/body limits; malformed receipt; read failure после
  подтверждения; отмена навигации; logout с поздним ответом; Unicode/XSS.
- Первоначальный browser прогон выявил ошибку helper, не gateway: Node fetch
  заменял вручную заданный Host, поэтому negative probe отправлял допустимый
  запрос. Helper переведён на bounded `node:http.request`, negative probe
  повторно прошёл. Production security policy не ослаблялась.
- Независимые spec/security и quality reviews: findings не осталось;
  изменение raw HTTP helper дополнительно проверено reviewer.

Тяжёлые проверки выполнялись последовательно: MemoryMax 6 GiB, SwapMax 1 GiB,
CPUQuota 200%, Cargo jobs 2. Новые зависимости, миграции и Core API не добавлены;
платные provider Runs и пользовательские credentials не использовались.
Полные UI milestone/epic gates не закрыты. На момент этой приёмки изменения
были локальными; публикация выполняется отдельным шагом по запросу пользователя.
