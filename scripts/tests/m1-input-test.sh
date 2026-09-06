#!/usr/bin/env bash
# Synthetic files only: never inspect the operator's Codex home or run inference.
set -euo pipefail
umask 077
readonly TEST_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$TEST_DIR/../lib/m1-input.sh"
case_root=$(mktemp -d /tmp/forge-m1-input-test.XXXXXXXX)
trap 'm1_close_auth; rm -r -- "$case_root"' EXIT

M1_CLEANUP_DONE=1 M1_CLEANUP_STATUS=9 M1_CORE_PID=123 M1_SUPERVISOR_TOKEN=foreign
M1_AUTH_FD=99 M1_COMMAND_DEADLINE=1 M1_APPROVED=true M1_ROOT=/foreign
m1_reset_launcher_state
[[ "$M1_CLEANUP_DONE:$M1_CLEANUP_STATUS" == 0:0 ]]
[[ -z "$M1_CORE_PID$M1_SUPERVISOR_TOKEN$M1_ROOT" && "$M1_APPROVED" == false ]]
[[ -z ${M1_AUTH_FD:-} && -z ${M1_COMMAND_DEADLINE:-} ]]

synthetic_auth() {
    jq -n '{auth_mode:"chatgpt",last_refresh:"2026-09-06T00:00:00Z",tokens:{
      access_token:"synthetic-access-not-a-real-key",
      refresh_token:"synthetic-refresh-not-a-real-key",
      id_token:"synthetic-id-not-a-real-key",account_id:"synthetic-account"}}' >"$1"
    chmod 600 "$1"
}

synthetic_auth "$case_root/source.json"
m1_open_auth "$case_root/source.json"
# Replace the path after validation: copy must still use the originally opened inode.
mv "$case_root/source.json" "$case_root/original.json"
printf 'not credentials\n' >"$case_root/source.json"
M1_AUTH="$case_root/copy.json"
m1_copy_auth
cmp "$case_root/original.json" "$M1_AUTH"
[[ $(stat -c '%a' "$M1_AUTH") == 600 ]]
[[ -z ${M1_AUTH_FD:-} ]]
M1_ROOT="$case_root"
M1_AUTH="$case_root/auth.json"
cp "$case_root/copy.json" "$M1_AUTH"
m1_remove_import_copy
[[ ! -e "$M1_AUTH" && -f "$case_root/original.json" ]]
M1_AUTH="$case_root/original.json"
if m1_remove_import_copy; then exit 1; fi

reject() {
    if m1_open_auth "$1" >"$case_root/output" 2>&1; then
        printf 'Unexpected auth acceptance: %s\n' "$1" >&2
        exit 1
    fi
    ! grep -q 'synthetic-access' "$case_root/output"
    [[ -z ${M1_AUTH_FD:-} ]]
}

ln -s "$case_root/original.json" "$case_root/link.json"
reject "$case_root/link.json"
mkfifo "$case_root/fifo.json"
reject "$case_root/fifo.json"
ln "$case_root/original.json" "$case_root/hardlink.json"
reject "$case_root/hardlink.json"
synthetic_auth "$case_root/wide.json"
chmod 644 "$case_root/wide.json"
reject "$case_root/wide.json"
printf '{"OPENAI_API_KEY":"synthetic-access"}\n' >"$case_root/api.json"
reject "$case_root/api.json"
reject "$case_root/source.json"

printf 'model = "sample-model"\n[profiles.other]\nmodel = "wrong"\n' >"$case_root/config.toml"
[[ $(m1_model_hint "$case_root/config.toml") == sample-model ]]
printf '[profiles.other]\nmodel = "wrong"\n' >"$case_root/nested.toml"
[[ -z $(m1_model_hint "$case_root/nested.toml") ]]
printf 'model = 123 # "wrong"\n' >"$case_root/invalid.toml"
[[ -z $(m1_model_hint "$case_root/invalid.toml") ]]

# Without consent, the selected file must not even be opened in noninteractive mode.
(
    m1_open_auth() { printf 'opened' >"$case_root/opened"; }
    M1_AUTH_SOURCE="$case_root/original.json"
    M1_MODEL=explicit-model
    M1_TASK_TEXT='some task'
    M1_APPROVED=false
    if m1_prepare_inputs </dev/null >"$case_root/consent-output" 2>&1; then exit 1; fi
    [[ ! -e "$case_root/opened" ]]
)

printf 'M1 input tests passed: descriptor copy, auth bounds, model hint and explicit consent.\n'
