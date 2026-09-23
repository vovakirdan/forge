# Local Control Room: installer handoff

This document records the current build and runtime interface for backend M4.
UI4.1 browser acceptance and M4 clean-host installation have separate evidence.

## Build artifact

From the repository checkout, run `just ui-install` and `just ui-build-live`.
The artifact is the entire `frontend/dist-live/` directory, including
`forge-live-manifest.json`. Build `forge-ui` and `forge-cli` with
`cargo build --locked -p forge-ui -p forge-cli`. The installed host needs those
native binaries and the static artifact; it does not need Bun, Node, Vite, the
mock UI or the TanStack server bundle.

The gateway loads and verifies the manifest before it binds. It checks each
asset path, type and SHA-256 against the files it serves. An incomplete or
modified artifact stops startup. The manifest is an integrity check for an
owner-supplied artifact, not a publisher signature. The frontend build must
contain no credentials or host-specific secrets.

## Runtime boundary

Run `forge-ui serve --assets-dir /absolute/path/to/dist-live` as the same
local owner as Core. Optional `--core-socket` and `--control-socket` select
absolute Unix socket paths. By default, the Core API socket is `api.sock` and
the UI control socket is `ui-control.sock` under Forge's runtime directory:
`$XDG_RUNTIME_DIR/forge`, then `$XDG_STATE_HOME/forge/run`, then
`$HOME/.local/state/forge/run`. The final fallback is `/tmp/forge`. The
owner-only socket directory and peer UID checks are enforced by the services.

The gateway listens on `127.0.0.1` with a kernel-assigned port and prints its
actual origin on startup. No Core management TCP listener is required. The
Core HTTP API remains on its private Unix socket. The gateway must start after
its static artifact is in place; Core can be temporarily unavailable and the
UI reports that condition. Starting or stopping the gateway does not start,
stop or resume Project execution. Restarting the gateway invalidates its
in-memory browser sessions and login codes.

In an owner terminal, run `forge-cli ui login` (or pass
`--control-socket /absolute/path/to/ui-control.sock`). The CLI obtains a
single-use code and prints it with the current origin only to the controlling
terminal. Paste the code into that origin's login form within five minutes.
The browser session lasts up to eight hours, is held in `sessionStorage`, and
is not automatically renewed. Issue a new code after expiry or gateway restart.
The installer must not place a login code or session token in argv, env,
systemd journal, a URL, a file or a generated HTML page.

The gateway serves its own assets and a fixed allowlist of same-origin API
paths. Its Core client uses only the configured private Unix socket. Do not
replace it with a general HTTP proxy, a remote listener or Core's management
router on TCP. CLI and UI use the same canonical Core commands and data; they
may be open together. Closing or reloading a browser tab has no Run lifecycle
effect.

## Operator checks

1. Start the existing local data services and Core through the documented
   development or installed workflow. For development, `just dev-up` starts
   PostgreSQL, NATS, Redis and MinIO without deleting named volumes.
2. Start `forge-ui serve --assets-dir "$PWD/frontend/dist-live"` from the
   repository root. Read the printed loopback origin.
3. Run `forge-cli ui login` in a second owner terminal, open the printed origin
   and enter the code. Verify Project selection and the authenticated UI
   health/read state.
4. Log out, obtain a fresh code and log in again. Repeat after a gateway
   restart to verify that the old session expires. Leave Core running and
   verify that CLI Project reads still work.

For failure diagnosis, inspect gateway startup errors for manifest/socket
problems, Core readiness for a broken private API socket, and the UI's explicit
read or event-connection state. Record status codes, request IDs and canonical
Event/Run IDs. Do not copy authorization headers, login codes, provider tokens,
raw credential files or object-storage keys into evidence. An index outage or
stale derived memory is not a failed canonical Task.

## M4 installation checklist

- Package `dist-live/`, `forge-ui` and `forge-cli` together from one revision.
- Create owner-only state/runtime directories and private socket permissions.
- Start Core and Supervisor according to their existing recovery contract;
  start the UI gateway against the installed artifact and Core socket.
- Expose only the printed loopback origin. Use `forge-cli ui login` for owner
  entry after each gateway restart or session expiry.
- Check installed UI health/login/Project read after clean-host install and
  reboot, without treating that smoke as UI4 feature acceptance.

The installer, systemd units, wizard and clean-host/reboot proof belong to M4.
