# Runtime image preflight

Supervisor checks the actual image's CLI version before reading the Run's private
invocation, preparing its WorkSurface, or mounting credentials into a container.
An immutable image reference alone does not prove an adapter is installed.

The M1 check accepts exactly `codex-cli 0.153.2` for `codex_cli`, and `1.18.29`
for `opencode_runtime`. A different version, missing executable, malformed or
oversized stdout, failed exit, or timeout rejects provisioning with
`runtime_image_preflight_failed`. It does not change provider/model, pull another
image, fall back to host execution, or start the employee.

The probe uses a separate uniquely named rootless Podman container:

- the same digest-pinned image, with `--pull=never`;
- no network, credentials, host mounts or inherited host proxy variables;
- read-only root, dropped capabilities, a private temporary directory;
- explicit CPU, memory and process limits;
- a 15-second container deadline plus a 15-second client deadline;
- at most 4096 stdout bytes; stderr is discarded and output is never logged;
- automatic removal and bounded targeted cleanup on failure.

The client deadline may be followed by up to six seconds of kill/cleanup work.
Podman's own container deadline also applies if the Supervisor probe future is
interrupted. Probe containers never receive a Task WorkSurface or a Run grant.
Actual Run containers also disable Podman's implicit host proxy-env propagation;
their proxy configuration comes only from the managed invocation.

The synthetic TASK-12 fixture advertises the expected Codex version to exercise
the same provisioning path. This is deliberately not evidence that a real Codex
runtime was tested. Version discovery checks compatibility, not image provenance:
production profiles must use reviewed, built image digests.

Validation:

```sh
FORGE_RUNTIME_PROBE_IMAGE='<built image@sha256:digest>' \
  cargo test -p forge-supervisor actual_pinned_image_probes_both_clis_without_credentials -- --ignored
```

On 2026-09-06 the keyless probe passed for both real CLIs in
`localhost/forge-runtime@sha256:d191a0d5dc0144b22a6282da5b9e2c204facdc57531feec7c81f47a72e8a7779`,
and rejected an intentionally wrong expected version. It made no inference call.
Separate backend tests check rejection before surface/private-invocation reads
and writable private tmpfs when a Run has no persistent WorkSurface. On Podman
4.9.3 the tmpfs uses `mode=1777`: `uid`/`gid` mount options are rejected.
