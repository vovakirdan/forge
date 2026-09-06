# Sandbox runtime: первая Linux-реализация

**Дата:** 5 сентября 2026

**Область:** TASK-12; дальнейшие provider/evidence gates остаются отдельными.

## Execution boundary

RunSpec v2 задаёт immutable profile, digest-pinned image, Task-owned surface и
ресурсные ограничения. Supervisor использует только rootless Podman с cgroup v2.
При ошибке нет host-process fallback. Исторический fake RunSpec v1 остаётся
отдельным детерминированным executor.

Контейнер получает read-only rootfs, `network=none`, dropped capabilities,
`no-new-privileges`, explicit UID/GID, CPU/memory/PID limits и ограниченный scratch
tmpfs. Host home, container socket и административный Core UDS не монтируются.
Shell и Git остаются доступны внутри выданной среды.

Container name детерминирован по run/fence/epoch; labels содержат host, boot и
surface. Identity сохраняется до запуска. Monitor использует `inspect`, а не
наличие gRPC-соединения, как свидетельство жизни среды. Supervisor shutdown
отсоединяет real monitors, не останавливая контейнеры. После restart journal
становится `unknown`, затем фактический running container принимается под
наблюдение без нового `start`.

Новый writer не допускается при `active` или `unknown` journal identity с тем же
surface ID. Canonical Lease/reservation дополнительно контролируется Core.
Elapsed wall limit считается от Podman `StartedAt`, а не от перезапуска monitor.
Graceful stop посылает SIGINT, затем применяется SIGKILL и ожидание фактического
exit. Exit code сам по себе не отправляет Pipeline outcome.

Transient inspect failure не удаляет monitor или RunControl. Unknown-среда
продолжает проверяться; stop действует по ранее проверенному immutable container
ID, даже если inspect временно сломан. Переполнение journal не отключает физический
stop: terminal observation повторяется до durable записи. Stop, полученный до
start checkpoint, запрещает запуск employee, включая остановку во время provision.

## TaskWorkSurface

- `none`: ограниченный ephemeral `/workspace` tmpfs.
- `filesystem_sandbox`: отдельное persistent дерево Task.
- `git_worktree`: Task-private bare clone и linked worktree, без общих writable
  metadata/hardlinks с source repository. Git links относительные и переживают
  перенос дерева в container mount.
- Read-only access использует отдельную копию после quiescence прежнего writer.
  Symlinks копируются как ссылки, не разыменовываются хостом.

Source selection берётся только из operator-owned binding. Подготовка Git не
меняет основной checkout, не выполняет merge/push и отключает hooks/global config.
Частичные workspace и завершённые containers автоматически не удаляются.
Host-owned manifest находится отдельно: `surface-manifests/<surface-id>.json`;
он не монтируется в Run и читается с лимитом 16 KiB без symlink/hardlink fallback.
Старое дерево без такого manifest не принимается автоматически: его writable
`surface.json` не является доказательством ownership. Данные сохраняются для
явного восстановления, а не удаляются или переинициализируются.

## Private materialization

Core создаёт owner-only grants:
`<grants-directory>/<run-id>/<epoch>/invocation.json`, `stdin` и нужный secret file.
`RunnerInvocation` содержит только program/argv, несекретный env, managed file
descriptors, source/target credential paths и wrapper limits. Prompt и credential
bytes в этот JSON или RunSpec не входят.

| Host source | Container path | Режим |
|---|---|---|
| Run grant directory | `/run/forge-input` | read-only |
| Codex `auth.json` | `/run/forge-secrets/auth.json` | read-only |
| LiteLLM Run `api-key` | `/run/forge-secrets/api-key` | read-only |
| Per-Run runtime directory | `/run/forge` | private writable |
| Per-Run evidence directory | `/run/forge-evidence` | writable |
| `<gateways-directory>/<run-id>` | `/run/forge-gateway` | private Run directory, read-only |

Gateway directory содержит только Run-scoped socket. Core сохраняет inode
директории при restart и заменяет socket entry; это позволяет surviving Run
переподключиться. Mount самого socket inode сделал бы восстановление невозможным.

Codex обновляет private `/run/forge/codex-home/auth.json`; он не является raw-log
или artifact. Core обязан сначала принять/encrypt auth writeback либо сохранить
encrypted conflict material, затем выполнять explicit plaintext cleanup.

Настройки binary: `--state-directory`, `--grants-directory`,
`--gateways-directory`. Grants/Gateway defaults — соответствующие подкаталоги
persistent state. Unix socket path ограничен ОС; при длинном state path нужно
явно выбрать короткий owner-only `--gateways-directory` в runtime storage.
`--evidence-max-bytes` (default 2 GiB) ограничивает admission по сохранённым raw
файлам и полным output reservations active/unknown Runs. При исчерпании новые
Runs не запускаются, старые evidence не удаляются. Это retention/admission budget,
не filesystem quota: произвольные прямые записи shell в writable mounts требуют
отдельного quota boundary и не ограничиваются stdout/stderr wrapper budget.

## Wrapper и общий transport

`forge-runner` находится внутри pinned image и не работает в host mode. Он
материализует private config/auth, очищает environment, подаёт prompt через stdin,
запускает child process group и сохраняет redacted stdout/stderr и exit evidence.
Строки ограничены 1 MiB, общий output — максимум 256 MiB на Run. Переполнение
помечается `output_incomplete` и вызывает управляемый stop; чтение из pipe
продолжается. Reasoning/thinking JSON-события не сохраняются raw fallback.
Перед сохранением каждой строки redactor перечитывает bounded auth snapshots,
включая обновлённые opaque refresh tokens. Credential-shaped записи отбрасываются;
небезопасный/недоступный auth snapshot вызывает incomplete evidence и stop.
Это не обнаружение произвольного кодирования или намеренной обфускации секрета.

Loopback endpoints `127.0.0.1:4097` и `:4098` передают HTTP/MCP и CONNECT в один
per-Run scoped UDS, максимум 32 одновременных соединения. Wrapper не принимает
решения о project/task/provider permissions. Core Gateway проверяет scope на
каждом запросе; CONNECT требует отдельной provider allowlist policy.

API lane использует `forge-opencode-driver`: он управляет OpenCode server,
session/SSE и API abort. Native lane использует Codex CLI. Wrapper не объявляет
их conformance только по наличию executable.

## Проверки

Детерминированные tests покрывают replay, surface exclusion, Git relocation,
read-only snapshot и фильтрацию reasoning/known secrets. Explicit Podman tests:

```sh
cargo build --locked -p forge-supervisor --bin forge-runner
podman build -f infra/runtime/Containerfile.fixture -t localhost/forge-runner-fixture:m1 .
cargo test --locked -p forge-supervisor podman_ -- --ignored
```

Fixture содержит fake executable в реальном Ubuntu/Podman environment. Это
доказательство sandbox/restart/evidence boundary, не авторизации, inference или
provider conformance. Реальные Codex/OpenCode images и live gates — TASK-18–19.
Regression gates также покрывают replacement Gateway socket у surviving Run,
stop при сломанном inspect/full journal и отказ от запуска после pending stop.
