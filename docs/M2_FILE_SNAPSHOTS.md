# M2: файловые снимки и входы Task

Файловый snapshot — неизменяемый Artifact с выбранными файлами. Для него не
нужен Git commit. Это способ передать документ, скрипт, картинку или незавершённую
работу другой задаче, не открывая ей чужую рабочую поверхность.

Snapshot не является GitCandidate, результатом review или разрешением Integration.
Он не меняет lifecycle и не подменяет обязательные артефакты Pipeline.

## Команды оператора

Команды доступны Human/System Manager через обычный command envelope:
`project_id`, актуальная Project `expected_revision`, `payload` и уникальный
`Idempotency-Key`. Employee не может выдавать эти management-команды.

| Команда | Payload помимо `task_id` и `expected_task_revision` | Результат |
| --- | --- | --- |
| `import_task_file_snapshot` | `title`, абсолютный `root`, явный список `paths` | Pending import из каталога оператора |
| `capture_task_file_snapshot` | `title`, явный список `paths` | Pending capture сохранённой writer surface |
| `attach_task_file_input` | `artifact_id` | Вход будущих Runs другой Task того же Project |

Пример payload импорта:

```json
{
  "task_id": "SOURCE_TASK_UUIDV7",
  "expected_task_revision": 1,
  "title": "Migration notes and sample input",
  "root": "/home/operator/selected-inputs",
  "paths": ["notes.md", "data/sample.bin"]
}
```

Пути относительны к `root`, а для capture — к worktree Task. Capture не принимает
произвольный host path: Core определяет последнюю writer surface и передаёт
Supervisor точный Run/fence/epoch. Он не останавливает Run сам. Сначала оператор
явно останавливает Task и дожидается физической quiescence; одной `waiting`
недостаточно. При активном Run или незавершённой Integration capture отклоняется.

Существующее правило неизменности закрытой Task сохраняется: новые snapshots и
входы нельзя прикрепить к `done`/`cancelled`. Capture нужен до закрытия задачи.
Уже запечатанные артефакты закрытой Task по-прежнему можно использовать как входы
другой незакрытой Task. Для отдельных файлов вне runtime возможен import в новую
Task; прямой import из Forge execution directory запрещён.

## Фиксация и восстановление

1. Core атомарно фиксирует pending intent, audit, outbox и command receipt.
   Доступ к файлам начинается после commit. Повтор с тем же idempotency key
   возвращает ту же операцию.
2. Для Task surface pending intent резервирует её: новый writer и Integration
   ждут. Supervisor проверяет журнал и физическую остановку, затем читает только
   выбранные файлы. Захват не сбрасывает HEAD, index или dirty files.
3. Байты и descriptor сохраняются в приватном staging. Завершённый staging
   повторно используется после перезапуска, без перечитывания изменённого
   источника. Частичный staging не считается успешным результатом.
4. Core сохраняет тела через существующий `object_store`/MinIO. Ключ содержит
   Project, Artifact и SHA-256. Запись create-only; повторная запись проверяет
   размер и digest. Бинарные и пустые файлы не проходят redacting EvidenceSpool.
5. Core одной транзакцией создаёт Artifact `file_snapshot`, прикрепляет его к Task,
   публикует `artifact_created`, `task_artifact_attached` и статус `sealed`.
   Task revision увеличивается от прикрепления Artifact, lifecycle не меняется.

Операция имеет `pending → sealed` или `pending → failed`. Ошибка хранилища
оставляет staging и pending operation для повторной доставки. Reservation
освобождается только после `sealed` либо подтверждённого завершения всех чтений
с ошибкой. Остановка async worker не позволяет новому Supervisor обогнать ещё
работающий blocking reader. Неподтверждённое состояние остаётся pending, не
превращается в предполагаемый успех по таймеру.

Каждый проход ограничен 32 операциями. Циклический курсор продвигается до их
обработки: зависший Supervisor или ошибочный receipt не вытесняют навсегда более
поздние операции, в том числе независимые локальные imports.

Если Task закрыли во время импорта, Artifact не прикрепляется; операция получает
`task_closed_before_capture_completed`. Capture не запрещает явную отмену Task
и не превращает остановленную работу в выполненную.

Проверка состояния:

```text
GET /v1/projects/P/tasks/T/file-snapshots?limit=50
GET /v1/projects/P/tasks/T/file-inputs
```

Первый ответ содержит `id`, зарезервированный `artifact_id`, `title`, `state`,
`error_code`, `manifest`, `created_at` и курсор следующей страницы. До `sealed`
Artifact ещё не существует. Ответы и audit не раскрывают import root или
внутренние Supervisor requests. Manifest содержит относительный путь, размер,
executable bit, SHA-256 и scoped object key каждого файла.

## Доставка Employee

`attach_task_file_input` принимает только запечатанный snapshot другой Task
того же Project. Прикрепление меняет Project revision и audit, но не Task
revision, текущий proposal или уже созданный RunSpec. Новые Task Runs v6
фиксируют полный список входов; старые Runs не получают их задним числом.

Core материализует проверенные тела, Supervisor повторно проверяет manifest,
digest и режим, после чего монтирует отдельный read-only каталог:

```text
/run/forge-inputs/
  inputs.json
  <artifact-id>/
    manifest.json
    files/<selected-relative-path>
```

В `/workspace/worktree` ничего не накладывается и не копируется автоматически.
Employee явно читает или копирует нужные файлы. Отдельный Git source bundle
находится в `/run/forge-source`; это другой канал входных данных.

## Ограничения M2

- До 64 файлов и 16 MiB суммарно на snapshot; до 8 snapshot inputs на Task.
- Только явно перечисленные обычные файлы. Нет glob, обхода каталогов или
  автоматического экспорта всего workspace.
- Запрещены traversal, symlinks, hardlinks, special files, `.git`, известные
  credential/service paths, runtime root и весь настроенный SecretStore.
  Проверяется каждый полный выбранный путь, даже при импорте из родительского
  каталога. Изменение файла во время чтения приводит к отказу.
- Проверка путей не является анализом содержимого на все возможные секреты.
  Оператор отвечает за явно выбранные документы.
- Сохраняются байты и executable bit, но не owner, timestamp, ACL и xattrs.
  Отдельного механизма review файлового snapshot без Git commit пока нет.
- Staging, файлы и незавершённая диагностика не удаляются автоматически.

## Проверка вручную

В отдельном управляемом Project с настроенным object storage и работающим Core:

1. Создай две незакрытые Task. В каталоге вне runtime выбери текстовый, пустой и
   бинарный файл. Выполни import в первую Task и дождись `sealed`.
2. Измени оригиналы. Прикрепи прежний `artifact_id` ко второй Task и запусти её:
   Employee должен получить прежние байты в read-only mount, без файлов в workspace.
3. Для capture явно останови работающую Task, дождись quiescence, выбери tracked
   и untracked файлы, выполни capture. Повтори команду с тем же idempotency key:
   операция и Artifact не должны дублироваться.
4. Проверь отказ на capture активного Run, чужой Project, symlink и `.git/config`.
   Ошибки не должны запускать/останавливать Run или менять lifecycle.

Keyless тесты покрывают canonical PostgreSQL commands, safe file I/O, Supervisor
guards и реальные Podman mounts/capture. Сквозной Core smoke использует InMemory
object backend, а настоящий MinIO проверяется отдельно. Команды и доказанные
границы — в [M2 closeout](2026-09-11-m2-closeout.md). Это не подтверждение работы
моделей. Основной [live Git launcher](M2_OPERATOR_SCENARIO.md) не включает
файловое упражнение в свой `passed`.
