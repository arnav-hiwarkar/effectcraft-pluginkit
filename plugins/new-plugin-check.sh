#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/new-plugin-check}"
jobs="${JOBS:-4}"
kinds=(effect gpu-effect panel tool task)
name_of() {
    if [ "$1" = effect ]; then echo settings; else echo "$1"; fi
}
for kind in "${kinds[@]}"; do
    if [ -e "plugins/$(name_of "$kind")" ]; then
        echo "plugins/$(name_of "$kind") exists; this check makes and removes that folder" >&2
        exit 1
    fi
done
cleanup() {
    for kind in "${kinds[@]}"; do rm -rf "${ROOT:?}/plugins/$(name_of "$kind")"; done
}
trap cleanup EXIT
for kind in "${kinds[@]}"; do
    name="$(name_of "$kind")"
    cargo run --quiet --package xtask -- plugin new "$name" --kind "$kind" > /dev/null
    cargo test -j "$jobs" --quiet --manifest-path "plugins/$name/Cargo.toml"
    cargo build -j "$jobs" --quiet --release --target wasm32-unknown-unknown --manifest-path "plugins/$name/Cargo.toml"
    echo "PASS $kind"
done
cargo test -j "$jobs" --quiet --manifest-path plugins/cookbook/Cargo.toml
echo "PASS cookbook"
