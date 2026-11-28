#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/lib.sh"
OUT="${1:-$APP_TARGET/bench-projects}"
N="${N:-30}"
cd "$ROOT"
rm -rf "$OUT"
mkdir -p "$OUT"

if [ -z "${SKIP_BUILD:-}" ]; then
    for p in lumen-glow retro halo; do xtask_plugin plugins/$p; done
    build_app_packages -j 4 --package effectcraft-cli
fi
export EFFECTCRAFT_CONFIG_DIR="$OUT/config"
export TMPDIR="$OUT"
export EFFECTCRAFT_PLUGIN_PATH="$ROOT/plugins/dist/org.craftkit.lumen-glow:$ROOT/plugins/dist/org.craftkit.retro:$ROOT/plugins/dist/org.craftkit.halo"

if [ "$(uname)" = Darwin ]; then MEASURE=(/usr/bin/time -l); PEAK='maximum resident set size'; else MEASURE=(/usr/bin/time -v); PEAK='Maximum resident set size'; fi

comp() { echo "{\"name\":\"Bench\",\"width\":$1,\"height\":$2,\"frameRate\":30,\"duration\":10}"; }
solid() { echo "{\"name\":\"S$1\",\"color\":\"#8a6a4$1\"}"; }
apply() { echo "{\"effect\":\"$1\"}"; }

project() {
    local name="$1" size="$2"
    shift 2
    "$CLI" run comp.new "$(comp $size)" "$@" --empty --save-as "$OUT/$name.ecproj" > /dev/null
}

project glow-levels "1920 1080" layer.newSolid "$(solid 0)" effect.apply "$(apply org.craftkit.lumen-glow.glow)" effect.apply "$(apply ec.color.levels)"
project neon-stack "1920 1080" layer.newSolid "$(solid 1)" layer.newSolid "$(solid 2)" layer.newSolid "$(solid 3)" \
    layer.newSolid "$(solid 4)" layer.newSolid "$(solid 5)" layer.newSolid "$(solid 6)" effect.apply "$(apply org.craftkit.retro.neon)"
project halo-4k-region "3840 2160" layer.newSolid "$(solid 7)" effect.apply "$(apply org.craftkit.halo.flare)"

run() {
    local name="$1"
    shift
    local log="$OUT/$name.log"
    "${MEASURE[@]}" "$CLI" bench --project "$OUT/$name.ecproj" --n "$N" "$@" > "$log" 2>&1
    printf '%-18s %s | peak %s MB\n' "$name" "$(grep -o 'wall min.*ms; CPU' "$log" | head -1 | sed 's/; CPU//')" \
        "$(awk -v k="$PEAK" '$0 ~ k {printf "%d", ($1 > 1e8 ? $1 / 1048576 : $1 / 1024)}' "$log")"
    grep ' fx ' "$log" | sed 's/^/    /'
}

run glow-levels
run neon-stack
run halo-4k-region --roi 1280,720,1280,720
