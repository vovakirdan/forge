#!/usr/bin/env bash
# Create only a fresh, disposable local repository; never accept an existing path.
set -euo pipefail
umask 077

if (($#)); then
    if [[ $# == 1 && ( $1 == --help || $1 == -h ) ]]; then
        printf '%s\n' 'Usage: bash scripts/prepare-m2-repository.sh' \
            'Creates a private /tmp/forge-m2-repository.* bare Git repository.' \
            'Prints its manifest as JSON. No services, credentials or providers are used.' \
            'Files are retained; registering the source and starting Forge are separate actions.'
        exit 0
    fi
    printf 'No target path or other arguments are accepted; use --help.\n' >&2
    exit 2
fi

for executable in git jq mktemp env; do
    command -v "$executable" >/dev/null || { printf 'Missing command: %s\n' "$executable" >&2; exit 1; }
done
readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly FIXTURE_ASSETS="$SCRIPT_DIR/../infra/fixtures/m2-repository"
readonly GIT_BIN="$(command -v git)"
FIXTURE_ROOT=$(mktemp -d /tmp/forge-m2-repository.XXXXXXXX)
readonly FIXTURE_ROOT
readonly FIXTURE_SOURCE="$FIXTURE_ROOT/source.git"

# Do not inherit repository selection, user Git config, templates, hooks or signing.
fixture_git() {
    env -i PATH=/usr/bin:/bin LC_ALL=C \
        GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_SYSTEM=/dev/null GIT_CONFIG_GLOBAL=/dev/null \
        GIT_TERMINAL_PROMPT=0 \
        GIT_AUTHOR_NAME='Forge fixture' GIT_AUTHOR_EMAIL=fixture@invalid \
        GIT_COMMITTER_NAME='Forge fixture' GIT_COMMITTER_EMAIL=fixture@invalid \
        "$GIT_BIN" -c core.hooksPath=/dev/null -c commit.gpgSign=false "$@"
}

printf 'Retained isolated fixture: %s\n' "$FIXTURE_ROOT" >&2
fixture_git init --quiet --bare --template= --initial-branch=main "$FIXTURE_SOURCE"
readme=$(fixture_git --git-dir="$FIXTURE_SOURCE" hash-object -w --stdin <"$FIXTURE_ASSETS/README.md")
normalizer=$(fixture_git --git-dir="$FIXTURE_SOURCE" hash-object -w --stdin <"$FIXTURE_ASSETS/normalize-lines.sh")
tree=$(printf '100644 blob %s\tREADME.md\n100755 blob %s\tnormalize-lines.sh\n' "$readme" "$normalizer" \
    | fixture_git --git-dir="$FIXTURE_SOURCE" mktree)
base=$(fixture_git --git-dir="$FIXTURE_SOURCE" commit-tree "$tree" -m 'Seed isolated M2 exercise')
fixture_git --git-dir="$FIXTURE_SOURCE" update-ref refs/heads/main "$base" ''

jq -n --arg root "$FIXTURE_ROOT" --arg source "$FIXTURE_SOURCE" --arg base "$base" \
    '{fixture_root:$root,source:$source,target_ref:"refs/heads/main",initial_base:$base,
      registration:{name:"Isolated M2 exercise",source:$source,target_ref:"refs/heads/main"}}' \
    | tee "$FIXTURE_ROOT/fixture.json"
