# M2: реальный сценарий одной командой

Этот сценарий проверяет полный Git workflow выбранного провайдера: две задачи,
раздельные рабочие поверхности, адресные сообщения, общий Inbox, независимое
review, report-only QA и публикацию результата. Код и отчёты пишут настоящие
Employees. Скрипт создаёт только конфигурацию и вызывает именованные команды Core.

Один успешный прогон подтверждает только выбранную lane. Остальные провайдеры,
pin policy и файловые snapshots требуют отдельных проверок. Платные вызовы
не входят в обычный `just test-m2`.

## 1. Подготовить пустой playground

Твой `/home/zov/projects/forge-m2-playground` уже инициализирован через Git.
В нём пока не должно быть приложения, файлов или коммитов:

```bash
cd /home/zov/projects/forge-m2-playground
bash /home/zov/projects/forge/scripts/run-m2.sh init
```

Для другого каталога сначала создай его и выполни `git init --initial-branch=main`.
Используй отдельный checkout вне Forge. Linked worktree не подходит.

Если `./forge-m2` уже создан, повторять `init` не нужно. Пока bare target остаётся
без коммитов, можно снова выполнить `./forge-m2 run`: это создаст новую сессию,
сохранив прежние DB, runtime и evidence. Настройки и код launcher берутся из
текущего Forge checkout; бинарники пересобираются при подготовке.

Если хотя бы одна Integration уже опубликовала commit, нужен **новый** пустой
playground — даже если весь сценарий завершился `failed`. Старую target и
evidence не сбрасывай: это результаты реального прогона, а не disposable cache.

`init` создаст:

- `./forge-m2` — локальную команду, связанную с этим Forge checkout;
- `.forge-m2/config.json` — приватные настройки без содержимого ключей;
- `.forge-m2/target.git` — пустой bare target с unborn `main`.

Scaffold исключён через `.git/info/exclude`. Исходный checkout остаётся без
коммитов и приложения. Скрипт не делает фиктивный initial commit и не
перезаписывает существующий scaffold. Target выбрана bare, чтобы публикации
не мешала checked-out ветка.

## 2. Запустить

На Linux нужны Rust/Cargo, rootless Podman, podman-compose, Git, jq, util-linux
(`flock`, `uuidgen`) и стандартные GNU shell utilities. Используется существующая
Forge development topology. Подготовка может скачивать зависимости и image layers;
сама сборка не делает inference.

```bash
./forge-m2 run
```

Wizard спросит lane, точную модель и путь к авторизации. Для Codex предложит
текущий `auth.json` из `CODEX_HOME` или `~/.codex`, но прочитает токены только
после подтверждения расхода лимитов. Модель не подменяется автоматически.

| Lane | Что предоставить |
| --- | --- |
| `codex_cli` | Уже авторизованный Codex `auth.json` с ChatGPT subscription, не API-key auth. |
| `claude_cli` | Отдельный файл subscription setup-token и стабильную метку аккаунта. См. [Claude setup](M2_CLAUDE_RUNTIME.md). |
| `openrouter_api` | Файл upstream API key, выбранную модель и работающий настоящий локальный LiteLLM. |
| `openai_api` | Файл upstream API key, выбранную модель и работающий настоящий локальный LiteLLM. |

Файлы auth/token/key должны быть обычными, принадлежать текущему пользователю,
иметь права `0600`, одну ссылку и не быть symlinks. Содержимое ключа не вставляется
в аргументы CLI или config.json. Источник авторизации не изменяется; Core
сохраняет отдельную зашифрованную версию.

API wizard дополнительно спросит loopback HTTP origin LiteLLM, путь к его
master-key файлу и денежный лимит **на Run**. Gateway уже должен обслуживать
выбранную upstream модель. Launcher не ставит LiteLLM, не подключает fixture,
не угадывает модель и не делает fallback. Не используй synthetic gateway от
keyless integration tests для этого упражнения.

Настройки сохраняются, но каждый `run` снова требует подтверждения. Для явного
noninteractive запуска можно передать параметры:

```bash
./forge-m2 run \
  --lane codex_cli \
  --model YOUR_EXACT_MODEL \
  --auth-file /absolute/private/auth.json \
  --yes
```

`--yes` разрешает реальные расходы, а не только создание файлов. Не добавляй
его в CI или цикл автоматических retries. Не запускай этот сценарий параллельно
с другими live gates.

## 3. Что произойдёт

Launcher создаст отдельные DB, Secret Store, Core, Supervisor и короткий приватный
runtime-каталог `/tmp/fm2.XXXXXXXX`. Он соберёт текущий pinned runtime image,
используя максимум два Cargo build jobs по умолчанию. Общие development services
переиспользуются; canonical данные других Forge сессий не меняются.

1. Создаются stopped Project и Pipeline `work → review → qa → integrate → done`.
   Writer получает capacity 3. Reviewer и QA — другие Employees, сначала disabled.
2. Task A должна создать только `normalize-lines.sh`; Task B — только `README.md`.
   Обе начинают с unborn Git, каждая на собственной поверхности. Общего
   writable checkout у них нет.
3. Оба writer Run отвечают `READY` на свои канонические инструкции и ждут.
   Скрипт проверяет два живых контейнера и разные `/workspace` mounts.
4. Общий Inbox вопрос создаёт третий taskless Communication Run того же writer.
   Он читает доску через Forge, отвечает и завершает conversation assignment.
   Это не дополнительная Task и не доступ к чужим рабочим файлам.
5. Адресные `GO` отправляются точным Run/fence/epoch. Employees пишут, проверяют
   и коммитят свои файлы, прикрепляют `work_report` и предлагают Git candidates.
6. Только после принятия обоих первых candidates, физической остановки writers
   и проверки, что target ещё пустая, launcher включает reviewer и QA.
7. Reviewer рассматривает закреплённый read-only candidate и выдаёт честный
   verdict. QA выполняет конкретные проверки задачи и пишет `qa_report`.
   Автоматического поиска test framework или универсальной `run all tests`
   команды нет. Необязательные project hooks здесь не настроены.
8. Первая Integration создаёт target ref. Вторая встречает изменившуюся target
   и возвращает **ту же Task** по `stale_base` в work. Новый writer получает
   immutable source bundle и сам объединяет независимые истории, сохраняет оба
   файла, коммитит, повторяет свежие review и QA, затем публикует новый candidate.
9. После `done` launcher останавливает Project, дожидается наблюдаемой остановки,
   собирает evidence и прекращает собственные daemons. Он не исполняет
   полученный агентский код на host.

`--allow-unrelated-histories` нужен только для этого упражнения с двумя
независимыми root commits. Это не автоматическая merge policy Forge.

## 4. Наблюдать и остановить

Пока первый терминал занят прогоном, во втором:

```bash
cd /home/zov/projects/forge-m2-playground
./forge-m2 status
./forge-m2 evidence
./forge-m2 stop
```

`Ctrl-C` в терминале запуска также запрашивает остановку. Дождись завершения
cleanup: логическая `waiting` сама по себе не доказывает остановку контейнера.
Launcher ограничивает свои действия точными PID/lifetime identities и
собственными container labels + mounts. Общие development services не выключаются.

Во время Review → QA → Integration → rework launcher сразу, затем каждые
30 секунд печатает оставшееся время и сводку публичных состояний Runs:
ожидание запуска, provisioning, running, stopping и неопределённое состояние.
Отдельно считает незавершённые Employee-стадии без текущего активного Run и
задачи на системной Integration, которой Employee Run не требуется. Старые
остановленные Runs не считаются активной доработкой. Это наблюдения, не
доказательство прогресса модели или диагноз причины ожидания. Сводка не
перезапускает scheduler, не меняет Tasks и не сбрасывает deadline.

В начальном ожидании READY подтверждённый отказ или остановка writer Run,
потеря наблюдаемого состояния после старта либо `waiting`/закрытие Task прерывают
сценарий сразу. Launcher показывает Task/Run IDs, доступные публичные состояния
и путь к `supervisor.log`, затем выполняет обычный cleanup. Отсутствующий
`reason_code` выводится как `unavailable`: тип incident не выдаётся за полную
причину отказа. Начальное `unknown` при `provision_requested` остаётся нормальным
ожиданием старта. История старых Runs в последующем rework эту проверку не запускает.

Если launcher был убит через `SIGKILL` или хост перезагрузился, его cleanup
не мог завершиться. `stop` попытается закрыть gate и остановить подтверждённые
контейнеры этой сессии, но сообщит `unverified` и оставит диагностику для ручной
проверки. Он не сигналит угаданным PID и не выполняет `podman stop --all`.

Лимиты по умолчанию:

- один Run: 600 секунд, 2 CPU, 4 GiB RAM, 256 PID, 8 MiB вывода;
- весь сценарий: 1800 секунд от начала workflow, после сборки и подготовки
  конфигурации, при живом launcher; переходы между стадиями таймер не сбрасывают;
- обычно 10 provider Runs; остановка при наблюдении 12 Runs.

Последний лимит — polling guard: между наблюдением и остановкой может стартовать
дополнительный Run. Wall guard также требует работающего наблюдателя. API spend
cap действует на каждый Run, не на весь проект; in-flight расходы могут его
превысить. Денежная стоимость подписочных Runs остаётся неизвестной.
Лимиты сценария можно уменьшить в `.forge-m2/config.json` до запуска.
После истечения общего времени ещё выполняется остановка и сбор evidence,
поэтому итоговый выход команды может быть позднее самого deadline.

## 5. Где результат

Команды покажут приватный каталог сессии. Его путь также сохранён здесь:

```bash
jq . .forge-m2/current.json
./forge-m2 evidence
```

В сессии находятся `session.json`, `core.log`, `supervisor.log`, `setup.log`,
runtime files и `evidence/`. В `evidence/` сохранены публичные Task/Run проекции,
канонические сообщения, review records, Integration history, checkpoint overlap
и итоговый `report.json`. Полные тексты отчётов и private logs не публикуются
автоматически; они могут содержать чувствительные данные проекта.

Итог:

- `passed` — все заявленные наблюдения этого Git сценария подтверждены;
- `failed` — реальный прогон стартовал, но workflow, сбор evidence, проверка
  или остановка не завершились успешно;
- `unverified` — необходимых live evidence нет, например подготовка прервалась.

В `report.json` результаты разделены:

- `workflow` и `workflow_exit_status` — результат исполнения сценария до cleanup;
- `evidence_collection` — удалось ли собрать необходимые проекции и Git target;
- `cleanup` — результат остановки и очистки приватной копии авторизации;
- `assessment` — прошла ли приёмка полного набора evidence.

Ошибка экспорта не означает, что контейнеры продолжают работать: возможны
`workflow=passed`, `evidence_collection=failed`, `cleanup=passed` и итоговый
`outcome=failed`. При неполном сборе assessment остаётся `unverified`.
Старые отчёты не содержат новых полей; их отсутствие не считается успехом.

Большие JSON-проекции передаются в `jq` через stdin, а не аргументы процесса.
Пагинация сохраняет прежние ограничения числа страниц; некорректная страница,
повторный cursor или ошибка чтения прерывают сбор. Неудачное чтение или разбор
не заменяют ранее сохранённый файл пустым либо частичным JSON. Это атомарность
отдельного файла, а не всего набора evidence: при ошибке набор может содержать
проекции разных моментов и не допускается к успешной приёмке.

Если workflow завершился, но сбор отчёта сломался, сохранённую сессию можно
проверить отдельно: скопировать evidence в новый приватный каталог, read-only
экспортировать недостающие Events из её БД и выполнить файловую assessment.
Для этого не нужны новые provider Runs. Это операторское восстановление,
не автоматическая команда launcher: исходный отчёт остаётся неизменным,
а новый содержит источник, hashes, время экспорта и повторной проверки остановки.

Зелёный отчёт означает, что Forge принял результаты и выполнил заданный процесс.
Он не делает выводы LLM истинными и не заменяет твоё чтение review/QA отчётов.

Приложение находится в bare target, не в первоначальном пустом checkout:

```bash
git --git-dir=.forge-m2/target.git log --oneline --graph --all
git --git-dir=.forge-m2/target.git ls-tree -r --name-only main
git --git-dir=.forge-m2/target.git show main:normalize-lines.sh
git --git-dir=.forge-m2/target.git show main:README.md
```

Ожидаются оба файла, сохранённые worker commits и история stale-base rework.
Скрипт не делает checkout и не запускает код на host. Перед собственным запуском
прочитай код либо используй изолированную среду.

Повторный `run` — новая сессия, не resume. После публикации target уже непустая,
поэтому launcher предложит другой пустой playground. Старые DB, контейнеры,
Git history, настройки и логи сохраняются. Сброса и удаления данных нет.

## 6. Отдельные pin/file упражнения

Keyless-приёмка этих границ с настоящими Core/Supervisor/Podman и команды её
повтора описаны в [M2 closeout](2026-09-11-m2-closeout.md). Ниже — отдельный
ручной операторский сценарий; он не требуется для пересогласованного закрытия.

Они не включены в зелёный итог основного скрипта. Выполняй их в отдельном
управляемом Project с работающим Core; завершённый launcher собственный Core
останавливает. Все мутации используют стандартный command envelope с текущей
Project revision и уникальным idempotency key.

Для Git policy:

1. Прочитай `GET /v1/projects/P/tasks/T/git-source-policy`.
2. Выполни `set_task_git_source_policy` с `task_id`, `expected_policy_revision`,
   `policy: {"mode":"pinned_commit","commit":"FULL_SHA"}` и `reason`.
3. Убедись, что уже запущенный Run не изменился. Следующий writer должен получить
   новую policy revision и точный выбранный SHA. Его сохранённый HEAD/dirty state
   не сбрасывается автоматически.
4. Верни `policy: {"mode":"latest_target"}` отдельной командой с новой revision.
   Если pin несовместим с target, требуется явное решение, не скрытый unpin.

Для файлового входа без Git:

Источник и получатель новых артефактов должны быть незакрытыми Task; уже
запечатанный snapshot закрытой Task использовать можно. Подробный контракт и
ограничения описаны в [M2_FILE_SNAPSHOTS.md](M2_FILE_SNAPSHOTS.md).

1. `import_task_file_snapshot`: `task_id`, `expected_task_revision`, `title`,
   `root` — явный абсолютный каталог, `paths` — список конкретных обычных файлов.
   Или после явного stop и подтверждённой quiescence используй
   `capture_task_file_snapshot` с теми же полями, но без `root`.
2. Прочитай `GET /v1/projects/P/tasks/T/file-snapshots`, дождись `sealed`.
   При `pending`/`failed` не объявляй импорт успешным. Нужен настроенный S3/MinIO.
3. Для **другой** Task того же Project вызови `attach_task_file_input` с её
   `task_id`, текущей `expected_task_revision` и `artifact_id` sealed snapshot.
4. Новый Run получит `/run/forge-inputs/ARTIFACT_ID/manifest.json` и read-only
   `files/`. Employee копирует нужное явно; overlay поверх workspace отсутствует.
5. Измени исходный файл и проверь, что ранее созданный snapshot не изменился.
   Отдельно проверь пустой и бинарный файл и отказ на symlink/path traversal.

## Keyless проверка самого launcher

```bash
cd /home/zov/projects/forge
bash scripts/tests/m2-launcher-test.sh
```

Она создаёт временный пустой Git и использует synthetic CLI responses. Это
проверка scaffolding, secret/approval guards и command contracts, не live proof.
