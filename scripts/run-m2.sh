#!/usr/bin/env bash
# Explicit local operator exercise, not an installer or unattended provider gate.
set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$SCRIPT_DIR/lib/dev.sh"
source "$SCRIPT_DIR/lib/m1-input.sh"
source "$SCRIPT_DIR/lib/m1-commands.sh"
source "$SCRIPT_DIR/lib/m1-session.sh"
source "$SCRIPT_DIR/lib/m2-common.sh"
source "$SCRIPT_DIR/lib/m2-config.sh"
source "$SCRIPT_DIR/lib/m2-session.sh"
source "$SCRIPT_DIR/lib/m2-seed.sh"
source "$SCRIPT_DIR/lib/m2-workflow.sh"
source "$SCRIPT_DIR/lib/m2-evidence.sh"

m2_main "$@"
