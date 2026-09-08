# M2: explicitly approved live provider smoke

This gate starts **two concurrent real Runs of one Employee** for one selected
provider lane, delivers an addressed native input to only one Run, and requests
managed stop. It consumes subscription quota or API credits. It is not invoked
by `just test-m2`, `just test-unit` or `just test-integration`.

Use `codex_cli`, `claude_cli`, `openrouter_api` or `openai_api`. No lane, model,
account or credential file is discovered automatically. Each lane must be run
separately before claiming its authenticated live acceptance.

Build the current runtime image with `just build-runtime` and select its
printed digest. Start the development services described in the README. Create
a private settings JSON file using your editor, with permissions `0600`. The
credential file and, for API lanes, proxy master file must also be owner-only
regular files. Put paths in settings, never key contents.

Example subscription settings (all placeholders require explicit replacement):

```json
{
  "lane": "codex_cli",
  "credential_file": "/absolute/private/codex-auth.json",
  "account_id": "EXPLICIT_ACCOUNT_ID",
  "model": "EXPLICIT_MODEL",
  "image": "localhost/forge-runtime@sha256:EXACT_64_HEX_DIGEST",
  "limits": {
    "cpu_millis": 2000,
    "memory_bytes": 4294967296,
    "pids": 256,
    "wall_seconds": 180,
    "stop_grace_seconds": 5
  },
  "budget": {
    "max_output_bytes": 4194304,
    "requests_per_minute": 20,
    "tokens_per_minute": 20000,
    "max_spend_microusd": null
  }
}
```

For Claude, use `claude_cli`, an explicitly selected subscription setup-token
file, and a stable account label; see [Claude setup](M2_CLAUDE_RUNTIME.md).
For API lanes, omit `account_id`, select the upstream API key file, set a
positive `max_spend_microusd`, and add:

```json
{
  "proxy_endpoint": "http://127.0.0.1:4000",
  "proxy_master_file": "/absolute/private/forge-proxy-master"
}
```

These fields belong in the same settings object. The local LiteLLM gateway
must already be configured for the explicitly selected upstream model. The
harness neither installs a proxy nor chooses a fallback provider.

Only after approving real usage, run from the Forge checkout:

```sh
FORGE_M2_LIVE=1 \
FORGE_M2_LIVE_SETTINGS=/absolute/private/m2-live.json \
just test-m2-live
```

The harness requires wall limits of 60–240 seconds, output at most 8 MiB, and
an API budget at most five USD **per Run**, not per test. Choose a smaller cap
appropriate for your model. In-flight usage can overshoot a budget; subscription
monetary cost remains unknown, not zero.

Run this gate alone and allow cleanup to finish. It retains private session
evidence, checks both managed state and its own container inventory, and never
rewrites the source auth file. It uses empty sandbox directories, not your Git
repository. Its stop leaves Tasks waiting; it does not establish delivery,
review, QA or integration correctness. The separate isolated-Git acceptance
suite covers that workflow without paid inference.

An ignored test, a synthetic driver fixture or a successful registration is
not live evidence. Record the selected lane and actual result when you run it.
