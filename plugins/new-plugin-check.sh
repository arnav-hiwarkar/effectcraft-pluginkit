#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/new-plugin-check}"
names=(check-effect check-panel check-gpu check-both)
flags=("" "--panel" "--gpu" "--panel --gpu")
cleanup() {
    for name in "${names[@]}"; do rm -rf "$ROOT/plugins/$name"; done
}
trap cleanup EXIT
cleanup
for i in "${!names[@]}"; do
    name="${names[$i]}"
    # shellcheck disable=SC2086
    cargo run --quiet --package xtask -- plugin new "$name" ${flags[$i]} > /dev/null
    cargo test --quiet --manifest-path "plugins/$name/Cargo.toml"
    cargo build --quiet --release --target wasm32-unknown-unknown --manifest-path "plugins/$name/Cargo.toml"
    echo "PASS $name ${flags[$i]}"
done
