# Claude Code adapter

This crate targets Claude Code `2.1.263`, subscription authentication through an
explicitly enrolled `claude setup-token`, and `cli_wrapper` transport. The host
version/help were inspected without reading credentials or calling a model.
The JSONL fixture is synthetic contract evidence, **not a captured live run**.

`ClaudeAdapter::prepare` returns a provider-neutral invocation. It uses a fresh
`CLAUDE_CONFIG_DIR`, explicit settings, Forge-only MCP, and stream-json stdin.
The outer rootless container remains the isolation boundary; Bash is available
inside it. Native project hooks, auto-memory, plugins, and implicit settings are
disabled; project hook execution belongs to Forge's configured assignments.

The container-local `forge-claude-driver` replaces itself with the pinned Claude
binary at `/usr/local/bin/claude`. It reads only the owner-only managed token file,
then exposes the token to that process through `CLAUDE_CODE_OAUTH_TOKEN`. Secrets
are absent from invocation args, managed config, serialized state and diagnostics.
This is explicitly `isolated_runtime_secret`, not secret-blind execution.
The setup token has no refresh/writeback. Rotation requires a newly enrolled
credential and a new immutable profile reference; existing Runs keep their pin.

`encode_user_message` supports subsequent input on the same open stdin pipe.
`parse_jsonl_event` separates UUID replay receipts from runtime observations.
A replay means only runtime receipt, never Employee acknowledgment. The driver
must check the pinned session and outstanding message IDs, deduplicate events,
and preserve pending messages on exit. Result usage includes cache reads/writes
in normalized input tokens; unknown counters stay unknown. Subscription dollar
amounts are not interpreted as billable cost. Raw tool/error/reasoning bodies are
not normalized, and a completed turn never accepts a Task outcome.

## Runtime integration

Core enrolls the distinct `claude_subscription` credential, validates static
profile capabilities, and prepares one stdin message with stable per-Run IDs.
Supervisor probes the actual pinned CLI before attaching credentials and starts
the driver in the selected Task surface. The static-input v2 path ends stdin
after the initial message; it does not support duplex Inbox delivery or resume.
Core verifies the reported session ID, accepts at most one terminal result and
records usage even for interrupted/failed results. Turn success never accepts work.

Synthetic Core tests cover two private Runs, credential-kind/project isolation,
secret rejection and quiescent cleanup. Credential-free image probes verify the
installed binaries. Authenticated subscription, Forge MCP use, model execution,
and native live stop still require explicit opt-in acceptance. No host home or
API-key fallback is supported. See [the operator guide](../../docs/M2_CLAUDE_RUNTIME.md).

Checks: `cargo test -p forge-provider-claude --locked` and
`cargo clippy -p forge-provider-claude --all-targets --locked -- -D warnings`.

Protocol/settings references:
[CLI](https://code.claude.com/docs/en/cli-reference),
[authentication](https://code.claude.com/docs/en/authentication),
[settings](https://code.claude.com/docs/en/settings),
[environment](https://code.claude.com/docs/en/env-vars), and
[network](https://code.claude.com/docs/en/network-config), and
[official SDK message types](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/types.py).
