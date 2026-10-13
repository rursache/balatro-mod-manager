# Linux x86_64 build image for Tauri v2 (deb, AppImage, rpm)
# Ubuntu 22.04 keeps the glibc baseline low for the AppImage
FROM --platform=linux/amd64 ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    APPIMAGE_EXTRACT_AND_RUN=1 \
    NO_STRIP=1

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential ca-certificates curl wget file pkg-config patchelf xdg-utils \
    clang libclang-dev cmake nasm \
    libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev \
    libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev \
    libssl-dev libdbus-1-dev rpm \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && cargo install --locked tauri-cli --version "^2" \
    && rm -rf /usr/local/cargo/registry

WORKDIR /src/src-tauri

# The frontend is built on the host first, so skip beforeBuildCommand here
CMD ["cargo", "tauri", "build", "--target", "x86_64-unknown-linux-gnu", "--bundles", "deb,appimage,rpm", "-c", "{\"build\":{\"beforeBuildCommand\":\"\"}}"]
