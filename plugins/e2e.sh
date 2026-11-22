#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/target/plugin-e2e}"
CLI="$ROOT/target/debug/effectcraft-cli"
cd "$ROOT"
rm -rf "$OUT"
mkdir -p "$OUT"

for manifest in plugins/*/plugin.json; do
    cargo run --quiet --package xtask -- plugin "$(dirname "$manifest")"
done
cargo build --quiet --package effectcraft-cli

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

render() {
    local name="$1" effect="$2" min_bytes="$3"
    shift 3
    "$CLI" run comp.new '{"name":"E2E","width":960,"height":540,"duration":2}' "$@" effect.apply "{\"effect\":\"$effect\"}" \
        --empty --save-as "$OUT/$name.ecproj" > /dev/null
    "$CLI" render-frame --project "$OUT/$name.ecproj" --time 0.5 --out "$OUT/$name.png" > /dev/null
    local bytes
    bytes="$(wc -c < "$OUT/$name.png")"
    if [ "$bytes" -lt "$min_bytes" ]; then
        echo "FAIL $name: the frame is $bytes bytes, which is what an unchanged flat frame compresses to"
        exit 1
    fi
    echo "PASS $name rendered ($bytes bytes) -> $OUT/$name.png"
}

render plasma-beam org.craftkit.plasma-beam.beam 20000 layer.newSolid '{"name":"Dark","color":"#05060a"}'
if [ -d "$ROOT/plugins/dist/org.craftkit.lumen-glow" ]; then
    render lumen-glow org.craftkit.lumen-glow.glow 20000 layer.newText '{"text":"GLOW","size":160}'
fi

"$ROOT/plugins/glide/verify.sh"
"$ROOT/plugins/pulse/verify.sh"
