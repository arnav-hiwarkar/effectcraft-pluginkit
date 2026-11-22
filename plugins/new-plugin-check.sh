#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/new-plugin-check}"
jobs="${JOBS:-4}"
kinds=(effect gpu-effect panel tool task)
cleanup() {
    for kind in "${kinds[@]}"; do rm -rf "${ROOT:?}/plugins/check-$kind"; done
}
trap cleanup EXIT
cleanup
for kind in "${kinds[@]}"; do
    name="check-$kind"
    cargo run --quiet --package xtask -- plugin new "$name" --kind "$kind" > /dev/null
    cargo test -j "$jobs" --quiet --manifest-path "plugins/$name/Cargo.toml"
    cargo build -j "$jobs" --quiet --release --target wasm32-unknown-unknown --manifest-path "plugins/$name/Cargo.toml"
    echo "PASS $kind"
done
cargo test -j "$jobs" --quiet --manifest-path plugins/cookbook/Cargo.toml
echo "PASS cookbook"
