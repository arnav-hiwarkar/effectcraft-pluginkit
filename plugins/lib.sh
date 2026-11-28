PLUGINS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$PLUGINS_DIR")"
PROFILE="${PROFILE:-debug}"
case "$PROFILE" in
    debug) PROFILE_FLAG="" ;;
    release) PROFILE_FLAG="--release" ;;
    *)
        echo "PROFILE must be debug or release, not $PROFILE" >&2
        exit 1
        ;;
esac
APP_TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
CLI_IS_BUILT_HERE=""
if [ -z "${CLI:-}" ]; then
    CLI="$APP_TARGET/$PROFILE/effectcraft-cli"
    CLI_IS_BUILT_HERE=1
fi

build_app_packages() {
    [ -n "$CLI_IS_BUILT_HERE" ] || return 0
    (cd "$ROOT" && cargo build $PROFILE_FLAG --quiet "$@")
}

xtask_plugin() {
    (cd "$ROOT" && cargo run --quiet --package xtask -- plugin "$@")
}
