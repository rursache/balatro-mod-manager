#!/usr/bin/env bash
# Builds Linux x86_64 bundles (deb, AppImage, rpm) into dist/linux
# Runs inside Docker (image from packaging/Dockerfile) so it works the same on macOS, Linux and CI
# SKIP_FRONTEND=1 reuses an existing ./build instead of running the frontend build
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
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

# Named volumes keep Linux build output and the cargo cache out of the host target dir
docker run --rm --platform linux/amd64 \
  -v "$ROOT":/src \
  -v bmm-linux-target:/target \
  -v bmm-linux-cargo-registry:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/target \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  "$IMAGE" \
  bash -c 'set -e
    cargo tauri build --target x86_64-unknown-linux-gnu --bundles deb,appimage,rpm -c "{\"build\":{\"beforeBuildCommand\":\"\"}}"
    B=/target/x86_64-unknown-linux-gnu/release/bundle
    rm -rf /src/dist/linux && mkdir -p /src/dist/linux
    cp "$B"/deb/*.deb "$B"/appimage/*.AppImage "$B"/rpm/*.rpm /src/dist/linux/
    chown "$HOST_UID:$HOST_GID" /src/dist
    chown -R "$HOST_UID:$HOST_GID" /src/dist/linux'

echo "Linux artifacts in $OUT"
ls -lh "$OUT"
