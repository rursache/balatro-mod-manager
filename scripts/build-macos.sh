#!/usr/bin/env bash
# Builds the macOS .app and .dmg into dist/macos
# Usage: scripts/build-macos.sh [universal|arm64|x86_64]   (default: universal)
# Signing: set APPLE_SIGNING_IDENTITY (and APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID to notarize), otherwise ad hoc
# SKIP_FRONTEND=1 reuses an existing ./build instead of running the frontend build
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="${1:-universal}"
OUT="$ROOT/dist/macos"

case "$ARCH" in
  universal) TARGET="universal-apple-darwin"; NEEDED="aarch64-apple-darwin x86_64-apple-darwin" ;;
  arm64)     TARGET="aarch64-apple-darwin";   NEEDED="aarch64-apple-darwin" ;;
  x86_64)    TARGET="x86_64-apple-darwin";    NEEDED="x86_64-apple-darwin" ;;
  *) echo "Unknown arch '$ARCH', use universal, arm64 or x86_64" >&2; exit 1 ;;
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
if [ "${SKIP_FRONTEND:-0}" = "1" ]; then
  CONFIG_ARGS=(-c '{"build":{"beforeBuildCommand":""}}')
fi

# CI=true skips the Finder automation used to style the DMG window
MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}" CI="${CI:-true}" \
  bun tauri build --target "$TARGET" --bundles app,dmg ${CONFIG_ARGS[@]+"${CONFIG_ARGS[@]}"}

BUNDLE="$ROOT/target/$TARGET/release/bundle"
rm -rf "$OUT" && mkdir -p "$OUT"
cp "$BUNDLE"/dmg/*.dmg "$OUT/"
APP="$(find "$BUNDLE/macos" -maxdepth 1 -name '*.app' | head -n 1)"
ditto -c -k --keepParent "$APP" "$OUT/$(basename "$APP" .app)_${ARCH}.app.zip"

echo "macOS artifacts in $OUT"
ls -lh "$OUT"
