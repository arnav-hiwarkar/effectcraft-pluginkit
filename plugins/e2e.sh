#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/lib.sh"
OUT="${1:-$APP_TARGET/plugin-e2e}"
cd "$ROOT"
rm -rf "$OUT"
mkdir -p "$OUT"

for manifest in plugins/*/plugin.json; do
    xtask_plugin "$(dirname "$manifest")"
done
build_app_packages --package effectcraft-cli --package effectcraft-plugin-helper
export CLI

export EFFECTCRAFT_CONFIG_DIR="$OUT/config"

INSTALLED_ID="org.craftkit.plasma-beam"
PACKAGE="$(find "$ROOT/plugins/dist" -maxdepth 1 -name "$INSTALLED_ID-*.craftplugin" | sort | tail -n 1)"
if [ -z "$PACKAGE" ]; then
    echo "FAIL no .craftplugin file for $INSTALLED_ID in plugins/dist"
    exit 1
fi
"$CLI" exec plugin.install "{\"path\":\"$PACKAGE\"}" > /dev/null
"$CLI" exec plugin.approve "{\"id\":\"$INSTALLED_ID\"}" > /dev/null
echo "PASS installed $INSTALLED_ID from $(basename "$PACKAGE")"

EFFECTCRAFT_PLUGIN_PATH="$(find "$ROOT/plugins/dist" -mindepth 1 -maxdepth 1 -type d ! -name "$INSTALLED_ID" | sort | paste -sd: -)"
export EFFECTCRAFT_PLUGIN_PATH

"$CLI" exec plugin.list --json > "$OUT/plugins.json"
python3 - "$OUT/plugins.json" <<'PY'
import json, sys
plugins = json.load(open(sys.argv[1]))
stopped = [f"{p['id']}: {p['state']} {p['error'] or ''}" for p in plugins if p["state"] != "running"]
assert plugins and not stopped, f"plugins not running: {stopped}"
print("PASS", len(plugins), "plugins running:", ", ".join(p["id"] for p in plugins))
PY

stored_frames=()
golden() {
    local plugin="$1" name="$2"
    shift 2
    stored_frames+=("plugins/$plugin/tests/frames/$name.png")
    xtask_plugin render "plugins/$plugin" "$@" --compare "plugins/$plugin/tests/frames/$name.png" --out "$OUT/golden/$plugin-$name.png" > /dev/null
    echo "PASS $plugin $name matches its stored frame"
}

render() {
    local name="$1" effect="$2"
    shift 2
    local comp='{"name":"E2E","width":960,"height":540,"duration":2}'
    "$CLI" run comp.new "$comp" "$@" --empty --save-as "$OUT/$name-plain.ecproj" > /dev/null
    "$CLI" run comp.new "$comp" "$@" effect.apply "{\"effect\":\"$effect\"}" --empty --save-as "$OUT/$name.ecproj" > /dev/null
    "$CLI" render-frame --project "$OUT/$name-plain.ecproj" --time 0.5 --out "$OUT/$name-plain.png" > /dev/null
    "$CLI" render-frame --project "$OUT/$name.ecproj" --time 0.5 --out "$OUT/$name.png" > /dev/null
    if cmp -s "$OUT/$name-plain.png" "$OUT/$name.png"; then
        echo "FAIL $name: the frame with $effect is byte-identical to the frame without it"
        exit 1
    fi
    echo "PASS $name changed the frame -> $OUT/$name.png"
}

render plasma-beam org.craftkit.plasma-beam.beam layer.newSolid '{"name":"Dark","color":"#05060a"}'
render lumen-glow org.craftkit.lumen-glow.glow layer.newText '{"text":"GLOW","size":160}'

source "$PLUGINS_DIR/goldens.sh"

for stored in plugins/*/tests/frames/*.png; do
    case " ${stored_frames[*]} " in
        *" $stored "*) ;;
        *)
            echo "FAIL $stored has no golden line in plugins/goldens.sh"
            exit 1
            ;;
    esac
done

"$PLUGINS_DIR/glide/verify.sh"
"$PLUGINS_DIR/pulse/verify.sh"
