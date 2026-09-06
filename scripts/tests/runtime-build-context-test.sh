#!/usr/bin/env bash
# Exercise Podman's ignore semantics with synthetic files, never the repo context.
set -euo pipefail
umask 077

readonly TEST_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPO_ROOT="$(CDPATH= cd -- "$TEST_DIR/../.." && pwd -P)"
(($# <= 1)) || { printf 'Usage: bash %s [ignorefile]\n' "$0" >&2; exit 2; }
readonly IGNORE_FILE="${1:-$REPO_ROOT/.containerignore}"
[[ -f "$IGNORE_FILE" && ! -L "$IGNORE_FILE" ]] || exit 2
[[ $(timeout --kill-after=1 10 podman info --format '{{.Host.Security.Rootless}}') == true ]] || {
    printf 'This test requires rootless Podman.\n' >&2
    exit 1
}

readonly CASE_ROOT="$(mktemp -d /tmp/forge-runtime-context-test.XXXXXXXX)"
readonly CONTEXT="$CASE_ROOT/context"
readonly SUFFIX="${CASE_ROOT##*/}"
readonly IMAGE="localhost/forge-runtime-context-test:${SUFFIX,,}"
readonly CONTAINER="${SUFFIX,,}"
printf 'Synthetic context, image %s and unstarted container %s retained at %s\n' \
    "$IMAGE" "$CONTAINER" "$CASE_ROOT"

allowed=(
    infra/runtime/Containerfile.fixture
    infra/runtime/Containerfile
    infra/runtime/codex-fixture.sh
    infra/runtime/package.json
    infra/runtime/package-lock.json
    target/debug/forge-runner
    target/debug/forge-opencode-driver
)
blocked=(
    infra/runtime/unlisted.env
    infra/dev/auth.json
    target/debug/unlisted-build-file
    target/other/unlisted-build-file
    var/dev/secrets.env
    unlisted-root-file
)
for path in "${allowed[@]}" "${blocked[@]}"; do
    install -D -m 600 /dev/null "$CONTEXT/$path"
done
install -m 600 "$IGNORE_FILE" "$CONTEXT/.containerignore"
install -m 600 "$TEST_DIR/fixtures/runtime-build-context.Containerfile" "$CASE_ROOT/Containerfile"
printf '%s\n' "${allowed[@]}" | LC_ALL=C sort >"$CASE_ROOT/expected"

timeout --kill-after=1 45 podman build --pull=never --network=none --http-proxy=false \
    --layers=false --file "$CASE_ROOT/Containerfile" --tag "$IMAGE" "$CONTEXT" \
    >"$CASE_ROOT/build.log" 2>&1
# The container is only created/exported: no process or synthetic binary runs.
timeout --kill-after=1 10 podman create --pull=never --network=none --http-proxy=false \
    --read-only --cap-drop=all --security-opt=no-new-privileges --name "$CONTAINER" "$IMAGE" \
    >"$CASE_ROOT/container.id"
timeout --kill-after=1 10 podman export --output "$CASE_ROOT/rootfs.tar" "$CONTAINER"
tar -tf "$CASE_ROOT/rootfs.tar" | while IFS= read -r path; do
    case "$path" in
        context/|context/*/) ;;
        context/*) printf '%s\n' "${path#context/}" ;;
    esac
done | LC_ALL=C sort >"$CASE_ROOT/actual"
if ! cmp -s "$CASE_ROOT/expected" "$CASE_ROOT/actual"; then
    printf 'Build context differs from the seven-file allowlist. Expected:\n' >&2
    sort "$CASE_ROOT/expected" >&2
    printf 'Actual synthetic paths:\n' >&2
    sort "$CASE_ROOT/actual" >&2
    exit 1
fi
printf 'runtime-build-context-test: passed (seven allowed files; all sentinels excluded)\n'
