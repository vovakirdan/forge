# M2: исходная версия, файловые входы и live-сценарий

Статус: M2 закрыта по пересогласованной приёмке 11 сентября; provider live-gates отложены.
Дата: 9 сентября 2026.
Актуализация: 11 сентября 2026. Этот документ дополняет `2026-09-06-m2-specs.md`.
Владелец отложил authenticated provider gates; они не считаются пройденными.
Текущий статус — в [M2 closeout](2026-09-11-m2-closeout.md). Датированные записи
ниже сохраняют историю результатов, в том числе прежние условия закрытия M2.

## Git source policy

У Task отдельная версия настройки исполнения: `latest_target` по умолчанию
или `pinned_commit` с полным SHA. Human/Manager меняет её именованной командой
`set_task_git_source_policy`. Настройка действует на будущие writer Runs до
следующего изменения; каждый Run фиксирует её revision и выбранную Git revision.
Изменение этой настройки не меняет Task revision и не инвалидирует текущий
Git proposal. Существующий Run не меняет исходную версию задним числом.

Начальное состояние поверхности — явное `Commit` или `Unborn`, без нулевого SHA
и искусственного initial commit. Новая поверхность начинается с выбранной
версии. Сохранённая поверхность сохраняет HEAD, index, commits и dirty files;
изменение policy не вызывает checkout/reset. Несовместимость закреплённого
источника с integration target требует явного решения Manager/Human, а не
автоматического unpin или бесконечного retry.

## Git delivery и первая публикация

Supervisor до старта каждого нового writer подготавливает immutable Git bundle
и descriptor из зарегистрированного источника. Предыдущий владелец поверхности
должен быть физически остановлен. Точная выбранная revision журналируется до
экспорта; повторное выполнение использует тот же выбор. Bundle доставляется
отдельным read-only mount. Employee сам выполняет локальный fetch и решает,
когда делать merge/rebase, разрешать конфликты и коммитить.

Path-free результат подготовки доступен в canonical audit и диагностике Run.
Ошибка экспорта, нехватка места или неопределённость не разрешают старт. После
подготовки повторно проверяется stop. Данные при отказе сохраняются.

Пустой Git остаётся unborn, первый настоящий commit создаёт Employee. Первая
Integration публикует принятый candidate напрямую через create-only CAS в
отсутствующую target ref. Для существующей target сохраняется exact-candidate
двухродительский merge/CAS. Гонка первых публикаций возвращает reconcile или
stale_base, без перезаписи победителя. Удалённая ранее существовавшая ветка не
считается новым пустым проектом.

Нужны новый TaskStage RunSpec и additive migrations после 0033. Старые Runs,
bindings и Integration intents остаются читаемыми с прежней семантикой; старый
Supervisor не может молча проигнорировать обязательную новую возможность.

## Файловые snapshots

Snapshot — immutable Artifact задачи: manifest, относительные пути, размер,
executable bit и SHA-256 каждого выбранного файла. Git commit не нужен.
Источники: явно выбранные локальные файлы оператора либо выбранные файлы
остановленной Task, которые читает Supervisor по команде Core.

Human/Manager действует через именованные команды. Capture не останавливает
Run неявно: сначала stop, затем доказанная quiescence. Резервирование поверхности
действует до фиксации snapshot. Только явно перечисленные обычные файлы: без
рекурсивного обхода и glob по умолчанию. Traversal, `.git`, служебные/credential
пути, symlinks, hardlinks и special files запрещены.

Байты, включая пустые и бинарные файлы, хранятся неизменно через object_store /
MinIO. Redacting EvidenceSpool логов здесь не применяется. Отдельная команда
прикрепляет snapshot как вход будущих Runs другой Task того же Project.
Доставка — отдельный read-only mount, без overlay/перезаписи workspace.
Employee явно копирует нужные файлы. Snapshot не является GitCandidate,
review approval или разрешением Integration; lifecycle задачи не меняется.
Review snapshot без commit не входит в эту итерацию.

## Launcher

Из отдельного пустого Git checkout:

```bash
cd /home/zov/projects/forge-m2-playground
bash /home/zov/projects/forge/scripts/run-m2.sh init
```

`init` создаёт ignored private config, пустую bare target и локальную команду
`./forge-m2`; он не пишет приложение и не создаёт initial commit. Команды:
`run`, `status`, `stop`, `evidence`. Wizard выбирает одну из четырёх lanes:
Codex CLI, Claude CLI, OpenRouter API, OpenAI API, с явными model/auth inputs.
Текущая Codex auth предлагается, но читается только после подтверждения.
API требует настроенный настоящий LiteLLM: fixture и скрытый fallback запрещены.

Каждая сессия изолирована: отдельная DB, private runtime, закреплённый текущий
образ, ограничение каждого Run и всей сессии. Расход реальных лимитов требует
подтверждения; автоматических платных повторов нет. Writer capacity — 3,
reviewer и report-only QA — отдельные Employees. Pipeline:
work → review → qa → integrate → done, rework остаётся в той же Task.

Две задачи стартуют из unborn: A создаёт только `normalize-lines.sh`, B — только
`README.md`. READY/GO сообщения точным Runs подтверждают overlap. Общий Inbox
вопрос writer создаёт третий, taskless Communication Run того же Employee.
Reviewer и QA сначала disabled; после accepted первых candidates и остановки
обоих writers launcher включает их именованными командами. Первая Integration
создаёт target, вторая получает stale_base. Повторный writer получает свежий
bundle, явно объединяет независимые истории, коммитит и проходит новые review/QA.
`--allow-unrelated-histories` — инструкция только этого упражнения, не default
Forge. Launcher не пишет код или verdicts и не исполняет агентский код на host.

Evidence: публичные API, фактический target SHA и физическая quiescence
контейнеров. Ctrl-C/stop завершают только принадлежащие сессии процессы и
контейнеры. Неуспешные сессии сохраняются; rerun не делает implicit resume.

## Проверки и критерий завершения

- Git: latest/pin/future change; retained dirty state; moved target; replay,
  restart/stop; unborn; create-only CAS race; stale rework и повторное review.
- Snapshots: без Git; tracked/untracked; неизменность после изменения источника;
  пустые/бинарные файлы; scope/path attacks; отсутствие overlay и writer race.
- Launcher: keyless contract tests; approval guard; secrets не попадают в
  settings/logs; timeout/stop; явные passed/failed/unverified.
- Регрессии M0–M2 и независимое review с исправлением findings.
- Реальный полный Git сценарий выбранной lane и отдельные keyless pin/file
  exercises с настоящими файловыми и контейнерными границами.
  По решению владельца 11 сентября четыре authenticated provider gates и
  отдельная Claude MCP/artifact-приёмка отложены и не блокируют M2. Один Codex
  не подтверждает остальные lanes; deferred не означает passed.

## Рабочий checklist

- [x] Source policy, Core/storage, совместимость RunSpec.
- [x] Git bundle delivery, unborn и первая Integration.
- [x] Snapshot import/capture/attachment/delivery.
- [x] Launcher, сценарий и операторская документация.
- [x] Keyless проверки и регрессии.
- [x] Независимое review и исправления.
- [x] Полный выбранный Codex Git-сценарий принят по сохранённым live evidence.
- [x] Pin/file exercises без модели с настоящим rootless Podman.
- [ ] Отложенные authenticated provider gates с отдельным разрешением лимитов.

Контракты оператора: [Git source policy](M2_GIT_SOURCE_POLICY.md),
[файловые snapshots](M2_FILE_SNAPSHOTS.md),
[live-сценарий](M2_OPERATOR_SCENARIO.md).

Независимое review привело к исправлениям:

- полный выбранный путь проверяется против runtime root и SecretStore;
- pending capture блокирует Integration и dispatch; blocking reader удерживает
  provisioning/journal при отмене async worker;
- очередь captures обходит зависшие операции по кругу, без голодания импортов;
- `Stopped` и quiescence записываются атомарно; source preflight различает
  физически остановленный контейнер и ещё не полученный Core ACK;
- reviewer/QA получают read-only profiles, а native GO receipt проверяется
  по конкретному сообщению, Run, fence и epoch.

Общий регрессионный прогон также выявил пропущенный `TaskStage` assignment
при отправке нового RunSpec v6. Core теперь передаёт точные Task/stage/Queue
identities; wire-контракт покрыт тестом. Повторный M1 runtime suite, включая
реальный Podman без провайдера, прошёл.

## Выполненная проверка реализации

9 сентября 2026, с `CARGO_BUILD_JOBS=2`:

- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
  — passed.
- `cargo test --workspace --all-features --locked -- --quiet` — passed;
  opt-in integration/live tests остаются ignored в этой команде.
- `just test-integration` — passed целиком: PostgreSQL/NATS/Redis/MinIO,
  11 настоящих Podman sandbox tests, регрессии M0–M2 и M0 CLI smoke до `done`.
  В частности, прошли 22 Git workflow tests, 5 snapshot PG tests и отдельный
  сквозной тест доставки immutable inputs в будущие Runs.
- Supervisor capture tests — passed: настоящий Git worktree, tracked/untracked,
  binary/empty files, restart/replay, отказы при чужой или активной поверхности.
  Физический inspector в этих трёх тестах синтетический.
- Shell contract tests M1 input/commands/session и M2 repository/launcher,
  `bash -n`, `git diff --check` — passed.
- OpenAPI YAML читается, все 458 локальных `$ref` разрешаются.

Настоящие MinIO и Podman в keyless тестах не являются proof работы модели:
использовались synthetic credentials/runtime, без provider inference. На момент
этой проверки реальные CLI/API gates, полный операторский прогон и live pin/file
exercises ещё не запускались. Playground был передан пустым, без коммитов
и scaffold; `init` и разрешение расхода лимитов остаются явными действиями оператора.

## Исправление после первого live-запуска

Операторский Codex-запуск 9 сентября завершился `failed` на READY-barrier.
Первый writer ответил `READY`, второй получил `StartFailed` ещё до создания
контейнера. Cleanup подтвердил остановку; target осталась без коммитов.
Этот запуск не подтверждает закрытие M2 или работу остальных provider lanes.

Причина: NativeMailbox создаёт штатный `input-receipts` внутри evidence epoch,
а прежний capacity scan запрещал там все каталоги. Исправление разрешает только
этот одноуровневый каталог. Его обычные файлы, включая временные `.pending-*`,
учитываются в общих byte/entry limits. Закреплённые directory descriptors и
metadata-only leaf handles предотвращают переход по подменённым symlinks;
произвольные каталоги, вложенность и special files по-прежнему запрещены.
Удаление receipt после Core ACK между перечислением и проверкой допустимо.

Начальный READY-barrier теперь отдельно проверяет состояние writers и Task:
подтверждённый отказ завершает его с диагностикой без ожидания всех 180 секунд.
Начальное неопределённое состояние до provision и штатные последующие rework
не считаются отказами. Автоматического нового платного Run нет.

Регрессионный тест с двумя настоящими Podman-контейнерами и штатным конструктором
NativeMailbox воспроизвёл `UnsafeSurface` до исправления и прошёл после него.
Провайдер и inference в этом тесте не используются. Добавлены unit-проверки
capacity/races и shell-проверки раннего отказа, здорового старта и безопасной
диагностики. Повторная live-приёмка остаётся действием оператора.

После исправления повторно прошли `cargo fmt`, workspace Clippy с `-D warnings`,
обычный workspace test suite и 12 opt-in Supervisor sandbox/hook tests, включая
новую двухконтейнерную регрессию. PostgreSQL/NATS/MinIO aggregate suite на этом
этапе повторно не запускался. M2 launcher/READY shell tests, `bash -n` и
`git diff --check` также прошли.

Независимое RO-ревью выявило неверное ожидание объекта вместо строкового enum
`runtime_report.failure` в диагностике launcher. Чтение исправлено и покрыто
тестом с реальной формой API; повторное ревью не обнаружило новых findings.
Реальный провайдер повторно не запускался; исходная live-сессия сохранена.

## Исправление после второго live-запуска

Повторный Codex-прогон подтвердил две отдельные рабочие поверхности и overlap
трёх Runs: двух writers и taskless Communication. Communication завершилась
успешно. Полный Git-сценарий до `done` этот запуск не подтвердил.

Writer A повторно получил READY через native input, хотя сообщение уже было
видно в bootstrap. Заголовок общей функции `addressed_prompt` подставлял
`DeliverRuntimeInput.command_id` вместо canonical Inbox message `id`.
Employee использовал этот ID в `inbox.acknowledge`, получил `not_found` и
создал `escalation_pending`. Настоящее GO для A осталось в очереди, без native
acceptance. Writer B ответил на GO; затем project stop/cleanup остановил сессию.

Принятое исправление разделяет два идентификатора: wrapper извлекает единственный
top-level `id` из canonical source как typed UUIDv7 и показывает его Employee.
Delivery command ID остаётся во внутренних native RPC, events и receipts.
Отсутствующий, некорректный или дублированный `id` даёт безопасную ошибку без
payload; новые зависимости и полная повторная domain validation не нужны.

Повторная доставка bootstrap-visible сообщения допустима при at-least-once
transport. Исправление не создаёт semantic ACK, не подавляет доставку и не меняет
Inbox policy. Регрессии проверяют Codex bootstrap READY → native READY → GO →
close и различие canonical/native transport IDs в OpenCode без paid calls.
На старом helper регрессии common, Codex и OpenCode воспроизвели ошибку.
После исправления прошли 5 common prompt tests, 2 Codex native-session tests
и 3 OpenCode native-session tests. Дополнительно проверены JSON-object shape,
отказ на массив с UUID вместо объекта и сохранение допустимых пробелов в source.
Bootstrap READY в тесте — synthetic provider event, не настоящий Gateway ACK.

Повторно прошли `cargo fmt --all -- --check`, workspace Clippy с
`--all-targets --all-features --locked -- -D warnings`,
`cargo test --workspace --all-features --locked -- --quiet` и `git diff --check`.
Opt-in service/container/live tests в этом прогоне остались ignored:
изменена только model-facing обёртка, а не storage, RPC или sandbox boundary.
Независимое RO-ревью новых findings не обнаружило.

Реальный провайдер повторно не запускался; сохранённая сессия не изменялась.
Полная live-приёмка ещё не подтверждена. M2 остаётся открытой.

## Регрессия live-прогона 10 сентября: Integration → rework

Реальный Codex-прогон обнаружил потерянное пробуждение scheduler: после
`stale_base` Task и новая QueueEntry были сохранены, но все предыдущие Runs
уже остановились и больше не было события, запускающего dispatch. Общий
deadline и cleanup сработали; это не успешная live-приёмка.

После commit результата Integration Core вызывает обычный dispatcher, который
заново проверяет gate, физическую quiescence, capacity и остальные условия
допуска. Идентичный receipt может повторить этот вызов, но не применение
результата. Ошибка dispatch логируется и не превращает уже принятый receipt в
отказ. Это не новая периодическая retry-система: после ошибки следующий вызов
требует существующего wake/replay. Новый Run не создаётся обходным путём.

Регрессионные проверки не вызывают dispatcher вручную после `stale_base`:
проверяют самостоятельный rework после остановки всех Runs, повторный receipt,
ошибку dispatch после commit и закрытый gate. Launcher дополнительно показывает
состояния исполнения каждые 30 секунд, не добавляя управляющих команд.

Проверка 10 сентября 2026: regression test без ручного dispatch сначала
воспроизвёл отсутствие нового `ProvisionRun`, затем прошёл после исправления.
Прошли все 12 целевых integration tests, общий `bash scripts/test-m2.sh`,
workspace Clippy с `--all-targets --all-features --locked -- -D warnings`,
`cargo fmt --all -- --check`, launcher shell tests, `bash -n` и
`git diff --check`. В общем наборе прошли все 25 Git workflow tests.
Использовались реальные PostgreSQL/NATS и изолированные Git fixtures,
синтетические Supervisor/provider observations; paid inference не запускался.
Независимое RO-ревью закрыто без нерешённых findings.

Исходная live-сессия и её target сохранены. Первая Task уже опубликовала commit,
поэтому повторная live-приёмка требует нового пустого playground по
[operator scenario](M2_OPERATOR_SCENARIO.md). M2 остаётся открытой.

## Регрессия live-прогона 10 сентября: большой журнал evidence

В следующей сессии `/tmp/fm2.LLNIphjg` с `codex_cli / gpt-6-astra` обе Task
дошли до `done`, включая stale-base rework, повторные review/QA и публикацию.
Все 10 Runs остановились. Экспорт Events упал в `m2_events`:
JSON передавался одним аргументом `jq --argjson` и превысил локальный лимит
аргумента ОС (128 KiB). Исходный launcher report остался `failed`, assessment
не выполнялась; ошибка экспорта также ошибочно выставляла `cleanup=failed`.

Согласованное исправление:

- большие JSON в Events, пагинации, progress/status, overlap checkpoints,
  session patch и итоговом отчёте передаются через stdin;
- malformed/multiple-document API page, неверный порядок событий и повторный
  cursor прерывают сбор без публикации частичного результата;
- код возврата источника проверяется до записи; неудачное чтение либо разбор
  не перезаписывают прежний файл evidence;
- workflow, evidence collection, cleanup и assessment имеют отдельные статусы;
  при неполных evidence итог остаётся неуспешным, assessment не запускается.

Ограничения числа страниц и wall/run guards не увеличивались. Новые сервисы,
зависимости, provider retries и изменения canonical state не добавлялись.

Регрессии воспроизвели E2BIG до исправления и проверили одностраничный ответ
больше 128 KiB, накопленный журнал/список больше 2 MiB, Unicode, порядок,
ошибки курсора/JSON и сохранность прежних файлов. Отдельно проверены 16
комбинаций workflow/collection/cleanup/assessment, EXIT trap, Ctrl-C,
ошибки stop/target и крупные projections. Прошли `m2-launcher-test.sh`
(включая READY, progress, JSON и report tests), `m2-repository-test.sh`,
6 CLI unit tests и 2 Core event-projection tests, `bash -n`, workspace fmt,
Clippy с `--all-targets --all-features --locked -- -D warnings` и
`git diff --check`. ShellCheck недоступен. Полный PG/NATS aggregate suite
повторно не запускался: изменения ограничены shell-обвязкой.

### Восстановленная проверка, не повторный provider run

Исходные `session.json`, evidence и Git target сохранены. В отдельном
приватном каталоге `/tmp/forge-m2-recovery.2mjuRuEL` собрана их копия и
экспортированы 271 canonical Events (sequence 1–271) через PostgreSQL
`REPEATABLE READ READ ONLY`. Экспорт соответствует публичному EventEnvelope;
исходные серверные JSON bytes сохранены отдельно. Core/Supervisor и агенты
для этого не запускались, код Employees на host не исполнялся.

Файловая `m2_assess` прошла, включая GO receipts и Communication completion.
Повторно проверены отсутствие живых session containers и исходных процессов
Core/Supervisor, неизменность исходных файлов по SHA-256 и target commit
`55659f975927d9750faa97f322e86e02eb98d8bc`. В recovery-каталоге сохранены
копии кода приёмки, manifests и отдельный `report.json` с
`kind=m2_recovered_offline_assessment`, `assessment=passed`.

Это подтверждает выбранный Codex Git-сценарий по сохранённым live evidence,
но не превращает первоначальный launcher exit в успешный. Другие provider
lanes и pin/file упражнения этим восстановлением не проверялись; M2 в целом
остаётся открытой. Семантическая верность отчётов Employees не выводится из
результата файловой assessment.

## Закрытие 11 сентября 2026

После решения владельца отложить authenticated provider gates пройдены
физические pin/snapshot exercises без модели, общий keyless M2 gate, полная
интеграционная регрессия M0–M2 с CLI smoke, workspace tests, fmt и Clippy.
Независимое review новых тестов закрыто без unresolved findings. M2 закрыта
по этому объёму; провайдерная live-приёмка остаётся unverified. Точные команды,
границы InMemory/MinIO, результаты и evidence — в
[M2 closeout](2026-09-11-m2-closeout.md).
