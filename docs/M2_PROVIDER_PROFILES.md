# M2: four explicit provider profiles

M2 selects four lanes. They share `RuntimeBinding`, the common Run ledger,
Supervisor isolation and Forge Gateway. Native authentication differs; it is
not copied from an arbitrary host configuration or silently exchanged for an
API key.

| Template lane | Adapter / pinned version | Provider | Credential kind | Delivery |
| --- | --- | --- | --- | --- |
| `codex_cli` | `codex_cli` / `0.153.2` | `openai` | `codex_chatgpt` | isolated subscription snapshot |
| `claude_cli` | `claude_code_cli` / `2.1.263` | `anthropic` | `claude_subscription` | isolated setup-token |
| `openrouter_api` | `opencode_runtime` / `1.18.29` | `openrouter` | `api_key` | restricted per-Run proxy key |
| `openai_api` | `opencode_runtime` / `1.18.29` | `openai` | `api_key` | restricted per-Run proxy key |

API lanes require the separately configured local LiteLLM Provider Gateway.
The upstream API key stays outside the worker container. A native subscription
credential is necessarily exposed inside its own sandbox; it is not exposed to
another Run or copied back into the operator's original auth file.

Cursor, Gemini and **Grok Build CLI** remain deferred. No route is selected just
because another provider is unavailable. Model selection is explicit; the
template does not choose a default model, fetch a catalog or make a paid probe.

## Offline template, then an audited command

`forge-cli profile-template --payload JSON` prints the payload for
`configure_employee_runtime`. It does not contact Core, open credential files,
enroll a credential or start execution. It reuses the actual adapter capability
declarations and domain validation instead of maintaining another version list.
The CLI's domain/provider dependencies exist only for this offline composition;
it does not call Core as a library.

Example input shape; replace identifiers, image digest, model and prompts:

```json
{
  "lane": "codex_cli",
  "project_id": "PROJECT_UUIDV7",
  "employee_id": "EMPLOYEE_UUIDV7",
  "binding_id": "ENROLLED_BINDING_UUIDV7",
  "secret_id": "ENROLLED_SECRET_UUIDV7",
  "account_id": "EXPLICIT_SUBSCRIPTION_ACCOUNT",
  "model": "EXPLICIT_MODEL",
  "image": "localhost/forge-runtime@sha256:EXACT_64_HEX_DIGEST",
  "system_prompt": "Use only the assigned sandbox and scoped Forge tools.",
  "employee_prompt": "Follow the pinned stage instructions. Commit Git work explicitly before proposing a candidate; never invent an accepted result.",
  "limits": {
    "cpu_millis": 2000,
    "memory_bytes": 4294967296,
    "pids": 256,
    "wall_seconds": 600,
    "stop_grace_seconds": 5
  },
  "budget": {
    "max_output_bytes": 16777216,
    "requests_per_minute": 60,
    "tokens_per_minute": 100000,
    "max_spend_microusd": null
  },
  "live_input": true,
  "access": "read_write"
}
```

API templates omit `account_id` or set it to null. Codex uses the enrolled
snapshot's account identity. Claude uses an explicit stable operator account
label for subscription capacity. An API lane should use an explicit monetary
budget; observed spend can overshoot while requests are in flight. Native
subscription monetary cost is unknown, not zero. All lanes retain the local
wall/output/resource limits across subsequent input turns.

The resulting profile is a fresh immutable UUID/revision. Generate it once and
reuse the resulting payload and idempotency key when retrying its command. Do
not regenerate a different profile under an old retry key. Existing Runs keep
their pinned configuration after a later Employee reconfiguration.

The template uses `filesystem_sandbox`. A Git Task overrides source and base
through its own repository binding; the provider does not select a repository.
Read-only review copies are derived by Core from an accepted candidate.

`live_input: false` preserves the one-turn lane. Enabling it opts into the pinned
driver's native session input, not resuming a process after a crash. See
[native input](M2_NATIVE_INPUT.md) for receipt and budget semantics.

## Proof boundaries

CLI unit tests validate all four templates. The `m2_provider_profiles` test uses
synthetic credentials and an isolated PostgreSQL schema to exercise actual
enrollment/registration and immutable profile rejection. It starts no Supervisor
and cannot establish authenticated provider behavior.

Pinned image probes, native driver fixtures and paid live runs are separate
gates. A missing key or an unrun live check remains unverified. The execution
ledger, not this list of supported template names, records current acceptance.

For an explicitly approved real check, see the [live gate](M2_LIVE_GATE.md).
