#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${OUT:-/tmp/bench-start}"
CLIS="${CLIS:-$ROOT/target/release/effectcraft-cli}"
ROUNDS="${ROUNDS:-6}"
COUNTS="${COUNTS:-0 20 100}"
cd "$ROOT"
if [ ! -d "$OUT/dist" ]; then
    cargo xtask plugin new bench-plugin --kind effect > /dev/null
    cargo xtask plugin plugins/bench-plugin --native --out "$OUT/dist" > /dev/null
    rm -rf plugins/bench-plugin
fi
for mode in wasm native; do
    for count in $COUNTS; do
        python3 -I plugins/bench-start.py "$OUT/dist/com.example.bench-plugin" "$OUT/$mode-$count" "$count" "$mode"
    done
done
echo "round load cli config phase real_s user_s max_rss_bytes instructions"
for round in $(seq "$ROUNDS"); do
    load=$(uptime | sed 's/.*averages: //' | awk '{print $1}')
    for cli in $CLIS; do
        for mode in wasm native; do
            for count in $COUNTS; do
                for phase in cold warm; do
                    [ "$phase" = cold ] && rm -rf "$OUT/$mode-$count/Plugins/.data/.compiled"
                    figures=$(EFFECTCRAFT_CONFIG_DIR="$OUT/$mode-$count" /usr/bin/time -l "$cli" info --empty --json 2>&1 > /dev/null |
                        awk '/ real/{r=$1; u=$3} /maximum resident/{m=$1} /instructions retired/{i=$1} END{print r, u, m, i}')
                    echo "$round $load $(basename "$cli") $mode-$count $phase $figures"
                done
            done
        done
    done
done
