#!/usr/bin/env bash
# Builds the Windows x64 installer into dist/windows
# On Windows (Git Bash) it builds natively and produces NSIS and MSI
# On macOS or Linux it cross compiles with cargo-xwin and produces NSIS only (unsigned)
# Cross build needs: rustup target x86_64-pc-windows-msvc, cargo-xwin, llvm, lld, nsis (makensis)
# SKIP_FRONTEND=1 reuses an existing ./build instead of running the frontend build
# RELEASE_VERSION=x.y.z sets the bundle and app version and names artifacts for a release (see common.sh)
# MSI is skipped when RELEASE_VERSION is not WiX compatible (anything beyond x.y.z or x.y.z-N)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/common.sh
source "$ROOT/scripts/common.sh"
TARGET="x86_64-pc-windows-msvc"
OUT="$ROOT/dist/windows"
BUNDLE="$ROOT/target/$TARGET/release/bundle"

cd "$ROOT"
bun install --frozen-lockfile

CONFIG_ARGS=()
CONFIG_JSON="$(tauri_config_json)"
if [ -n "$CONFIG_JSON" ]; then
  CONFIG_ARGS=(-c "$CONFIG_JSON")
fi

# Stale installers from older versions would confuse the copy below
rm -rf "$BUNDLE/nsis" "$BUNDLE/msi"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    NATIVE_BUNDLES="nsis,msi"
    if [ -n "${RELEASE_VERSION:-}" ] && ! msi_compatible "$RELEASE_VERSION"; then
      echo "Skipping MSI: WiX cannot encode version $RELEASE_VERSION, NSIS only" >&2
      NATIVE_BUNDLES="nsis"
    fi
    bun tauri build --target "$TARGET" --bundles "$NATIVE_BUNDLES" ${CONFIG_ARGS[@]+"${CONFIG_ARGS[@]}"}
    ;;
  *)
    rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"
    for tool in cargo-xwin makensis; do
      command -v "$tool" >/dev/null || { echo "Missing $tool, see the header of this script" >&2; exit 1; }
    done
    # clang-cl, llvm-rc and llvm-lib come from llvm, lld-link from lld (keg only on Homebrew)
    if command -v brew >/dev/null; then
      for formula in llvm lld; do
        prefix="$(brew --prefix "$formula" 2>/dev/null || true)"
        [ -d "$prefix/bin" ] && PATH="$prefix/bin:$PATH"
      done
      export PATH
    fi
    export XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-$HOME/.cache/xwin}"
    bun tauri build --runner cargo-xwin --target "$TARGET" --bundles nsis ${CONFIG_ARGS[@]+"${CONFIG_ARGS[@]}"}
    ;;
esac

rm -rf "$OUT" && mkdir -p "$OUT"
if [ -n "${RELEASE_VERSION:-}" ]; then
  cp "$BUNDLE"/nsis/*.exe "$OUT/$(asset_name windows-x64-setup.exe)"
  if compgen -G "$BUNDLE/msi/*.msi" >/dev/null; then
    cp "$BUNDLE"/msi/*.msi "$OUT/$(asset_name windows-x64.msi)"
  fi
else
  cp "$BUNDLE"/nsis/*.exe "$OUT/"
  if compgen -G "$BUNDLE/msi/*.msi" >/dev/null; then
    cp "$BUNDLE"/msi/*.msi "$OUT/"
  fi
fi

echo "Windows artifacts in $OUT"
ls -lh "$OUT"
