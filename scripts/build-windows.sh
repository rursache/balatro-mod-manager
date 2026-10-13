#!/usr/bin/env bash
# Builds the Windows x64 installer into dist/windows
# On Windows (Git Bash) it builds natively and produces NSIS and MSI
# On macOS or Linux it cross compiles with cargo-xwin and produces NSIS only (unsigned)
# Cross build needs: rustup target x86_64-pc-windows-msvc, cargo-xwin, llvm, lld, nsis (makensis)
# SKIP_FRONTEND=1 reuses an existing ./build instead of running the frontend build
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="x86_64-pc-windows-msvc"
OUT="$ROOT/dist/windows"

cd "$ROOT"
bun install --frozen-lockfile

CONFIG_ARGS=()
if [ "${SKIP_FRONTEND:-0}" = "1" ]; then
  CONFIG_ARGS=(-c '{"build":{"beforeBuildCommand":""}}')
fi

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    bun tauri build --target "$TARGET" ${CONFIG_ARGS[@]+"${CONFIG_ARGS[@]}"}
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

BUNDLE="$ROOT/target/$TARGET/release/bundle"
rm -rf "$OUT" && mkdir -p "$OUT"
cp "$BUNDLE"/nsis/*.exe "$OUT/"
if compgen -G "$BUNDLE/msi/*.msi" >/dev/null; then
  cp "$BUNDLE"/msi/*.msi "$OUT/"
fi

echo "Windows artifacts in $OUT"
ls -lh "$OUT"
