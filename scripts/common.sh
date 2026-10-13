# shellcheck shell=bash
# Sourced by the build-*.sh scripts
# RELEASE_VERSION=1.2.3 (or v1.2.3) switches on release mode: the bundle version and the in app
# version come from it and artifacts get names like Balatro-Mod-Manager-1.2.3-macos-universal.dmg
# Without it the build keeps Tauri's default artifact names and the app shows the git tag or commit

ASSET_PREFIX="Balatro-Mod-Manager"

if [ -n "${RELEASE_VERSION:-}" ]; then
  RELEASE_VERSION="${RELEASE_VERSION#v}"
  if ! [[ "$RELEASE_VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
    echo "RELEASE_VERSION '$RELEASE_VERSION' is not semver (expected x.y.z or x.y.z-pre)" >&2
    exit 1
  fi
  export RELEASE_VERSION
  export BMM_VERSION="${BMM_VERSION:-$RELEASE_VERSION}"
fi

# JSON for `tauri build -c`, empty when nothing needs overriding
tauri_config_json() {
  local parts=()
  if [ "${SKIP_FRONTEND:-0}" = "1" ]; then
    parts+=('"build":{"beforeBuildCommand":""}')
  fi
  if [ -n "${RELEASE_VERSION:-}" ]; then
    parts+=("\"version\":\"$RELEASE_VERSION\"")
  fi
  if [ "${#parts[@]}" -gt 0 ]; then
    local IFS=,
    printf '{%s}' "${parts[*]}"
  fi
}

# Release asset name for a platform suffix, e.g. asset_name macos-universal.dmg
asset_name() {
  printf '%s-%s-%s' "$ASSET_PREFIX" "$RELEASE_VERSION" "$1"
}

# WiX only takes major.minor.patch with an optional numeric prerelease, see tauri-bundler
msi_compatible() {
  local v="$1"
  [[ "$v" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)(-([0-9]+))?$ ]] || return 1
  [ "${BASH_REMATCH[1]}" -le 255 ] && [ "${BASH_REMATCH[2]}" -le 255 ] \
    && [ "${BASH_REMATCH[3]}" -le 65535 ] && [ "${BASH_REMATCH[5]:-0}" -le 65535 ]
}
