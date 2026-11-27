#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/target/bench-app}"
CLI="$ROOT/target/debug/effectcraft-cli"
N="${N:-7}"
cd "$ROOT"
rm -rf "$OUT"
mkdir -p "$OUT"

if [ -z "${SKIP_BUILD:-}" ]; then
    cargo run --quiet --package xtask -- plugin plugins/lumen-glow
    cargo build --quiet --package effectcraft-cli
fi
export EFFECTCRAFT_CONFIG_DIR="$OUT/config"
export EFFECTCRAFT_PLUGIN_PATH="$ROOT/plugins/dist/org.craftkit.lumen-glow"

COMP='{"name":"Bench","width":1920,"height":1080,"frameRate":30,"duration":4}'
SOLID='{"name":"S","color":"#8a6a40"}'
BLUR='{"path":"effects/#1/blurriness","value":20}'

project() {
    local name="$1"
    shift
    "$CLI" run comp.new "$COMP" "$@" --empty --save-as "$OUT/$name.ecproj" > /dev/null
}

project solid layer.newSolid "$SOLID"
project blur layer.newSolid "$SOLID" effect.apply '{"effect":"ec.blur.gaussian"}' prop.set "$BLUR"
project stack5 layer.newSolid "$SOLID" effect.apply '{"effect":"ec.blur.gaussian"}' prop.set "$BLUR" \
    effect.apply '{"effect":"ec.color.levels"}' effect.apply '{"effect":"ec.color.curves"}' \
    effect.apply '{"effect":"ec.color.huesaturation"}' effect.apply '{"effect":"ec.stylize.glow"}'
project lumen layer.newSolid "$SOLID" effect.apply '{"effect":"org.craftkit.lumen-glow.glow"}'
ten=()
for i in 0 1 2 3 4 5 6 7 8 9; do
    ten+=(layer.newSolid "{\"name\":\"S$i\",\"color\":\"#8a6a4$i\"}" effect.apply '{"effect":"ec.color.huesaturation"}'
        prop.set '{"path":"transform/opacity","value":50}')
done
project ten "${ten[@]}"

cpu() {
    "$CLI" bench --project "$OUT/$1.ecproj" --time 1 --n "$N" "${@:2}" 2>&1
}
median_of() {
    sed -nE 's/^bench \(no cache\).*median ([0-9.]+) ms.*/\1/p'
}

row() {
    printf '%-34s %10s\n' "$1" "$2"
}
printf '%-34s %10s\n' "case (1920x1080, median of $N)" "ms"
row "a solid, no effect" "$(cpu solid | median_of)"
row "b solid + gaussian blur 20" "$(cpu blur | median_of)"
row "c solid + 5 built-in effects" "$(cpu stack5 | median_of)"
lumen="$(cpu lumen --cached)"
echo "$lumen" > "$OUT/lumen.txt"
row "d lumen-glow CPU" "$(echo "$lumen" | median_of)"
gpu="$("$CLI" bench --project "$OUT/lumen.ecproj" --comp Bench --time 1 --n "$N" --gpu 2>&1)"
echo "$gpu" > "$OUT/lumen-gpu.txt"
row "e lumen-glow GPU (cold)" "$(echo "$gpu" | awk '$1=="Bench" && $2=="Full" {print $5}')"
ten="$(cpu ten --cached)"
echo "$ten" > "$OUT/ten.txt"
row "f 10 layers, one effect each" "$(echo "$ten" | median_of)"
row "g lumen-glow, same frame cached" "$(echo "$lumen" | sed -nE 's/.*same frame median ([0-9.]+) ms.*/\1/p')"
row "h lumen-glow, next frame cached" "$(echo "$lumen" | sed -nE 's/.*next frame median ([0-9.]+) ms.*/\1/p')"
row "i 10 layers, next frame cached" "$(echo "$ten" | sed -nE 's/.*next frame median ([0-9.]+) ms.*/\1/p')"
