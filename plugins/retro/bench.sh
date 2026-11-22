#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
rounds=${ROUNDS:-10}
out=${OUT:-$here/target/bench}
jobs=${JOBS:-4}
export CARGO_TARGET_DIR="$root/target"
mkdir -p "$out/wasm" "$out/native"

cargo build -j "$jobs" --release --quiet --target wasm32-unknown-unknown --manifest-path "$here/Cargo.toml" --target-dir "$here/target"
cargo build -j "$jobs" --release --quiet --manifest-path "$here/Cargo.toml" --target-dir "$here/target"
cp "$here/target/wasm32-unknown-unknown/release/retro.wasm" "$out/wasm/plugin.wasm"
cp "$here/plugin.json" "$out/wasm/plugin.json"
lib=$(ls "$here/target/release/" | grep -E '^libretro\.(dylib|so)$|^retro\.dll$' | head -n 1)
cp "$here/target/release/$lib" "$out/native/$lib"
target="$(uname -s | tr '[:upper:]' '[:lower:]' | sed 's/darwin/macos/')-$(uname -m | sed 's/arm64/aarch64/')"
sed "s|\"entry\": { \"wasm\": \"plugin.wasm\" }|\"entry\": { \"native\": { \"$target\": \"$lib\" } }|" "$here/plugin.json" > "$out/native/plugin.json"

cargo run -j "$jobs" --release --quiet --manifest-path "$here/check/Cargo.toml" --example card -- "$out/card.png" 1920 1080
cargo build -j "$jobs" --release --quiet --manifest-path "$root/Cargo.toml" -p craft-plugin-host --example render
render="$CARGO_TARGET_DIR/release/examples/render"

echo "1920x1080, minimum of $rounds runs each; render times exclude the copy into the frame"
for effect in crt neon; do
  for mode in wasm native; do
    line=$("$render" "$out/$mode" "$effect" "$out/$effect-$mode.png" --input "$out/card.png" --rounds "$rounds" --time 0.5)
    label=$([ "$mode" = wasm ] && echo "cpu sandboxed" || echo "cpu native")
    echo "$effect $label: $line"
  done
done
cargo run -j "$jobs" --release --quiet --manifest-path "$here/check/Cargo.toml" --example gpu -- "$rounds"
