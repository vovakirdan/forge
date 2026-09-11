#!/usr/bin/env bash

m3_error() { printf 'forge M3: %s\n' "$*" >&2; return 1; }

m3_confirm() {
    local answer hint chosen_home=${CODEX_HOME:-$HOME/.codex}
    [[ $M3_MODE == run ]] || return 0
    if [[ -z $M1_MODEL ]]; then
        [[ -t 0 ]] || { m3_error 'Noninteractive run requires --model.'; return 1; }
        hint=$(m1_model_hint "$chosen_home/config.toml")
        read -r -p "Точная модель Codex${hint:+ [$hint]}: " M1_MODEL
        M1_MODEL=${M1_MODEL:-$hint}
    fi
    [[ $M1_MODEL =~ ^[a-zA-Z0-9][a-zA-Z0-9._:/-]*$ ]] || return 1
    if [[ -z $M1_AUTH_SOURCE ]]; then
        [[ -t 0 ]] || { m3_error 'Noninteractive run requires --auth-file.'; return 1; }
        read -r -p "Auth файл [$chosen_home/auth.json]: " M1_AUTH_SOURCE
        M1_AUTH_SOURCE=${M1_AUTH_SOURCE:-$chosen_home/auth.json}
    fi
    [[ $M1_AUTH_SOURCE == /* && $M1_AUTH_SOURCE != *[[:cntrl:]]* ]] || return 1
    printf 'M3 Codex: %s\nAuth: %s\n' "$M1_MODEL" "$M1_AUTH_SOURCE"
    printf 'Обычно 3 Runs: onboarding → Task → summary. SystemJobs: максимум 4 попытки за скользящие 24 часа, одна на generation job.\n'
    printf 'Один Run одновременно; Task до 600s, job до 300s, сценарий до 1800s при живом launcher.\n'
    printf 'Остановка при наблюдении 6 Runs; возможен дополнительный Run между опросами. Расходуется подписка.\n'
    if [[ $M1_APPROVED != true ]]; then
        [[ -t 0 ]] || { m3_error 'Noninteractive run requires --yes.'; return 1; }
        read -r -p 'Разрешить эти реальные Codex Runs? [y/N] ' answer
        [[ $answer == y || $answer == Y || $answer == да ]] || return 1
    fi
    m1_open_auth "$M1_AUTH_SOURCE"
}

m3_prepare_playground() {
    local entry
    if [[ -z $M3_PLAYGROUND ]]; then M3_PLAYGROUND=$(mktemp -d /tmp/forge-m3-playground.XXXXXXXX); fi
    [[ -d $M3_PLAYGROUND && ! -L $M3_PLAYGROUND ]] || { m3_error 'Нужен существующий пустой каталог playground.'; return 1; }
    M3_PLAYGROUND=$(CDPATH= cd -- "$M3_PLAYGROUND" && pwd -P)
    [[ $M3_PLAYGROUND != "$FORGE_ROOT_DIR" && $M3_PLAYGROUND != "$FORGE_ROOT_DIR/"* && $M3_PLAYGROUND != *[[:cntrl:]]* ]] || return 1
    entry=$(find "$M3_PLAYGROUND" -mindepth 1 -maxdepth 1 -print -quit)
    [[ -z $entry ]] || { m3_error 'Playground уже непустой; создай новый, старые данные остаются.'; return 1; }
    env -i PATH=/usr/bin:/bin GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null \
        git -c core.hooksPath=/dev/null init --quiet --template= --initial-branch=main "$M3_PLAYGROUND" || return 1
    install -d -m 700 "$M3_PLAYGROUND/.forge-m3" "$M3_PLAYGROUND/.git/info"
    printf '/.forge-m3/\n' >"$M3_PLAYGROUND/.git/info/exclude"
}

m3_prepare() {
    local executable revision container_image
    for executable in cargo podman podman-compose jq uuidgen timeout install stat git openssl curl; do require_command "$executable"; done
    require_rootless_podman
    [[ $M3_AGENTMEMORY_IMAGE =~ ^[a-zA-Z0-9][a-zA-Z0-9./:@_-]*$ ]] || return 1
    [[ -z ${CARGO_TARGET_DIR:-} || $CARGO_TARGET_DIR == target || $CARGO_TARGET_DIR == "$FORGE_ROOT_DIR/target" ]] || return 1
    revision=$(podman image inspect --format '{{index .Labels "org.opencontainers.image.revision"}}' "$M3_AGENTMEMORY_IMAGE" 2>/dev/null) \
        || { m3_error 'Сначала собери закреплённый Forge AgentMemory image; смотри docs/M3_ACCEPTANCE.md.'; return 1; }
    [[ $revision == "$M3_AGENTMEMORY_REVISION" ]] || { m3_error 'AgentMemory image имеет другую source revision.'; return 1; }
    container_image=$(podman image inspect --format '{{.Id}}' "$M3_AGENTMEMORY_IMAGE")
    container_image=${container_image#sha256:}
    [[ $container_image =~ ^[0-9a-f]{64}$ ]] || return 1
    M3_AGENTMEMORY_IMAGE="sha256:$container_image"
    m3_confirm
    m3_prepare_playground
    M1_ROOT=$(mktemp -d /tmp/fm3.XXXXXXXX)
    M1_EXEC="$M1_ROOT/exec" M1_AUTH="$M1_ROOT/auth.json" M1_HOST="m3-manual-$(m1_uuid7)"
    # Reuse M1's dedicated database and exact-child cleanup without reusing its seed.
    M1_DATABASE="forge_m1_$(m1_uuid7 | tr -d '-')" M1_SESSION_NAME=M3
    install -d -m 700 "$M1_EXEC" "$M1_ROOT/evidence"
    jq -n --arg root "$M1_ROOT" '{root:$root}' >"$M3_PLAYGROUND/.forge-m3/session.json"
    [[ $M3_MODE != run ]] || m1_copy_auth
    printf 'M3 playground: %s\nПриватная сессия: %s\n' "$M3_PLAYGROUND" "$M1_ROOT"
    cd "$FORGE_ROOT_DIR"
    export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
    if [[ $M3_EXISTING_SERVICES != true ]]; then
        if [[ ! -e $FORGE_DEV_CONFIG_FILE && ! -e $FORGE_DEV_SECRETS_FILE ]]; then bash "$SCRIPT_DIR/dev-init.sh" >"$M1_ROOT/setup.log" 2>&1; fi
        bash "$SCRIPT_DIR/dev-up.sh" >>"$M1_ROOT/setup.log" 2>&1
    fi
    require_dev_environment
    wait_for_postgres
    wait_for_nats
    cargo build --workspace --bins --locked >>"$M1_ROOT/setup.log" 2>&1
    if [[ $M3_MODE == run ]]; then
        bash "$SCRIPT_DIR/build-runtime-image.sh" >>"$M1_ROOT/setup.log" 2>&1
        M1_IMAGE="localhost/forge-runtime@$(podman image inspect --format '{{.Digest}}' localhost/forge-runtime:m1)"
        [[ $M1_IMAGE =~ @sha256:[0-9a-f]{64}$ ]] || return 1
    fi
    m3_start_index
    export FORGE_AGENTMEMORY_URL="$M3_INDEX_URL" FORGE_AGENTMEMORY_SECRET_FILE="$M1_ROOT/agentmemory.token" FORGE_AGENTMEMORY_TIMEOUT_SECONDS=3
    export FORGE_HOST_MAX_RUNS=1 FORGE_PROJECT_MAX_RUNS=1 FORGE_ACCOUNT_MAX_RUNS=1
    m1_start_services
    M3_DEADLINE=$((SECONDS+1800))
    M1_PROJECT_ID=$(m1_uuid7)
    m1_command create_project '{"name":"M3 isolated memory acceptance"}' >/dev/null
    m1_record_session
}

m3_start_index() {
    local port deadline network
    M3_NETWORK="forge-m3-$M1_HOST" M3_VOLUME="forge-m3-$M1_HOST"
    openssl rand -hex 32 >"$M1_ROOT/agentmemory.token"
    printf 'AGENTMEMORY_SECRET=%s\n' "$(<"$M1_ROOT/agentmemory.token")" >"$M1_ROOT/agentmemory.env"
    podman network create --internal --label "forge.m3.session=$M1_HOST" "$M3_NETWORK" >"$M1_ROOT/network.log" 2>&1
    network=$(podman network inspect --format '{{.Internal}}' "$M3_NETWORK")
    [[ $network == true ]] || { m3_error 'AgentMemory network не internal.'; return 1; }
    podman volume create --label "forge.m3.session=$M1_HOST" "$M3_VOLUME" >"$M1_ROOT/volume.log" 2>&1
    M3_CONTAINER=$(podman run --detach --init --pull=never --network "$M3_NETWORK" \
        --label "forge.m3.session=$M1_HOST" --name "forge-m3-$M1_HOST" \
        --cpus 2 --memory 2g --pids-limit 256 --cap-drop=ALL --security-opt=no-new-privileges \
        --userns=keep-id:uid=1000,gid=1000 --volume "$M3_VOLUME:/data:U" \
        --publish 127.0.0.1::3111 --env-file "$M1_ROOT/agentmemory.env" \
        "$M3_AGENTMEMORY_IMAGE" 2>"$M1_ROOT/index-start.log")
    [[ $M3_CONTAINER =~ ^[0-9a-f]{64}$ ]] || return 1
    port=$(podman port "$M3_CONTAINER" 3111/tcp)
    [[ $port =~ ^127\.0\.0\.1:([1-9][0-9]{0,4})$ ]] || return 1
    M3_INDEX_URL="http://$port"
    deadline=$((SECONDS+90))
    until curl --silent --fail --max-time 2 "$M3_INDEX_URL/agentmemory/livez" >/dev/null 2>&1; do
        ((SECONDS<deadline)) || { m3_error 'AgentMemory livez не готов; смотри retained container logs.'; return 1; }
        sleep 1
    done
    # Evidence contains only labels, immutable image ID and the isolated network.
    jq -n --arg id "$M3_CONTAINER" --arg image "$M3_AGENTMEMORY_IMAGE" --arg revision "$M3_AGENTMEMORY_REVISION" \
        --arg network "$M3_NETWORK" --arg volume "$M3_VOLUME" --arg endpoint "$M3_INDEX_URL" \
        '{container_id:$id,image_id:$image,source_revision:$revision,network:$network,network_internal:true,volume:$volume,endpoint:$endpoint}' >"$M1_ROOT/evidence/index-runtime.json"
}

m3_index_control() {
    [[ $M3_CONTAINER =~ ^[0-9a-f]{64}$ && $(podman inspect --format '{{index .Config.Labels "forge.m3.session"}}' "$M3_CONTAINER") == "$M1_HOST" ]] || return 1
    timeout --kill-after=2 30 podman "$1" ${2:+"$2"} "$M3_CONTAINER" >/dev/null
}

m3_exit() {
    local status=$? cleanup=0
    trap - EXIT
    trap '' INT TERM
    m1_close_auth
    if [[ -n ${M1_ROOT:-} ]]; then
        if [[ -n ${M1_PROJECT_ID:-} && -n ${M1_CORE_PID:-} ]]; then
            m1_stop_project || cleanup=1
            m3_collect || cleanup=1
        fi
        if [[ -n ${M1_CORE_PID:-} || -n ${M1_SUPERVISOR_PID:-} ]]; then m1_cleanup || cleanup=1; fi
        if [[ -n $M3_CONTAINER ]]; then
            m3_index_control stop || cleanup=1
            podman logs "$M3_CONTAINER" >"$M1_ROOT/agentmemory.log" 2>&1 || cleanup=1
        fi
        if [[ -f ${M1_AUTH:-/nonexistent} ]]; then m1_remove_import_copy || cleanup=1; fi
        ((cleanup==0)) || status=1
        jq -n --arg phase "$M3_PHASE" --arg mode "$M3_MODE" --arg root "$M1_ROOT" \
            --argjson exit_code "$status" --argjson cleanup "$cleanup" \
            '{mode:$mode,phase:$phase,outcome:(if $exit_code==0 and $phase=="passed" and $cleanup==0 then "passed" else "failed" end),exit_code:$exit_code,cleanup_failed:($cleanup!=0),session:$root}' \
            >"$M1_ROOT/evidence/report.json" || status=1
        printf 'M3 evidence: %s/evidence/report.json\nСессия, DB, index volume/container/network сохранены; работа остановлена.\n' "$M1_ROOT"
    fi
    exit "$status"
}
