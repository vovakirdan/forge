# Epic EXT1.2 — Bounded planning wave

**Milestone:** EXT1 — Planning
**Статус:** Deferred extension; design gate
**Источник:** [UI implementation plan](../UI_IMPLEMENTATION_PLAN.md), [backend alignment](../UI_BACKEND_ALIGNMENT.md)

## Цель

Определить ограниченную planning wave: предложение ближайшего набора работы,
которое человек или System Manager рассматривает и явно утверждает. Wave не
является runtime queue, а предложение агента или LLM не меняет Task напрямую.

## Базовая граница

- PRD разрешает подробно планировать только ближайшую wave и хранить далёкую
  работу как краткие Epic notes.
- Task, Pipeline, priority и lifecycle принадлежат canonical Core commands.
- M0–M3 не предоставляют approved wave record, batch materialization или Lead
  authority для изменения Task.
- Goal/Epic остаются optional и требуют решения EXT1.1 до связывания с wave.

## Предлагаемое расширение

- Представить wave как bounded, versioned proposal с input scope, author,
  предложенными связями, estimate assumptions и сроком актуальности.
- Разделить proposal, human/Manager review и materialization в canonical Task.
- Разрешить только named, audited approval command создавать или менять
  утверждённые planning links и Task drafts в пределах policy.
- Сохранить исходную версию proposal, решение и причины отклонения для audit.

## Не в границах

- Прямой LLM write в Task, Goal, Epic, Pipeline или priority.
- Неограниченная генерация backlog, autonomous roadmap или mandatory Lead.
- Автоматический запуск созданных Task, обход approval и смена stage.
- Реализация планировщика, UI wizard или domain migration в этом epic.

## Зависимости

- EXT1.1 определяет optional Goal/Epic links и их audit semantics.
- Контракт UI2.2 определяет proposal/review presentation и требуемые read models.
- EXT1.2 не является prerequisite для UI0–UI4 или M4; до реализации нужен
  отдельный утверждённый design/API record.

## Исследовательские решения до task breakdown

- Что именно ограничивает wave: число Task, horizon, capacity, budget или
  явная policy проекта; какие лимиты являются advisory.
- Является ли wave самостоятельным record или artifact с typed metadata.
- Как proposal фиксирует input revisions и устаревает после Task/Pipeline/
  Goal/Epic изменений.
- Как approval materializes drafts, обновляет существующие links и исключает
  duplicate Task без неявного rebasing.
- Какие роли могут approve, reject, expire или reopen wave и какие события
  остаются immutable audit evidence.

## Будущая декомпозиция

После design approval возможны proposal model, policy evaluation, approval
command, audit/read projections и UI review flow; это не разрешение на реализацию.

## Exit gate

Утверждённый design record ограничивает scope и authority wave, описывает
staleness/materialization и доказывает, что агент не меняет Task без named
approval. Реализация начинается только после нового явного решения.

## Проверка

- Сверить flow proposal → review → approval/rejection с Core write boundary.
- Проверить сценарии stale proposal, partial approval, rejection и task-only
  проекта без Goal/Epic.
- Проверить, что UI2.2 не подразумевает autonomous backlog generation.

## Риски

Wave легко превращается в скрытый backlog generator или в обход Manager policy.
Также опасен stale proposal: без input revisions он может материализовать уже
неактуальные Task. Оба риска требуют bounded policy и audited approval.
