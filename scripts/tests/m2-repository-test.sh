#!/usr/bin/env bash
# Real Git fixture proof; no Forge service, provider or credential access.
set -euo pipefail
umask 077
readonly TEST_SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly PREPARE="$TEST_SCRIPT_DIR/../prepare-m2-repository.sh"

manifest=$(GIT_DIR=/definitely/not/a/repository GIT_WORK_TREE=/also/not/a/worktree bash "$PREPARE")
fixture_root=$(jq -er '.fixture_root' <<<"$manifest")
source_path=$(jq -er '.source' <<<"$manifest")
base=$(jq -er '.initial_base' <<<"$manifest")
[[ $fixture_root == /tmp/forge-m2-repository.* && $source_path == "$fixture_root/source.git" ]]
[[ $(stat -c '%a' "$fixture_root") == 700 && ! -L $fixture_root ]]
[[ $(git --git-dir="$source_path" rev-parse --is-bare-repository) == true ]]
[[ $(git --git-dir="$source_path" rev-parse refs/heads/main) == "$base" ]]
[[ $(git --git-dir="$source_path" ls-tree --name-only "$base" | wc -l) == 2 ]]
[[ $(git --git-dir="$source_path" ls-tree "$base" normalize-lines.sh) == 100755* ]]
[[ $(git --git-dir="$source_path" remote) == '' ]]
git --git-dir="$source_path" fsck --strict >/dev/null
jq -e --arg source "$source_path" '.registration.source == $source and .target_ref == "refs/heads/main"' \
    "$fixture_root/fixture.json" >/dev/null
if bash "$PREPARE" /home/zov/projects/forge >/dev/null 2>&1; then
    printf 'Fixture launcher accepted an existing target path.\n' >&2
    exit 1
fi
printf 'm2-repository-test: passed; retained fixture: %s\n' "$fixture_root"
