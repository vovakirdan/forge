# Epic UI2.3 — Management, Resolution и Recovery

**Milestone:** UI2 — Наблюдение, коммуникация и вмешательство человека
**Статус:** planned; реализация не начата
**Источник:** [UI plan](../UI_IMPLEMENTATION_PLAN.md),
[backend alignment](../UI_BACKEND_ALIGNMENT.md)
**База backend:** `1439211`; Core сохраняет исключительное право изменять state.
**Зависимости:** UI0.1–UI0.3, UI2.1, UI2.2.

## Цель

Сделать существующие management и recovery решения доступными оператору, сохранив
точные scope, revisions и смысл остановки. Показать очередь escalations и
допустимые действия без превращения UI в второй scheduler или resolver.

## В границах

- Project start/stop, Task pause/resume и Employee stop через named commands.
- Next-Run Employee constraints и scheduled resume с просмотром их состояний.
- Resolver routes, очередь/detail escalations, allowed outcomes и human resolution.
- Reroute, communication retry, recovery assessment и Git recovery действия
  только там, где их допускают существующие контракты Core.
- Аудит причины, target, revision, command receipt и последующих observations.

## Что уже есть и каких чтений нет

Backend содержит `configure_resolver_route`, `raise_escalation`,
`submit_human_resolution`, `reroute_escalation`, Task/Employee/Project control,
`accept_run_recovery_assessment`, `retry_communication`, `retry_git_integration`
и `accept_git_integration_result`. Resolution имеет собственный assignment;
contextual Task или Communication остаётся источником вопроса.

Нужны scoped list/detail reads для escalations, routes, assignments, next-Run
constraints, resume schedules и recovery state, которого нет в текущем DTO.
Это проекции существующих моделей; формы не требуют новых произвольных переходов.
Опорный код: `crates/forge-application/src/command.rs`,
`crates/forge-domain/src/resolution/`, `crates/forge-core/src/recovery/`.

## Контракты и зависимости

- UI2.1 даёт Run identity и физическое состояние, UI2.2 — источник Communication.
- Перед действием UI получает текущую revision и показывает конкретный target,
  причину и последствия. Конкурентный отказ требует перечитать state.
- Logical gate/stop request и physical stopped отображаются раздельно; timeout,
  lease expiry и provider failure сами по себе не доказывают quiescence.
- Нет подразумеваемой универсальной команды `abort_run`: кнопка вызывает только
  существующий scoped command. Новая операция требует отдельного контракта.
- Human resolution ограничено canonical allowed outcomes и текущей generation;
  ответ resolver не получает прав выполнять другую Task.
- Recovery acceptance и retry различаются: принятие уже случившегося Git effect
  не повторяет merge. Старый fence/epoch нельзя использовать для новой попытки.

## Не в границах

Manager LLM, новые стратегии recovery, принудительная смена Pipeline stage,
автоматическое одобрение решений, отмена произвольных процессов, direct DB writes
или обход retained physical ownership ради более быстрого retry.

## Направления будущей декомпозиции

1. Уточнить management read projections и таблицу доступных command actions.
2. Подключить resolver queue/detail с routing, allowed outcomes и human response.
3. Добавить scoped stop/resume/constraint/schedule и recovery confirmation flows.
4. Проверить гонки revisions, stale assignments и физически незавершённый stop.

Это направления работ; отдельные TASK-ID и файлы задач создаются позднее.

## Exit gate и проверки

- Keyless fixtures покрывают Task и Communication escalation, reroute и ответ
  человека; UI не показывает недопустимый outcome как выполняемое действие.
- Двойной click и повтор после timeout не создают повторных canonical effects.
- Stop остаётся pending до observation; occupied Run не становится retryable
  только из-за elapsed time, отказа провайдера или закрытого browser tab.
- Stale revision/generation/fence даёт отказ и актуализацию экрана без обхода Core.
- Recovery UI различает retry и acceptance уже подтверждённого Git result.
- Browser/API tests проверяют подтверждения, причины, audit links и недоступный
  scope; тестовые сценарии не требуют платных provider вызовов.

## Риски

Слово «остановлен» может скрыть продолжающийся процесс. Ещё опаснее повторить
физический Git effect через обычный retry или принять historical assessment
за разрешение текущему Run. UI обязан сохранять эти различия.
