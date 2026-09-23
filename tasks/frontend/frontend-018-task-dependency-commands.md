# FRONTEND-018 — Создание и удаление зависимостей Task

**Статус:** done
**Epic:** UI1.3 / UI0.2
**Приоритет:** P0
**Зависимости:** FRONTEND-017

## Результат и границы

В live Task-карточке owner выбирает другую Task того же Project из существующего
постраничного списка и создаёт `task_done` связь в направлении «Зависит от» или
«Блокирует». Существующую связь можно удалить после отдельного подтверждения.
Оба действия проходят через именованные Core commands; UI не вычисляет готовность
Task или право на dispatch.

Поиск по ключу, граф, stage/artifact conditions, редактирование mock Board и
новые Core commands/API не входят. Список кандидатов не загружает весь Project:
20 Task на страницу, без нового server-side поиска.

## Контракт

- Gateway открывает только `POST /api/commands/create_dependency` и
  `POST /api/commands/remove_dependency` после прежних session/Origin/Host
  проверок. Он отправляет в Core точные `/v1/commands/...` маршруты и исходные
  байты с `Idempotency-Key`; actor из browser не принимается.
- Envelope: `project_id`, текущая `expected_revision`, `payload` с
  `blocker_task_id` и `blocked_task_id`; create требует
  `required_condition: "task_done"`, remove не принимает condition.
- Receipt связан с `resource.kind: "task_dependency"` и ID зависимой Task.
  Изменение может увеличить Project revision более чем на один шаг из-за
  зависимых waits. Для уже существующей/отсутствующей связи Core может выдать
  no-op receipt с пустым `event_ids` и неизменной revision.
- При неизвестном результате browser предлагает повтор тех же body/key.
  Известный отказ требует обновить Project baseline и нового подтверждения.
  Receipt инвалидирует обе стороны зависимостей, Project, Task list и details;
  сам receipt не доказывает, что Run запущен или остановлен.

## Приёмка

1. Проверить контракт gateway: формы payload, self-link, scoped IDs, fixed paths,
   receipt change/no-op/replay, bounds, auth и отсутствие browser authority.
2. Проверить browser с keyless Core: выбор Task за первой страницей, создание и
   удаление, направления, отказ Core, неизвестный результат и exact retry,
   обновление зависимостей, смену Project/session, клавиатуру и узкий экран.
3. Последовательно выполнить Rust/frontend tests, typecheck, lint, build,
   security/browser gate и `git diff --check`. Фактические результаты записать
   ниже; keyless сценарий не является provider proof.

## Evidence

- `just ui-test-live`: 145/145 browser tests PASS, включая три сценария
  создания/удаления связей, выбор за первой страницей, exact retry,
  переходы между Project, клавиатуру и узкий экран. В том же gate прошли
  sandbox boundary (1/1), Core egress (2/2), secret scan и ожидаемый
  failure probe.
- `bun run test:live-unit`: 87 PASS; `bun run test:contracts`: 65 PASS;
  `cargo test -p forge-ui --lib --locked`: 82 PASS.
- `bun run build:live`, `bun run typecheck`, `bun run lint` (0 ошибок, 10 ранее существовавших
  предупреждений), `cargo clippy -p forge-ui --all-targets --all-features
  --locked -- -D warnings`, `cargo fmt --all -- --check` и `git diff --check`
  прошли.
- Browser gate использовал локальный keyless Core; исполнения provider Run и
  работы на внешнем стенде он не подтверждает. Изменения пока не закоммичены.
