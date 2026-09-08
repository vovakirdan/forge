# M2: ревью и QA конкретного commit

Имена стадий ничего не включают автоматически. Для read-only стадии Git-bound
Task Core выдаёт отдельный снимок последнего принятого candidate. Supervisor
создаёт его из commit через GitBackend; ignored и незакоммиченные файлы writer
не копируются. Scratch runtime остаётся отдельным от read-only source.

PipelineVersion может задать явную policy:

```json
{
  "id": "examine",
  "name": "Экспертная оценка",
  "executor_kind": "employee",
  "outcomes": ["yes", "again"],
  "workspace": {"kind": "git", "access": "read_only"},
  "acceptance_policy": {
    "kind": "candidate_review",
    "independent": true,
    "verdicts": {"yes": "accepted", "again": "rejected"}
  }
}
```

Соответствующие transitions отдельно определяют, куда ведут `yes` и `again`.
Все объявленные outcomes должны иметь verdict: `accepted`, `rejected` или
`inconclusive`. Core не определяет verdict по тексту отчёта и не знает
специальных названий `review`, `test` или `approved`.

Если `independent` включён (это default), scheduler исключает всех Employee,
которым Core выдавал writable работу над candidate, включая прерванные попытки.
Git author и последний Employee недостаточны. Проверка повторяется при принятии
outcome. Менеджерская привязка следующего исполнителя не обходит policy: она
становится явно заблокированной, если выбранный Employee не подходит.

Candidate review всегда требует Task-owned Git binding — даже если независимость
отключена. Employee-owned Git source не заменяет эту привязку. Core проверяет
это при approve, dispatch и приёме результата; отсутствие binding не превращает
настроенную policy в необязательную.

Read-only Employee прикрепляет отчёт к той же Task и в `outcome.submit` явно
указывает `candidate_commit`. Не тот SHA или уже устаревший snapshot отклоняются.
Разрешённый outcome и immutable CandidateReviewRecord сохраняются атомарно:
candidate/proposal, автор оценки, Run/fence/epoch, PipelineVersion/stage visit,
исходные артефакты и артефакты оценки. Это структурная приёмка, не доказательство
истинности выводов Employee. При возврате на работу не создаётся FixTask.

После новой writer-попытки старые оценки остаются историей своей ревизии, а не
разрешением автоматически принять следующую. Следующий writer не стартует,
пока предыдущий Run — в том числе reviewer — не освободит физическую резервацию.

QA — работа Employee по своему заданию. Report-only QA может использовать
read-only Git stage без acceptance policy: он оставляет отчёт и разрешённый
Pipeline outcome, но Core не выдумывает отдельную приёмку. Если QA должен
написать тесты, он получает явно настроенную writable стадию; её результат
проходит обычный Git proposal/inspection. Обнаружение тестовых фреймворков и
неявный запуск «всех тестов» не добавляются.

Сценарий `qa_test_development_creates_new_candidate_requiring_new_independent_review`
проверяет эту writable-ветку: implementation → независимое ревью → явная задача
QA дописать тесты → повторное независимое ревью той же Task. Он создаёт два
настоящих commit в приватной Git-копии; зарегистрированный source не меняется.
Вторая ревизия принимается только после подтверждения остановки writer и проверки
чистого HEAD. Старый review остаётся привязан к первой ревизии, повторный outcome
со старым SHA отклоняется, а оба writer исключаются из выбора reviewer.
Это PG/Core/Gateway acceptance с управляемым Supervisor и реальным GitBackend,
не запуск модели или контейнера. Тестовый файл создаётся по явному заданию QA;
сценарий не утверждает, что этот файл исполнялся тестовым фреймворком.

Оператор читает оценки через
`GET /v1/projects/{project_id}/tasks/{task_id}/reviews?limit=50&after={cursor}`.
Порядок страницы — descending record ID; авторитетное время остаётся полем
`recorded_at`. Каждая оценка также присутствует в каноническом журнале Task.

Объём этой policy пока ограничен Git candidate и Employee read-only assessment.
Human/machine acceptors и hooks — отдельные контракты, не неявные режимы этой
настройки. Результаты live LLM и полной Integration-приёмки учитываются отдельно
от синтетических Core/UDS/gRPC tests.
