#!/usr/bin/env bash
# A/B/A with a real rest between every run.
#
# ablate.sh runs its cases back to back, which heats the phone enough that two
# identical configurations read 101 fps and 62 fps in the same series. This
# rests the device between cases and repeats the first configuration last, so a
# difference is only believable when the two A rows agree.
set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

BUNDLE_ID="com.huyvu8051.desertrustmetal"
UDID="${UDID:?set UDID}"
REST="${REST:-240}"
# Sample after the frame rate has settled, not during the opening seconds when
# it still runs at the display cap.
SAMPLES="${SAMPLES:-14}"

run_case() {
    local label="$1" env_json="$2"

    echo "   (resting ${REST}s)" >&2
    sleep "$REST"

    local log
    log="$(mktemp)"
    xcrun devicectl device process launch \
        --device "$UDID" --terminate-existing -e "$env_json" --console "$BUNDLE_ID" \
        > "$log" 2>&1 &
    local pid=$!

    local waited=0
    while [ "$waited" -lt 120 ]; do
        sleep 5
        waited=$((waited + 5))
        [ "$(grep -ac 'draws 2363' "$log" 2>/dev/null)" -ge "$SAMPLES" ] && break
    done

    local line
    line="$(grep -a 'draws 2363' "$log" | tail -1 | tr -d '\r' | sed 's/^.*\[stats\]/[stats]/')"
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null

    printf '%-26s %s\n' "$label" "${line:-(no samples)}"
    rm -f "$log"
}

run_case "A  baseline"         '{"GFX_RES_SCALE":"1.0"}'
run_case "B  flat shader"      '{"GFX_FLAT":"1"}'
run_case "A  baseline again"  '{"GFX_RES_SCALE":"1.0"}'
