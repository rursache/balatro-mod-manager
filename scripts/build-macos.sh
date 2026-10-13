#!/usr/bin/env bash
# Builds the macOS .app and .dmg into dist/macos
# Usage: scripts/build-macos.sh [universal|arm64|x86_64]   (default: universal)
# Signing: set APPLE_SIGNING_IDENTITY (and APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID to notarize), otherwise ad hoc
# SKIP_FRONTEND=1 reuses an existing ./build instead of running the frontend build
# BUNDLES=app only builds the .app and copies it to dist/macos (CI signs and packages it afterwards)
# RELEASE_VERSION=x.y.z sets the bundle and app version and names artifacts for a release (see common.sh)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/common.sh
source "$ROOT/scripts/common.sh"
ARCH="${1:-universal}"
OUT="$ROOT/dist/macos"
BUNDLES="${BUNDLES:-app,dmg}"

case "$ARCH" in
  universal) TARGET="universal-apple-darwin"; NEEDED="aarch64-apple-darwin x86_64-apple-darwin" ;;
  arm64)     TARGET="aarch64-apple-darwin";   NEEDED="aarch64-apple-darwin" ;;
  x86_64)    TARGET="x86_64-apple-darwin";    NEEDED="x86_64-apple-darwin" ;;
  *) echo "Unknown arch '$ARCH', use universal, arm64 or x86_64" >&2; exit 1 ;;
esac

case "$BUNDLES" in
  app|app,dmg|dmg,app) ;;
  *) echo "Unknown BUNDLES '$BUNDLES', use app or app,dmg" >&2; exit 1 ;;
esac

if [ "$(uname -s)" != "Darwin" ]; then
  echo "macOS builds need a macOS host" >&2
  exit 1
fi

for t in $NEEDED; do
  rustup target list --installed | grep -qx "$t" || rustup target add "$t"
done

cd "$ROOT"
bun install --frozen-lockfile

CONFIG_ARGS=()
CONFIG_JSON="$(tauri_config_json)"
if [ -n "$CONFIG_JSON" ]; then
  CONFIG_ARGS=(-c "$CONFIG_JSON")
fi

BUNDLE="$ROOT/target/$TARGET/release/bundle"
# Stale bundles from older versions would confuse the copy below
rm -rf "$BUNDLE/macos" "$BUNDLE/dmg"

# CI=true skips the Finder automation used to style the DMG window
MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}" CI="${CI:-true}" \
  bun tauri build --target "$TARGET" --bundles "$BUNDLES" ${CONFIG_ARGS[@]+"${CONFIG_ARGS[@]}"}

rm -rf "$OUT" && mkdir -p "$OUT"
APP="$(find "$BUNDLE/macos" -maxdepth 1 -name '*.app' | head -n 1)"

if [ "$BUNDLES" = "app" ]; then
  ditto "$APP" "$OUT/$(basename "$APP")"
elif [ -n "${RELEASE_VERSION:-}" ]; then
  cp "$BUNDLE"/dmg/*.dmg "$OUT/$(asset_name "macos-$ARCH.dmg")"
  ditto -c -k --keepParent "$APP" "$OUT/$(asset_name "macos-$ARCH.zip")"
else
  cp "$BUNDLE"/dmg/*.dmg "$OUT/"
  ditto -c -k --keepParent "$APP" "$OUT/$(basename "$APP" .app)_${ARCH}.app.zip"
fi

echo "macOS artifacts in $OUT"
ls -lh "$OUT"
