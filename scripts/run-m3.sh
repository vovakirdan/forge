#!/usr/bin/env bash
# Developer acceptance only: isolated index plus optionally approved Codex Runs.
set -euo pipefail
umask 077
readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$SCRIPT_DIR/lib/dev.sh"
source "$SCRIPT_DIR/lib/m1-input.sh"
source "$SCRIPT_DIR/lib/m1-commands.sh"
source "$SCRIPT_DIR/lib/m1-session.sh"
source "$SCRIPT_DIR/lib/m3-session.sh"
source "$SCRIPT_DIR/lib/m3-seed.sh"
source "$SCRIPT_DIR/lib/m3-workflow.sh"

m3_usage() {
    printf '%s\n' \
        'Usage: bash scripts/run-m3.sh keyless' \
        '       bash scripts/run-m3.sh index [--playground EMPTY_DIRECTORY] [--agentmemory-image IMAGE]' \
        '       bash scripts/run-m3.sh run [--playground EMPTY_DIRECTORY] [--model MODEL] [--auth-file PATH] [--yes]' \
        'keyless: PostgreSQL + synthetic HTTP tests, no providers or personal AgentMemory.' \
        'index: real dedicated offline AgentMemory, no inference credentials.' \
        'run: index plus explicitly approved Codex onboarding, Task and summary.' \
        'All retained session paths are printed; a used playground is never overwritten.'
}

m1_reset_launcher_state
M3_MODE=${1:---help}
(($# == 0)) || shift
M3_PLAYGROUND='' M3_CONTAINER='' M3_PHASE=preparing M3_DEADLINE=0 M3_EXISTING_SERVICES=false
readonly M3_AGENTMEMORY_REVISION=c65e93e31e8865ac4843360e4560c077a93d010b
M3_AGENTMEMORY_IMAGE="localhost/forge-agentmemory:$M3_AGENTMEMORY_REVISION"
while (($#)); do
    case "$1" in
        --yes) M1_APPROVED=true; shift ;;
        --existing-services) M3_EXISTING_SERVICES=true; shift ;;
        --playground|--model|--auth-file|--agentmemory-image)
            (($# >= 2)) || { m3_usage; exit 2; }
            case "$1" in
                --playground) M3_PLAYGROUND=$2 ;; --model) M1_MODEL=$2 ;;
                --auth-file) M1_AUTH_SOURCE=$2 ;; --agentmemory-image) M3_AGENTMEMORY_IMAGE=$2 ;;
            esac
            shift 2 ;;
        *) m3_usage; exit 2 ;;
    esac
done
case "$M3_MODE" in
    --help|-h|help) m3_usage; exit 0 ;;
    keyless)
        cd "$FORGE_ROOT_DIR"
        bash "$SCRIPT_DIR/tests/m3-launcher-test.sh"
        require_dev_environment
        FORGE_INTEGRATION=1 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo test --locked -p forge-testkit \
            --test m3_knowledge --test m3_memory_projection --test m3_system_job_ownership \
            --test m3_system_job_lifecycle --test m3_context --test m3_system_job_gateway \
            --test m3_system_job_sources --test m3_onboarding_admission --test m3_system_job_restart \
            -- --ignored --test-threads=1
        exit ;;
    index|run) ;;
    *) m3_usage; exit 2 ;;
esac
trap m3_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
m3_prepare
m3_index_scenario
if [[ $M3_MODE == run ]]; then m3_employee_scenario; fi
m3_stop_proof
M3_PHASE=passed
