#!/usr/bin/env bash
# Build the Bevy baseline for a paired iPhone, install it, and launch it.
# Mirrors ../desert_metal_baseline/deploy.sh so the two are run the same way.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

PROJECT="DesertRustMetal.xcodeproj"
SCHEME="DesertRustMetal"
BUNDLE_ID="com.huyvu8051.desertrustmetal"
CONFIGURATION="${CONFIGURATION:-Release}"
DERIVED_DATA="${DERIVED_DATA:-build/DerivedData}"
CARGO_PROFILE="${CARGO_PROFILE:-release}"
GAME_ASSETS="../bevy_fps_demo/crates/client/assets"

usage() {
    cat <<'EOF'
usage: ./deploy.sh [options]

  -d, --device <name|udid>  target a specific device (default: the first paired iPhone/iPad)
  -c, --console             stay attached and stream the app's stdout
  -e, --env KEY=VALUE       pass an env var to the app (repeatable); see the ablation switches below
      --no-launch           install only
      --list                list paired iOS devices and exit
  -h, --help                show this

ablation switches (read by the Rust side at startup):
  GFX_MSAA=1          multisampling on
  GFX_SHADOW=<0-5>    shadow map quality (0 = off)
  GFX_LIGHT=<0-5>     directional light quality
  GFX_POINT_LIGHT=<0-5>
  GFX_RES_SCALE=<f>   render resolution scale

environment:
  CONFIGURATION   Release (default) or Debug      — the Xcode/Swift side
  CARGO_PROFILE   release (default) or dev-fast   — the Rust side
EOF
}

device=""
console=0
launch=1
env_pairs=()

while [ $# -gt 0 ]; do
    case "$1" in
        -d|--device) device="${2:?--device needs a value}"; shift 2 ;;
        -c|--console) console=1; shift ;;
        -e|--env) env_pairs+=("${2:?--env needs KEY=VALUE}"); shift 2 ;;
        --no-launch) launch=0; shift ;;
        --list) xcrun devicectl list devices; exit 0 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

resolve_device() {
    local json
    json="$(mktemp)"
    trap 'rm -f "$json"' RETURN
    xcrun devicectl list devices --json-output "$json" --quiet
    python3 - "$json" "${1-}" <<'PY'
import json, sys

path, wanted = sys.argv[1], sys.argv[2]
devices = [
    d for d in json.load(open(path))["result"]["devices"]
    if d.get("hardwareProperties", {}).get("platform") == "iOS"
]
if wanted:
    devices = [
        d for d in devices
        if wanted in (d["hardwareProperties"]["udid"], d["identifier"], d["deviceProperties"].get("name"))
    ]
    if not devices:
        sys.exit(f'no paired iOS device matches "{wanted}" — try ./deploy.sh --list')
elif not devices:
    sys.exit("no paired iOS device — plug one in, or pair it in Xcode > Window > Devices and Simulators")

chosen = devices[0]
print(chosen["hardwareProperties"]["udid"], chosen["deviceProperties"].get("name", "?"), sep="\t")
PY
}

target="$(resolve_device "$device")"
udid="${target%%$'\t'*}"
name="${target#*$'\t'}"
echo "==> device: $name ($udid)"

echo "==> cargo build --profile $CARGO_PROFILE"
cargo build --target aarch64-apple-ios --profile "$CARGO_PROFILE"

cargo_out="target/aarch64-apple-ios/$CARGO_PROFILE/libdesert_rust_metal.a"
[ -f "$cargo_out" ] || { echo "missing $cargo_out" >&2; exit 1; }
mkdir -p build/lib
cp "$cargo_out" build/lib/

# Bevy's iOS asset reader resolves relative paths against <bundle>/assets, so
# the map has to sit at assets/maps/desert_city. A real copy, not a symlink —
# Xcode's folder-reference copy does not reliably follow those.
echo "==> staging map assets"
mkdir -p build/bundle/assets/maps
rsync -a --delete "$GAME_ASSETS/maps/desert_city/" build/bundle/assets/maps/desert_city/

if [ ! -d "$PROJECT" ] || [ project.yml -nt "$PROJECT/project.pbxproj" ]; then
    command -v xcodegen >/dev/null || { echo "xcodegen is not installed — brew install xcodegen" >&2; exit 1; }
    echo "==> xcodegen generate"
    xcodegen generate
fi

echo "==> building $CONFIGURATION"
build_log="$(mktemp)"
trap 'rm -f "$build_log"' EXIT
if ! xcodebuild \
    -project "$PROJECT" \
    -scheme "$SCHEME" \
    -configuration "$CONFIGURATION" \
    -destination "platform=iOS,id=$udid" \
    -derivedDataPath "$DERIVED_DATA" \
    -allowProvisioningUpdates \
    build > "$build_log" 2>&1
then
    grep -E "error:|error$" "$build_log" >&2 || tail -40 "$build_log" >&2
    echo "full log: $build_log" >&2
    trap - EXIT
    exit 1
fi

app="$DERIVED_DATA/Build/Products/$CONFIGURATION-iphoneos/$SCHEME.app"
[ -d "$app" ] || { echo "built, but $app is missing" >&2; exit 1; }

echo "==> installing $(du -sh "$app" | awk '{print $1}')"
xcrun devicectl device install app --device "$udid" "$app" --quiet

if [ "$launch" = 0 ]; then
    echo "==> installed, not launching"
    exit 0
fi

launch_args=(--device "$udid" --terminate-existing)
if [ "${#env_pairs[@]}" -gt 0 ]; then
    launch_args+=(-e "$(python3 -c '
import json, sys
print(json.dumps(dict(pair.split("=", 1) for pair in sys.argv[1:])))' "${env_pairs[@]}")")
fi

echo "==> launching $BUNDLE_ID ${env_pairs[*]-}"
if [ "$console" = 1 ]; then
    xcrun devicectl device process launch "${launch_args[@]}" --console "$BUNDLE_ID"
else
    xcrun devicectl device process launch "${launch_args[@]}" "$BUNDLE_ID" --quiet
    echo "==> running (re-run with --console to stream its output)"
fi
