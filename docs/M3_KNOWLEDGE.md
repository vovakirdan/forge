# M3: canonical knowledge

Forge хранит Markdown pages и derived memory в PostgreSQL. AgentMemory получает
сменную retrieval projection. Недоступность индекса не откатывает принятую
команду и не блокирует управление проектом.

## Pages и публикация

`KnowledgePageKind`: `introduction`, `architecture`, `guide`, `policy`, `decision`.
Только опубликованные `policy` и `decision` имеют authority. Draft и withdrawn
не входят в обязательный слой контекста.

Все mutations проходят `POST /v1/commands/{name}` с обычными Project
`expected_revision` и idempotency key. Авторизованный local operator — Human;
Employee, Supervisor и semantic SystemJob не получают management route.
Домен допускает SystemManager только за trusted capability boundary; название
роли или `actor` в JSON не даёт таких прав.

| Command | Payload | Результат |
| --- | --- | --- |
| `author_knowledge_page` | `page_id`, `expected_page_revision`, `kind`, `title`, `markdown`, `source_refs` | `0` создаёт draft; положительная revision меняет только существующий draft того же kind |
| `publish_knowledge_page` | `page_id`, `expected_page_revision` | Draft становится published |
| `supersede_knowledge_page` | `page_id`, `expected_page_revision`, `title`, `markdown`, `source_refs` | Новая опубликованная revision того же page; прежняя остаётся в истории |
| `withdraw_knowledge_page` | `page_id`, `expected_page_revision` | Page перестаёт действовать; история сохраняется |

Каждая операция увеличивает page revision и Project revision. Snapshot, immutable
history, Event, outbox, projection intent и receipt фиксируются одной транзакцией.
Replay возвращает прежние Event IDs и не создаёт другую projection identity.
Published page редактируется через явный `supersede`, поэтому промежуточный draft
не снимает действующее правило. Withdrawn page не переиздаётся неявно.

Markdown ограничен 65536 UTF-8 bytes, title — 200 bytes, source refs — 128.
Большие материалы остаются Artifacts. Допустимые ссылки: `artifact` с
`artifact_id`, `event` с `event_id`, `task_handoff` с `handoff_id` и
`knowledge_page` с `page_id` и `revision`. Core проверяет существование и Project
scope источников. Human-authored page может не иметь внешних source refs:
`actor` и `command_id` её авторства обязательны. Derived entry без evidence
не принимается. При withdrawal прежние ссылки остаются историческими и не
мешают снять правило, если их источник уже withdrawn.

## Чтение и CLI

Owner-local API возвращает canonical content, а не текст из индекса:

- `GET /v1/projects/{project_id}/knowledge?after={page_id}&limit=50`
- `GET /v1/projects/{project_id}/knowledge/{page_id}`
- `GET /v1/projects/{project_id}/knowledge/{page_id}/history?after={revision}&limit=50`

List responses содержат `items` и `next_cursor`. Limit от 1 до 100. History
использует numeric revision cursor; page list — UUID cursor. Чужой Project
не может прочитать page через свой scope.

Существующий CLI использует эти же endpoints:

```sh
forge command author_knowledge_page \
  --project-id PROJECT_UUID --expected-revision 1 \
  --idempotency-key RESERVED_RETRY_KEY \
  --payload '{"page_id":"PAGE_UUID","expected_page_revision":0,"kind":"policy","title":"Review","markdown":"Require independent review.","source_refs":[]}'

forge get /v1/projects/PROJECT_UUID/knowledge/PAGE_UUID/history
```

`PROJECT_UUID` и `PAGE_UUID` должны быть заранее выбранными UUIDv7. При retry
сохраняются payload, Project revision и idempotency key.

## Derived records и projection

`DerivedMemoryEntry.subject` различает `task_summary`, `employee_memory_entry`
и `project_knowledge_entry`. Personal scope содержит Employee ID; у summary и
project entry scope общий. Ни один derived record не имеет флага accepted или
authority. Запись содержит source refs, SHA-256 Markdown, revision, generating
job ID и optional точный диапазон canonical Event sequence. Onboarding note
может не иметь Task ID; Task summary обязан его иметь.

Core принимает derived output только после проверки job lease/generation,
его frozen source allowlist и target scope. Persistence helper отдельно
проверяет canonical sources, hash и Event coverage. Existing revision принимает
только точный replay; изменение создаёт следующую revision. История append-only,
а withdrawal немедленно убирает entry из canonical retrieval.

Projection operation получает один UUIDv7 при canonical commit; retries и
explicit rebuild сохраняют его. Индекс подтверждается отдельно после strict
durable/index-ready acknowledgement. Ошибки сохраняются только как bounded
codes; повторное чтение pending batch имеет короткую задержку после отказа.
Ни данные AgentMemory, ни его scope metadata не заменяют canonical ACL и
проверку current revision перед выдачей контекста.

## Проверка

`cargo test --locked -p forge-domain knowledge --lib` проверяет publication
state, authority, scope и bounded input. Parser tests запрещают подменять
operation или actor в payload.

`FORGE_INTEGRATION=1 cargo test --locked -p forge-testkit --test m3_knowledge --
--ignored --test-threads=1` использует отдельные schemas в настроенном local
PostgreSQL и существующий NATS. Проверяются named commands, history, receipts,
outbox, rollback при injected receipt failure, source scope и personal memory.
Тесты не запускают модель и не мигрируют shared development schema.
