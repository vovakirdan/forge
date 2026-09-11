# M3 acceptance

Статус: keyless-наборы, реальный `index` и полный ограниченный `run` с явно
разрешённой моделью Codex `gpt-5.6-luna` прошли 11 сентября 2026.
Успешные реальные запуски подтверждаются сохранённым `evidence/report.json`
и содержимым остальных evidence-файлов; другие профили этим не подтверждены.

Проверено 2026-09-11 на текущем рабочем дереве Forge:

- Четыре PostgreSQL/HTTP-теста `m3_memory_projection` прошли, включая настоящий
  owner-only UDS и поддельные ответы внешнего индекса.
- `scripts/tests/m3-launcher-test.sh` прошёл без сервисов и credentials.
- `bash scripts/run-m3.sh index --existing-services` завершился с `exit_code: 0`,
  `outcome: passed`, `cleanup_failed: false`. Evidence:
  `/tmp/fm3.1eC0IIfq/evidence/report.json`. Повторный прогон окончательного кода:
  `/tmp/fm3.36GwQTAI/evidence/report.json`, также `passed`, cleanup успешен.
- После index restart поиск сохранил запись; при outage использовался canonical
  fallback; после catch-up: `indexed: 2`, `pending: 0`, `retired: 2`.
  Provider Runs не создавались. БД и остановленный индекс сохранены.
- Полный `run`: `/tmp/fm3.SpnLckre/evidence/report.json`, `outcome: passed`,
  exit 0, cleanup успешен. Ровно три Runs: onboarding → Task → summary;
  все остановлены. Task `done`, один Artifact, personal note с тремя source refs,
  TaskSummary с восемью source refs и coverage 23–41. Итог индекса: indexed 4,
  pending 0, retired 2. После stop новая generation summary осталась pending,
  число Runs не выросло в шестисекундном контрольном окне.
- Независимо проверено совпадение реального provider stdin с frozen manifests,
  точная модель всех Runs и отсутствие Task/Employee-владельца у SystemJobs.
  Employee ошибся в трёх UUID при прямом `memory.read`; Core отклонил их.
  Успешный live indexed search подтверждён, успешный live `memory.read` — нет.
  Правильные IDs были в stdin; положительные read-кейсы проверены keyless-тестами.
  После прогона исправлен только общий текст `not_found`, с отдельной регрессией;
  повторный live для этой диагностической правки не запускался.

Полный перечень проверок и ограничений — в [execution ledger](2026-09-11-m3-tasks.md).

## Что проверяем

Есть три отдельных режима:

| Режим | Что реально запускается | Что подтверждает |
| --- | --- | --- |
| `keyless` | PostgreSQL в отдельных тестовых схемах, HTTP fixtures | Канонические команды, projection retry, SystemJob ownership/lifecycle/sources, context, Gateway и onboarding admission |
| `index` | Core, Supervisor, отдельный AgentMemory с локальными embeddings | Реальный strict import/search, сохранность после restart, canonical fallback и catch-up |
| `run` | Всё из `index`, плюс явно подтверждённые Codex Runs | Onboarding нового Employee, обычная Task, ownerless summarizer, source-linked memory |

Это проверки Forge, а не универсальная кнопка запуска тестов пользовательского
репозитория. Сценарий не использует личную базу AgentMemory и не меняет source auth.

`just test-m3` запускает shell-регрессии и все девять M3 PostgreSQL-наборов:
`m3_knowledge`, `m3_memory_projection`, `m3_system_job_ownership`,
`m3_system_job_lifecycle`, `m3_context`, `m3_system_job_gateway`,
`m3_system_job_sources`, `m3_onboarding_admission`, `m3_system_job_restart`.
Нужны уже настроенные и
работающие development services. `just run-m3 index` и `just run-m3 run` вызывают
соответствующие режимы launcher; только второй запрашивает разрешение на Codex.

## Подготовка

Нужны Linux, rootless Podman, рабочее окружение разработки Forge, `jq`, `openssl`,
`curl`, Git и Rust. Для режима `run` нужна действующая ChatGPT-авторизация Codex
в обычном принадлежащем пользователю файле с правами `0600`.

AgentMemory собирается из закреплённого fork source:

```text
Source revision: c65e93e31e8865ac4843360e4560c077a93d010b
Local image tag: localhost/forge-agentmemory:c65e93e31e8865ac4843360e4560c077a93d010b
Build recipe: deploy/forge/build.sh в agentmemory-forge-m3
```

Ревизия опубликована 2026-09-11 в ветке `forge/m3-indexing` репозитория
`https://github.com/vovakirdan/agentmemory.git`. Ветка `main` зависимости не менялась.
На другой машине, из каталога для исходников:

```bash
git clone --branch forge/m3-indexing https://github.com/vovakirdan/agentmemory.git agentmemory-forge-m3
cd agentmemory-forge-m3
git checkout --detach c65e93e31e8865ac4843360e4560c077a93d010b
sh deploy/forge/build.sh
```

В уже подготовленном локальном checkout зависимости:

```bash
cd /home/zov/projects/agentmemory-forge-m3
sh deploy/forge/build.sh
```

Скрипт сборки использует `git archive HEAD`, требует чистый tracked worktree и
записывает точную ревизию в OCI label. Launcher сверяет label, затем фиксирует
фактический image ID. `--agentmemory-image IMAGE` выбирает другой локальный alias
только той же закреплённой ревизии; скачивания произвольного runtime image нет.

Первая сборка скачивает зависимости и закреплённые MiniLM assets. Запущенный
экземпляр использует packaged local model: внешние модели отключены. У него новая
именованная volume, уникальная Podman `--internal` network и REST-порт только на
host loopback. Личный home, `.agentmemory`, сокет Podman и исходники проекта
в контейнер не монтируются. Подписочные ключи туда не передаются.

## Запуск

Из checkout Forge сначала можно выполнить keyless-проверки:

```bash
just dev-up
bash scripts/run-m3.sh keyless
```

Реальный индекс без Codex:

```bash
bash scripts/run-m3.sh index --existing-services
```

Полный сценарий с явным выбором модели и auth:

```bash
bash scripts/run-m3.sh run --existing-services \
  --model gpt-5.6-luna --auth-file /home/zov/.codex/auth.json
```

Перед чтением auth launcher показывает модель, источник авторизации и лимиты.
`--yes` заменяет это подтверждение для осознанного noninteractive запуска;
в таком режиме `--model` и `--auth-file` обязательны. Автоподмены модели нет.

Без `--existing-services` launcher при необходимости подготавливает стандартные
локальные dev-сервисы. С этим флагом он только проверяет уже работающие PostgreSQL
и NATS; не вызывает `dev-up` и не меняет их lifecycle.

По умолчанию создаётся отдельный пустой playground в `/tmp`; можно передать
`--playground /absolute/empty/directory`. Каталог должен существовать и быть
пустым: launcher сам выполняет безопасный `git init` без hooks/templates.
Использованный playground не перезаписывается. Каноническая БД, runtime surfaces
и полные логи находятся в отдельной короткой приватной сессии `/tmp/fm3.*`.
Путь также сохранён в `<playground>/.forge-m3/session.json`.

Рабочий каталог Task — собственная filesystem sandbox; этот сценарий не
повторяет Git integration gate M2. Код задания пишет Employee, а не launcher.

## Ход сценария

1. Core создаёт и публикует Policy про `violet compass`. Launcher ждёт
   результата настоящего AgentMemory search, который Core гидратирует из PostgreSQL.
2. AgentMemory перезапускается. Тот же canonical page находится без повторного
   импорта со стороны сценария.
3. AgentMemory останавливается. Search явно возвращает `canonical_fallback`,
   а новая опубликованная Policy остаётся доступной и образует backlog.
4. После старта индекса новая Policy находится через strict projection.
5. В режиме `run` задаётся ограниченная политика SystemJobs, затем создаётся
   новый Employee. Его состояние onboarding должно быть `pending`, не `skipped`.
6. После открытия project execution gate выполняется ownerless onboarding Run.
   Завершение требует принятого результата, физической остановки и personal note.
7. Employee получает обычную Task: написать и проверить `normalize-lines.sh`.
   Task проходит собственный Pipeline с обязательным `stage_evidence`.
8. Отдельный ownerless summarizer создаёт `task_summary` с canonical source refs
   и coverage. Он не утверждает правдивость отчёта и не меняет lifecycle Task.
9. Project execution останавливается. Pending summary не должен породить новый
   Run в контрольном окне. Cleanup отдельно проверяет физическую остановку.

Обычно нужны три provider Runs. Настройки сценария: максимум один активный Run,
четыре попытки SystemJob за скользящие 24 часа, одна попытка на generation job;
новые sources или явный retry могут создать следующую generation того же job.
Task до 600 секунд,
SystemJob до 300 секунд. Общий срок — 1800 секунд после подготовки сервисов.
Launcher запрашивает stop при наблюдении шести Runs; между опросами возможен
дополнительный Run. Это расход лимита подписки, а не гарантия денежной стоимости.
Глобальный таймаут и observed Run guard действуют, пока launcher жив.

## Доказательства и остановка

Полный evidence находится в напечатанном `<session>/evidence/`:

- `report.json`: режим, итог, exit code и результат cleanup.
- `index-runtime.json`: source revision, image ID, точные container/network/volume IDs.
- `index-before-restart.json`, `index-after-restart.json`, `index-outage.json`,
  `index-caught-up.json`: индекс, сохранность, деградация и восстановление.
- `onboarding-before.json`, `onboarding-current.json`: pending → completed receipt.
- `task.json`, `memory-after-onboarding.json`, `memory-final.json`,
  `memory-query-final.json`: результаты работы и derived memory.
- `system-jobs.json`, `runs.json`, `run-<id>.json`: отдельные владельцы и Run diagnostics.
- `pinned-contexts.json`: полные сохранённые RunSpec и context manifests, включая
  source envelopes, revision/hash pins и coverage; это приватные данные проекта.
- `stop-proof.json`: отсутствие нового dispatch после project stop в шестисекундном окне.

`Ctrl-C` запускает named project stop и проверенный cleanup собственных процессов
и контейнеров. Ничего не удаляется: сессия, БД, index volume, остановленный
AgentMemory container и его network остаются для диагностики. Не публикуйте
каталог сессии: там есть encrypted Secret Store, отдельный индексный token,
runtime credential-файлы, приватные prompts и логи. Временная копия source auth удаляется после enrollment;
исходный auth-файл сохраняется.

Прохождение `index` не подтверждает настоящий SystemJob или Employee Run.
Прохождение `run` не заменяет keyless-проверки malformed output, поздних ответов,
fencing, stop активной работы и scope violations. Эти границы проверяются отдельно;
`stop-proof.json` доказывает только наблюдавшийся запрет нового dispatch.
