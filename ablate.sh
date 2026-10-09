#!/usr/bin/env bash
# Launch the already-installed baseline once per configuration and report the
# last stable [stats] line for each, so the cost of a feature is the difference
# between two rows rather than a guess.
set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

BUNDLE_ID="com.huyvu8051.desertrustmetal"
UDID="${UDID:-$(xcrun devicectl list devices --json-output /dev/stdout --quiet 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin)["result"]["devices"]; print(next(x["hardwareProperties"]["udid"] for x in d if x.get("hardwareProperties",{}).get("platform")=="iOS"))')}"

SETTLE_SECONDS="${SETTLE_SECONDS:-120}"

run_case() {
    local label="$1"; shift
    local env_json="$1"; shift

    local log
    log="$(mktemp)"
    xcrun devicectl device process launch \
        --device "$UDID" --terminate-existing -e "$env_json" --console "$BUNDLE_ID" \
        > "$log" 2>&1 &
    local pid=$!

    # Early samples are taken while textures are still loading and read far too
    # fast, so wait for a long run of frames and report the last one. A cooldown
    # between cases keeps thermal throttling from leaking into the comparison.
    local waited=0
    while [ "$waited" -lt "$SETTLE_SECONDS" ]; do
        sleep 5
        waited=$((waited + 5))
        [ "$(grep -ac 'draws 2363' "$log" 2>/dev/null)" -ge 12 ] && break
    done

    local line
    line="$(grep -a 'draws 2363' "$log" | tail -1 | tr -d '\r')"
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null

    if [ -z "$line" ]; then
        printf '%-28s  (no stats — map may not have loaded)\n' "$label"
    else
        printf '%-28s  %s\n' "$label" "${line#*] }"
    fi
    rm -f "$log"
    sleep "${COOLDOWN_SECONDS:-25}"
}

echo "device $UDID"
echo

run_case "warmup (discarded)"  '{"GFX_RES_SCALE":"1.0"}'
run_case "baseline (all on)"    '{"GFX_RES_SCALE":"1.0"}'
run_case "no shadow lookups"    '{"GFX_NO_SHADOWS":"1"}'
run_case "no normal map"        '{"GFX_NO_NORMAL_MAP":"1"}'
run_case "no specular (2x pow)" '{"GFX_NO_SPECULAR":"1"}'
run_case "no point light"       '{"GFX_NO_POINT_LIGHT":"1"}'
run_case "all four off"         '{"GFX_NO_SHADOWS":"1","GFX_NO_NORMAL_MAP":"1","GFX_NO_SPECULAR":"1","GFX_NO_POINT_LIGHT":"1"}'
run_case "baseline again"       '{"GFX_RES_SCALE":"1.0"}'
