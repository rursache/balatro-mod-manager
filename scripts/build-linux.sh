#!/usr/bin/env bash
# Builds Linux x86_64 bundles (deb, AppImage, rpm) into dist/linux
# Runs inside Docker (image from packaging/Dockerfile) so it works the same on macOS, Linux and CI
# SKIP_FRONTEND=1 reuses an existing ./build instead of running the frontend build
# RELEASE_VERSION=x.y.z sets the bundle and app version and names artifacts for a release (see common.sh)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/common.sh
source "$ROOT/scripts/common.sh"
IMAGE="${BMM_LINUX_IMAGE:-bmm-linux-builder}"
OUT="$ROOT/dist/linux"

command -v docker >/dev/null || { echo "Docker is required (OrbStack or Docker Desktop on macOS)" >&2; exit 1; }

cd "$ROOT"
docker build --platform linux/amd64 -t "$IMAGE" packaging

# The frontend is platform independent, so build it on the host and skip it in the container
if [ "${SKIP_FRONTEND:-0}" != "1" ]; then
  bun install --frozen-lockfile
  bun run build
fi

# Git in the container cannot always read the host checkout, so build.rs gets the version from here
if [ -z "${BMM_VERSION:-}" ]; then
  BMM_VERSION="$(git -C "$ROOT" describe --tags --exact-match HEAD 2>/dev/null \
    || git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || true)"
  BMM_VERSION="${BMM_VERSION#v}"
fi

# The container never runs the frontend build, version overrides ride along
TAURI_CONFIG="$(SKIP_FRONTEND=1 tauri_config_json)"

# Named volumes keep Linux build output and the cargo cache out of the host target dir
docker run --rm --platform linux/amd64 \
  -v "$ROOT":/src \
  -v bmm-linux-target:/target \
  -v bmm-linux-cargo-registry:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/target \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  -e TAURI_CONFIG="$TAURI_CONFIG" \
  ${BMM_VERSION:+-e BMM_VERSION="$BMM_VERSION"} \
  "$IMAGE" \
  bash -c 'set -e
    B=/target/x86_64-unknown-linux-gnu/release/bundle
    rm -rf "$B/deb" "$B/appimage" "$B/rpm"
    cargo tauri build --target x86_64-unknown-linux-gnu --bundles deb,appimage,rpm -c "$TAURI_CONFIG"
    rm -rf /src/dist/linux && mkdir -p /src/dist/linux
    cp "$B"/deb/*.deb "$B"/appimage/*.AppImage "$B"/rpm/*.rpm /src/dist/linux/
    chown "$HOST_UID:$HOST_GID" /src/dist
    chown -R "$HOST_UID:$HOST_GID" /src/dist/linux'

if [ -n "${RELEASE_VERSION:-}" ]; then
  mv "$OUT"/*.deb "$OUT/$(asset_name linux-amd64.deb)"
  mv "$OUT"/*.AppImage "$OUT/$(asset_name linux-amd64.AppImage)"
  mv "$OUT"/*.rpm "$OUT/$(asset_name linux-x86_64.rpm)"
fi

echo "Linux artifacts in $OUT"
ls -lh "$OUT"
