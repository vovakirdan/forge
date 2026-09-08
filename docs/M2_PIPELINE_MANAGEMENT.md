# M2: Pipeline management

Pipeline — изменяемая запись каталога. PipelineVersion — полный неизменяемый
граф: опубликованную версию нельзя изменить или удалить. `revision` каталога
и номер графа различаются; `latest_version` сохраняет последний опубликованный
номер даже после возврата default к более ранней версии.

## Команды

Каждая команда требует Project `expected_revision`, idempotency key,
`pipeline_id` и `expected_pipeline_revision`.

| Команда | Дополнительный payload | Эффект |
| --- | --- | --- |
| `publish_pipeline_version` | полный `definition`, `make_default` optional, default `false` | публикует следующий номер; при явном флаге выбирает его для новых Task |
| `set_pipeline_default_version` | `pipeline_version_id` | выбирает уже опубликованную версию этого Pipeline |
| `delete_pipeline` | нет | выставляет `deleted_at`; сохраняет графы и существующие Task |

`definition` содержит `task_kinds`, `entry_stage_id`, optional
`max_stage_visits`, `stages`, `transitions` — без имени каталога. Выполняются те
же проверки полноты, достижимости, outcomes и bounded cycles, что при создании
Pipeline. Новая версия, catalog revision, Project revision, audit, outbox и
receipt фиксируются одной транзакцией. Replay возвращает первоначальный результат.

Удалённый Pipeline нельзя менять или назначать новой Task. Повторная команда
удаления с новым idempotency key отклоняется; replay прежнего удаления допустим.
Старые Task, включая созданные до удаления drafts, продолжают работать с прежним
графом. Автоматической миграции Task нет.

## Выбор версии и stage contract

`create_task` принимает ровно один selector:

- `pipeline_version_id` явно выбирает неизменяемую версию;
- `pipeline_id` выбирает current default под той же транзакционной блокировкой.

Выбранная версия сохраняется сразу. Изменение default не переназначает ни draft,
ни исполняемую Task.

Stage содержит `instructions` и optional `workspace`:

```json
{
  "instructions": "Проверь изменения принятой ревизии и верни замечания",
  "workspace": { "kind": "git", "access": "read_only" }
}
```

`kind` — `any`, `filesystem` или `git`; `access` — `read_only` или `read_write`.
Это требования, не выдача прав и не выбор host path. Core проверяет совместимость
эффективного runtime binding до admission. Stage вместе с инструкциями входит
в зафиксированный контекст Run. Исторические версии без этих полей получают
пустые инструкции и отсутствие дополнительных требований — поведение M1.

HTTP Pipeline views возвращают immutable stage contract и текущие catalog
`catalog_revision`, `default_version_id`, `latest_version`, `deleted_at`.
Видимость каталога не меняет pinned graph.

## Проверки и границы среза

Domain и command conformance проверяют publication/default/delete, exact CAS,
rollback, replay, legacy defaults, immutable history и продолжение старой Task
после удаления Pipeline. Общие сценарии выполняются на memory и PostgreSQL.
Workspace compatibility и HTTP view имеют отдельные тесты.

Этот срез не добавляет hooks, independent acceptance, Task migration, новые
исполнители System stage или автоматический выбор другого Employee при
несовместимой назначенной среде. Эти возможности остаются отдельными частями M2.
