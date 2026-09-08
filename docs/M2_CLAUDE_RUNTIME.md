# M2: Claude Code subscription runtime

Срез подключает `claude_code_cli` версии `2.1.263` к существующему изолированному
stage-run. Он сохраняет Codex CLI и OpenCode API lanes. Это одноразовый запуск:
Core передаёт один контекст, stdin закрывается, Claude выполняет query loop.
Taskless one-shot разговоры используют тот же adapter через
[Communication assignment](M2_COMMUNICATION_RUNS.md). Для Claude доступен отдельный
opt-in [native input contract](M2_NATIVE_INPUT.md); provider-session resume не реализован.

## Enrollment и профиль

Оператор явно получает subscription token через `claude setup-token` и сохраняет
его в отдельный обычный файл mode `0600`. Forge не запускает login, не читает
host `~/.claude`, не монтирует домашний каталог и не пишет обновления авторизации
обратно. Не передавайте token в аргументах, HTTP body, prompt или отчёте.

Named command `enroll_credential` принимает такой payload в стандартном command
envelope проекта:

```json
{
  "secret_id": "<new UUID>",
  "binding_id": "<new UUID>",
  "kind": "claude_subscription",
  "source_file": "/absolute/private/path/to/setup-token"
}
```

Источник читает Core на локальном хосте. Проверка формы отделяет setup-token от
API key, но не доказывает действительность токена или доступность аккаунта.
Core убирает конечный перенос строки и сохраняет зашифрованный secret с отдельным
purpose. Исходный файл оператора Forge не удаляет. Для ротации нужны новый
`secret_id`/binding и новая версия профиля; текущие Runs сохраняют старый pin.

В `configure_employee_runtime` укажите существующий Employee и `RuntimeBinding`:

| Поле ExecutionProfile | Значение |
| --- | --- |
| `adapter_id`, `adapter_version` | `claude_code_cli`, `2.1.263` |
| `provider_id`, `model` | `anthropic`, явно выбранная модель |
| `credential_delivery` | `isolated_runtime_secret` |
| `credential_binding` | IDs enrollment, тот же Project, непустой `account_id`, разрешённый delivery mode |
| `capability_profile.transport_engine` | `cli_wrapper` |
| `capability_profile.capabilities` | `structured_events`, `model_selection`, `usage_reporting`, `controlled_stop`, `native_mcp`; опционально `live_input` |
| `capability_profile.credential_exposed_to_run` | `true` |

Capability snapshot также повторяет adapter ID и version. `account_id` — явно
выбранная оператором метка аккаунта, не извлечённая или проверенная идентичность.
API-key credential для этого adapter отвергается до dispatch. `session_resume`
также отвергается; незаявленные возможности не включаются автоматически.
Остальные поля binding — image digest, surface/access, limits, budget и prompts —
совпадают с [M1 runtime contract](M1_RUNTIME.md#credentials-и-профили).

## Граница исполнения

Перед выдачей credentials Supervisor проверяет `claude --version` в отдельном
контейнере без сети и host mounts. Каждый Run имеет собственные runtime directory
и surface, ограниченные CPU/RAM/PID, wall timeout и output budget. Bash остаётся
доступным внутри sandbox. Набор компиляторов и тестовых утилит зависит от image.

`forge-claude-driver` читает только `/run/forge/claude-home/setup-token` из
owner-only каталога и заменяет себя Claude process с `CLAUDE_CODE_OAUTH_TOKEN`.
Token доступен процессу и его shell: это объявленный isolated-runtime-secret
риск, не secret-blind режим. Host home и общий writable auth не используются.
Подписочный token не обновляется и не получает OAuth writeback.

Forge передаёт собственные settings и только свой MCP server. Project/host
settings, hooks, plugins, auto-memory, cloud MCP connectors и background tasks
отключены. Нативный provider egress разрешает только TLS CONNECT к
`api.anthropic.com:443` и `claude.ai:443`, с проверкой публичного IP и действующего
Run scope. OpenAI hosts доступны только Codex lane. Arbitrary web, package
registries и host network не открываются.

После подтверждённой остановки и безопасного импорта двух evidence streams Core
удаляет только временные delivery-копии token и stdin. Encrypted credential,
source оператора, surface и технические evidence сохраняются. При неполных
evidence секреты остаются в приватном runtime до безопасной очистки.

Core проверяет session correlation и один terminal result. Usage сохраняется
отдельно от success, включая failure/interruption; cache reads/writes входят в
input counter. Неизвестный usage остаётся неизвестным, дублирующий result не
удваивает счётчик. Subscription cost не выводится из условного USD поля Claude.
API budget fields не превращаются в жёсткий подписочный token/cost лимит.
Окончание query loop не закрывает Task: требуется явный Forge outcome и Artifact.

## Проверки и оставшиеся gates

```sh
cargo test -p forge-provider-claude --locked
cargo test -p forge-core --lib --locked
cargo test -p forge-supervisor --lib --locked -- --test-threads=1
just build-runtime
bash scripts/tests/runtime-build-context-test.sh
```

`just build-runtime` устанавливает pinned Claude только в image и проверяет
версии Node/Codex/OpenCode/Claude без credentials и inference. Default local tag
остаётся `localhost/forge-runtime:m1` для совместимости launcher; Run использует
напечатанный digest. Build context пропускает девять конкретных файлов.

`just test-integration` включает `m2_claude_lane`: синтетический setup-token,
реальные Core/PostgreSQL/Gateway, два Run и ручной Supervisor. Проверяются
отказ от API-key подмены, cross-project доступ, неподдержанные capabilities,
разделение surfaces, секреты в Gateway и очистка после quiescence. Provider
процесс в этом тесте не запускается. JSONL fixtures также синтетические.

Отдельный разрешённый live gate ещё нужен для subscription authentication,
выбранной модели, реального Forge MCP вызова, artifact submission и stop.
Native input проверен офлайн и на синтетическом Core/UDS contract, но требует
отдельного разрешённого live gate. Resume не реализован. Ни unit tests, ни версия
установленного CLI не подтверждают subscription authentication.

Основания конфигурации: [официальный CLI contract](https://code.claude.com/docs/en/cli-reference),
[authentication](https://code.claude.com/docs/en/authentication),
[settings](https://code.claude.com/docs/en/settings),
[environment](https://code.claude.com/docs/en/env-vars),
[network](https://code.claude.com/docs/en/network-config).
