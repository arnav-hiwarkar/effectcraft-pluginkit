# shellcheck shell=bash
HELPER_PACKAGE=effectcraft-plugin-helper
HELPER_NAME=craft-plugin-helper

require_helper_beside() {
  local app
  for app in "$@"; do
    [[ -x "$(dirname "$app")/$HELPER_NAME" ]] || { echo "error: $HELPER_NAME is not beside $app" >&2; exit 1; }
  done
  return
}
