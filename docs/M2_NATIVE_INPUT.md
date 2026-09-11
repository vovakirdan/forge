# M2: bounded native input inside one Run

The pinned profiles may explicitly request `live_input`. Existing one-shot
profiles and their serialized invocation bytes remain unchanged.

| Adapter | Native transport | Acceptance evidence |
| --- | --- | --- |
| Claude Code `2.1.263` | One stream-json process | Matching native user-message UUID and session echo |
| Codex `0.153.2` | One app-server stdio thread | Successful correlated `turn/start` response and native turn ID |
| OpenCode `1.18.29` | One private HTTP/SSE session | Exact persisted native user-message ID, via SSE or bounded GET |

OpenCode's OpenAI/OpenRouter API lanes still use the existing Provider Gateway;
this adds no direct upstream-key transport. Only Claude uses the runner's stdin
pump. Codex and OpenCode use separate container-local drivers and the same durable
input mailbox. No Task transitions or provider-specific Task rules live there.

## Delivery and authority

An addressed `task_execution` or `exact_run` Inbox message remains the canonical
source. Core validates its Project, recipient, immutable Task visit and, where
specified, exact Run/fencing token/environment epoch. An unrelated general Inbox
message cannot enter a Task Run. The Employee's own replies are output, not input.

Canonical Inbox identity and transport identity have different consumers:

| Identity | Consumer |
| --- | --- |
| Top-level `id` in `source_message_json` | Model-facing message reference; `inbox.acknowledge` and `inbox.reply` |
| `DeliverRuntimeInput.command_id` | Durable delivery intent, native transport correlation, events and receipts |

The addressed wrapper renders the canonical Inbox ID, not the delivery command
ID. It parses that top-level field as a UUIDv7; missing, malformed or duplicate
`id` fields fail closed with a generic error that excludes the source payload.
This is a small rendering-boundary check, not a second implementation of Core's
Inbox authorization or complete Message validation. Transport IDs and receipt
correlation remain unchanged.

A message already visible in the bootstrap ContextSnapshot can still arrive
through at-least-once native delivery. Both presentations refer to the same
canonical Inbox message. Correct rendering does not synthesize an Employee ACK,
suppress later deliveries, deduplicate model turns or change Inbox policy.

Under the Project mutation lock, Core creates an append-only input intent with a
stable UUID, scope, sequence and full source reference. Migration `0026` stores
intents and observations separately. Each reconciliation admits at most 128
outstanding messages; SQL excludes already scoped sources, so later pages do not
starve behind retained history. The final delivery rechecks authority under the
same Project lock with a bounded transport wait. Stop delivery has priority.

Supervisor journals the input before materializing its private, read-only mailbox
copy. Same ID and bytes replay; conflicting bytes fail closed. Neither that file
nor a successful stdin write creates `runtime_accepted`. The existing sandbox
driver feeds the source only after the previous turn finishes. The fixed wrapper
prevents source text becoming a CLI command. A successful OpenCode HTTP 204 alone
does not prove acceptance; neither does a successful Codex JSON-RPC write.

A native observation must match the expected input UUID and pinned session. Only then
does the driver emit a normalized receipt for Supervisor's durable replay. Core
validates its frozen exact scope again, including after a late receipt arrives
following closure. This is transport acceptance, **not** an
Employee ACK, a semantic check, or proof that the instruction was followed.
`inbox.acknowledge` / `inbox.reply` remain separate authenticated Gateway actions;
an instruction barrier is not satisfied by the native receipt.

## Completion, failure and budgets

`CloseAfterTurn` is a separate, durable, typed control intent. It is not model
text or a fixed idle timeout. Assignment completion or authority loss requests
input closure after the current turn; close dominates queued messages. Inputs still
unobserved at terminal close receive explicit `delivery_unknown`, never invented
acceptance. A later correlated native observation is appended without deleting
that uncertainty record; the projection prefers confirmed native acceptance.
Reusing a receipt ID with different facts is rejected. General graceful/force
stop remains independent of this channel.

One Run keeps one process/session, wall deadline and output budget across turns.
Claude's per-turn `result.usage` values are summed with overflow checks; cumulative
monetary/model totals are not summed again. Codex uses native thread-wide token
totals; OpenCode accumulates each unique completed assistant message once across
the session. Core checks monotonically increasing cumulative snapshots, exact
accepted inputs and session/turn correlation instead of summing those snapshots.
Unknown, regressing or uncorrelated usage stays unknown. Subscription mode does
not promise a hard token/cost cutoff. Malformed or unexpected native ordering
fails closed rather than fabricating completion or reusing another turn's usage.

Driver restart cannot infer native acceptance from retained files or repeat an
uncertain transcript: a private started marker fails closed. This slice provides
at-least-once transport and correlated receipts, not provider-session resume or
exactly-once model execution. Canonical messages and receipts survive cleanup of
acknowledged temporary mailbox/receipt copies; journal hashes retain replay
identity. Evidence files use the existing sandbox trust boundary, not independent
cryptographic attestation of a provider.

Taskless Communication keeps source-bound one-shot assignment semantics. A live
profile may wait for its typed completion close, but later general Inbox
messages still become separately admitted Communication assignments.
Resolution v4 is deliberately one-shot even when its Employee's pinned profile
also advertises `live_input`; it receives no native message/close channel.

## Verification boundary

The canonical-ID regression requires Codex's bootstrap READY → native READY
redelivery → GO → typed close sequence, plus OpenCode coverage separating the
canonical message ID from native transport IDs. Malformed, missing and duplicate
source IDs must produce payload-free errors. These are offline regression
requirements; the latest fix's verification status is recorded in the
[live-workflow notes](2026-09-09-m2-live-workflow-specs.md#исправление-после-второго-live-запуска).

Offline tests cover queueing while busy, matching native echoes, restart refusal,
durable journal replay, legacy capability rejection, more than 200 sequential
inputs, and cumulative usage. A PostgreSQL + real UDS test covers canonical input,
stale fences, duplicate receipt, independent Employee ACK, bounded source paging,
Project stop and unknown pending delivery. It uses a manual Supervisor and no
provider authentication or inference. Codex's synthetic stdio and OpenCode's
local HTTP/SSE fixtures cover two turns, busy queueing, native IDs, cumulative
usage, typed close and independent abort. A credential-free, network-disabled
probe of pinned Codex `0.153.2` confirmed initialize/thread-start response fields;
its allowlisted result is retained in `app-server-0.153.2-thread.json`. No model
turn was requested. Pinned OpenCode `/doc` confirmed message-ID request/lookup
schemas. These tests are not paid live-provider evidence.

```sh
cargo test --locked -p forge-provider-claude
cargo test --locked -p forge-supervisor --lib runtime_input
cargo test --locked -p forge-supervisor --lib runner::duplex
cargo test --locked -p forge-core --lib claude_live
cargo test --locked -p forge-provider-codex --test native_session
cargo test --locked -p forge-provider-opencode --test native_session
cargo test --locked -p forge-core --lib native_live
```

Protocol bases: [streaming input](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode),
[CLI replay flags](https://code.claude.com/docs/en/cli-reference), and
[per-turn versus cumulative usage](https://code.claude.com/docs/en/agent-sdk/cost-tracking#track-costs-in-streaming-input-mode).
Codex uses the maintained [app-server protocol](https://learn.chatgpt.com/docs/app-server),
checked against the pinned binary's generated JSON schema. OpenCode uses its
[server API](https://opencode.ai/docs/server/) and the pinned
[v1.18.29 generated SDK types](https://raw.githubusercontent.com/anomalyco/opencode/v1.18.29/packages/sdk/js/src/v2/gen/types.gen.ts).
