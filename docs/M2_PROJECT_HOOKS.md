# M2 — явные project hooks

Hook — настроенная владельцем команда проекта, а не агент, QA-работник или
автоматически найденная система тестов. Forge ничего не ищет по `package.json`,
`Makefile` или имени стадии. Отсутствие hooks — нормальная конфигурация.

## Версия и настройка

`configure_project_hook` создаёт immutable `ProjectHookVersion`. Core назначает
UUIDv7 и время, сохраняет версию вместе с Project revision, Event и idempotent
receipt. Повтор с тем же ключом возвращает ту же версию. Новая команда с тем же
именем создаёт **другой ID**: ни старые версии, ни Pipeline pins не меняются.
Удаление и редактирование версий не поддерживаются.

Поля: `name`, `image` с OCI digest, `command` (исполняемый файл и argv),
относительный `workdir`, `limits`, `max_output_bytes`, `applicable_task_kinds`,
`required`. Пустой список применимости означает все Task kinds. Пароли, ключи,
provider profile и Employee ID не входят в контракт; команда должна быть
несекретной. Идентификатор проекта берётся из command envelope.

Пример настройки (замените project/revision и image на свои):

```sh
forge command configure_project_hook \
  --project-id '<project-uuid>' --expected-revision 1 \
  --idempotency-key 'configure-checks-v1' \
  --payload '{
    "name":"Repository checks",
    "image":"localhost/forge-project@sha256:<64-hex-digest>",
    "command":["/usr/bin/make","check"],
    "workdir":".",
    "limits":{"cpu_millis":2000,"memory_bytes":1073741824,"pids":128,"wall_seconds":300,"stop_grace_seconds":10},
    "max_output_bytes":4194304,
    "applicable_task_kinds":["delivery"],
    "required":true
  }'
forge get '/v1/projects/<project-uuid>/hook-versions?limit=50'
```

GET возвращает `items` и `next_cursor`; следующая страница использует `after`.
Это локальный операторский API, не инструмент Employee Gateway.

## Pipeline pin и результаты

Стадия явно объявляет `executor_kind: system`, без Employee workspace/policy:

```json
{
  "kind":"project_hook",
  "hook_version_id":"<version-uuid-from-receipt>",
  "outcomes":{
    "passed":"checks_passed",
    "failed":"needs_changes",
    "timed_out":"needs_changes",
    "skipped":"checks_not_applicable"
  }
}
```

Этот объект — значение `system_action`. Все указанные outcome keys должны
совпадать с объявленной матрицей переходов стадии. Название стадии произвольное.
Для required hook ошибки и timeout могут вести только в Employee-доработку либо
Human/External wait, не непосредственно в Done. Advisory hook следует своей
матрице. Неприменимость — явный `Skipped`, а не отсутствующий отчёт.

Core проверяет применимость **до** требования Git. Для неприменимой стадии
сохраняются типизированный `HookSkipSpec`, запись invocation и Task artifact
`hook_result`; Run, Lease, candidate и sandbox не создаются. Поэтому analysis
или filesystem-задаче не нужен фиктивный репозиторий ради пропуска команды.
Далее применяется настроенный `skipped` outcome и обычные ограничения Pipeline.

Требование привязано к точному accepted candidate и immutable версии hook.
Все применимые required hooks версии Pipeline должны быть удовлетворены перед
Done и Git integration, даже если выбранная ветка обошла их стадии. Нет общей
обязательной стадии QA. Новый candidate не наследует успех старого commit.

## Core: admission и принятие отчёта

Core сам выбирает явно настроенную System-стадию и immutable Hook version.
Для применимой команды он фиксирует текущие Task revision, Pipeline version,
stage visit, accepted proposal и точный commit/tree. Invocation, provider-free
Run, Lease, физическая reservation и Event сохраняются до отправки Supervisor.
Перед отправкой Core повторно проверяет разрешение под блокировкой Project.
Заблокированные задачи и recovery-held проекты не занимают первые 64 позиции
ограниченного сканирования. Невалидная связка конфигурации и Pipeline оставляет
понятный wait конкретной Task, не ломая dispatch остальных задач проекта.

Результат команды и разрешение двинуть Task — разные факты. Core принимает
переход только после `Stopped`, при совпадении invocation, candidate,
Task revision и stage visit, в том же Core instance и при открытом Project.
Остановка, новый Core, устаревший или неполный результат сохраняют свидетельство,
но не дают права применить старый outcome. Актуальная стадия получает явный wait.

При обычном принятии Core прикрепляет `hook_result` к той же Task, применяет
заданную матрицу переходов и сохраняет System-action handoff без фиктивного
Employee или TaskStage Run. Непрочитанное обязательное сообщение, недостающий
required hook или невозможный переход удерживают задачу; отчёт не теряется.

## Sandbox и наблюдение

`HookRunSpec` v5 не содержит `RuntimeBinding`, модель или credential binding.
`ExecutionAssignment::Hook` несёт invocation ID, контекст Task, Pipeline version,
stage visit и candidate proposal. Это не TaskStage lease и не Employee:
в общих Run/Lease/physical projections Employee отсутствует.

Supervisor проверяет исходный host-owned manifest Task и создаёт **отдельный
Run-private Git snapshot** точного commit/tree. Рабочая копия hook доступна на
запись для временных файлов и результатов тестов. Исходный репозиторий, Task
writer, его dirty/untracked/ignored файлы и manifest не копируются поверх и не
изменяются. Snapshot сохраняется после запуска; автоматической очистки нет.

В M2 image должен содержать `/usr/local/bin/forge-runner`: можно расширить
Forge runtime bundle зависимостями своего проекта. Image pinned и уже установлен
локально (`--pull=never`); network отключён. Wrapper запускает явный argv внутри
rootless Podman, очищает окружение, ограничивает stdout/stderr и получает общие
CPU/memory/PID/wall-time ограничения. Gateway, proxy relay, provider credentials
и директории других Task не монтируются. Shell допустим как явно выбранный
исполняемый файл **внутри контейнера**, но не как host executor.

Lifecycle, физическая capacity, остановка и recovery остаются общими с другими
Run purposes. Перед новой работой нужен обычный admission; перезапуск не даёт
разрешения автоматически повторить команду с возможными side effects.

Supervisor добавляет к `Stopped.details_json` typed `hook_result` с invocation,
candidate proposal, commit/tree, `verdict`, `exit_code`, `output_incomplete`.
`passed` возможен только при exit 0 контейнера и процесса, полном отчёте и
отсутствии остановки. Missing/bad report, неполный вывод и прерванный запуск не
считаются успехом. Причина `wall_limit` сохраняется в journal после ACK и restart;
она даёт `timed_out`. Stop даёт `interrupted`, требующий management decision.

Проверки wrapper подтверждают факт выполнения настроенной команды, не истинность
всех её утверждений и не корректность произвольного кода проекта.

## Проверка реализации

Domain/Supervisor tests проверяют изоляцию purpose, отсутствие provider material,
exact candidate copy, отказ от mutable Task files, неполные отчёты, timeout и
journal replay. `m2_hook_registry` проверяет реальный private-schema PostgreSQL,
named command/idempotency, HTTP pagination/scope и запрет UPDATE/DELETE версии.
Это не live-проверка пользовательского репозитория или оплачиваемого провайдера.

Отдельные `podman::hook::tests::runtime` действительно запускают явную shell-команду
через `forge-runner` в rootless Podman. Проверены exit 0 и exit 7, точное содержимое
accepted Git snapshot, отсутствие dirty/ignored остатков writer и credentials,
запись результата только в private copy и положительная физическая quiescence.
Контейнеры остаются остановленными, файлы и evidence сохраняются. Все обращения
теста к Podman ограничены по времени; при сбое cleanup адресует только точный
контейнер этого Run. Эти два теста включены в общий `just test-integration`.

### Барьер завершения

Перед каждым Done и Git integration Core проверяет все применимые required
hooks pinned PipelineVersion. Проверка использует последний invocation для
точного Task, Pipeline version, stage, hook version и candidate proposal.
Новый running, held или failed invocation перекрывает прежний passed для той
же комбинации; исторический успех не служит запасным разрешением. Результат
должен совпадать также по invocation ID и проверенным commit/tree.

При принятии writer-предложения барьер проверяет именно новый proposal,
а не последний ранее принятый candidate. Неприменимые и advisory hooks не
создают обязательной проверки. Без настроенных hooks не требуется ни Git,
ни runner, ни искусственный успешный отчёт.

`m2_hook_runs` проверяет настоящий PostgreSQL/Core/Gateway и отдельный временный
Git repository. Проверки охватывают успешное завершение той же Task, ошибку,
missing/late/mismatched result, explicit skip без Run, обход стадии, повторную
проверку того же candidate и новый candidate после человеческого решения.
Supervisor reports в этих сценариях управляются тестом; это проверка
оркестрации, а не запуск команды hook или оплачиваемого провайдера.
