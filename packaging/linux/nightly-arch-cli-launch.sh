#!/bin/sh
set -eu

exec "$(dirname "$0")/.effectcraft-nightly/effectcraft-cli" "$@"
