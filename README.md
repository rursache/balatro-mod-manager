# [![Balatro Mod Manager](images/title.svg)](#)

Balatro Mod Manager is a standalone tool for [Balatro](https://store.steampowered.com/app/2379780/Balatro/) that makes finding, downloading and installing mods easy

![GitHub Downloads (all assets, all releases)](https://img.shields.io/github/downloads/rursache/balatro-mod-manager/total)
![GitHub License](https://img.shields.io/github/license/rursache/balatro-mod-manager)
![GitHub Tag](https://img.shields.io/github/v/tag/rursache/balatro-mod-manager)
![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/rursache/balatro-mod-manager/ci.yml)

![Rust](https://img.shields.io/badge/rust-%23000000.svg?logo=rust&logoColor=white)
![TypeScript](https://img.shields.io/badge/typescript-%23007ACC.svg?logo=typescript&logoColor=white)
![Fortran](https://img.shields.io/badge/Fortran-%23734F96.svg?logo=fortran&logoColor=white)
![Tauri](https://img.shields.io/badge/tauri-%2324C8DB.svg?logo=tauri&logoColor=%23FFFFFF)
![Svelte](https://img.shields.io/badge/svelte-%23f1413d.svg?logo=svelte&logoColor=white)

## About this fork

The original Balatro Mod Manager by Efe "Skyline" was discontinued and its servers were shut down. I liked it too much to let it go, so I picked it up and keep it going here

- Mods come from the community [balatro-mod-index](https://github.com/kasimeka/balatro-mod-index) and from [Thunderstore](https://thunderstore.io/c/balatro/), merged into one catalog, so nothing depends on the old servers anymore
- Multiplayer has its own tab and installs the official packages from Thunderstore (casual play, ranked still needs the official BET launcher)
- Releases, issues and updates live in this repository

Mods are listed by submitting them to the [index repository](https://github.com/kasimeka/balatro-mod-index) or by publishing them on Thunderstore, see [CONTRIBUTING.md](CONTRIBUTING.md)

<p align="center">
    <a href="https://star-history.com/#rursache/balatro-mod-manager&Date">
        <picture>
            <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=rursache/balatro-mod-manager&type=Date&theme=dark" />
            <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=rursache/balatro-mod-manager&type=Date" />
            <img width="75%" alt="Star History Chart" src="https://api.star-history.com/svg?repos=rursache/balatro-mod-manager&type=Date" />
        </picture>
    </a>
</p>

# [![Download](images/download.svg)](https://github.com/rursache/balatro-mod-manager/releases/latest)

Balatro Mod Manager runs on Windows, macOS and Linux and needs no extra libraries

> Note: Balatro Mod Manager is **NOT** compatible with the Xbox Game Pass version of Balatro

Grab the installer for your system from the **Assets** section of the [latest release](https://github.com/rursache/balatro-mod-manager/releases/latest)

- Windows: `Balatro-Mod-Manager-<version>-windows-x64-setup.exe` or `Balatro-Mod-Manager-<version>-windows-x64.msi`
- macOS: `Balatro-Mod-Manager-<version>-macos-arm64.dmg` (or the `.zip` with the app inside), or Homebrew
- Linux: `Balatro-Mod-Manager-<version>-linux.flatpak`, `-linux-amd64.AppImage`, `-linux-amd64.deb` or `-linux-x86_64.rpm`

Every release also ships `checksums.txt` with the SHA-256 of each file

### macOS

The macOS build is for Apple Silicon, signed with a Developer ID and notarized by Apple, so it opens without any Gatekeeper workaround

- Homebrew:
  ```bash
  brew install --cask rursache/tap/balatro-mod-manager
  ```
- Or open the `.dmg` and drag **Balatro Mod Manager** into `/Applications`

### Debian / Ubuntu

Download the `.deb` from the [latest release](https://github.com/rursache/balatro-mod-manager/releases/latest) and install it:

```bash
sudo apt install ./Balatro-Mod-Manager-*-linux-amd64.deb
```

### Flatpak (Steam Deck / Linux)

Download the `.flatpak` from the [latest release](https://github.com/rursache/balatro-mod-manager/releases/latest), then:

```bash
flatpak install --user Balatro-Mod-Manager-*-linux.flatpak
flatpak run io.balatro.ModManager
```

Or let the install script fetch the latest release Flatpak (it builds locally if none is available):

```bash
curl -sL https://raw.githubusercontent.com/rursache/balatro-mod-manager/main/scripts/linux-install.sh | bash -s -- --clone
```

On Linux the manager currently supports the Steam version of Balatro only

# [![Build](images/build.svg)](#build-prerequisites)

If you prefer building it yourself instead of using the [prebuilt installers](https://github.com/rursache/balatro-mod-manager/releases/latest), here is how

## Build Prerequisites

- [Rust via rustup](https://rustup.rs/) for the backend
- [Bun](https://bun.sh/) for the frontend, it also provides the Tauri CLI through `bun tauri`
- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS
- [Task](https://taskfile.dev/) for the task commands
  - **Arch Linux**: `sudo pacman -S go-task` (named `go-task` to avoid conflicts)
  - **Fedora**: `sudo dnf install go-task`
  - **Ubuntu/Debian**: the [official script](https://taskfile.dev/installation/#install-script) or `sudo snap install task --classic`
  - **macOS**: `brew install go-task`
  - **Windows**: `choco install go-task` or `winget install Task.Task`

## Building from source

1. Clone the repository and install the frontend dependencies:
   ```sh
   git clone https://github.com/rursache/balatro-mod-manager.git
   cd balatro-mod-manager && bun install
   ```
2. Build for your platform (see [Building Releases](#building-releases)):
   ```sh
   task build:macos   # or build:windows, build:linux
   ```

The install scripts do the same in one go:

- Windows (PowerShell): `iwr https://raw.githubusercontent.com/rursache/balatro-mod-manager/main/scripts/install.ps1 -useb | iex`
- macOS: `curl -sL https://raw.githubusercontent.com/rursache/balatro-mod-manager/main/scripts/install.sh | bash`
- Linux (Flatpak, from a checkout): `./scripts/linux-install.sh`

## Running the Project

### Development Mode

```sh
task debug
```

> Linux/Wayland: on Wayland sessions the app prefers X11 (XWayland) to avoid compositor protocol errors. Set `BMM_ALLOW_WAYLAND=1` before running to keep native Wayland

### Building Releases

Every platform has one script in `scripts/` that writes its artifacts to `dist/<platform>/`. The Taskfile wraps them and the release workflow calls the same scripts

| Task | Script | Output | Host requirements |
|---|---|---|---|
| `task build:macos` (`ARCH=arm64\|universal\|x86_64`, default arm64) | `scripts/build-macos.sh` | `.dmg` and zipped `.app` | macOS with Xcode, `rustup target add aarch64-apple-darwin` (plus `x86_64-apple-darwin` for universal or Intel builds) |
| `task build:windows` | `scripts/build-windows.sh` | NSIS `-setup.exe` (plus `.msi` on Windows) | Windows natively, or macOS/Linux with `rustup target add x86_64-pc-windows-msvc`, `cargo install --locked cargo-xwin`, LLVM, LLD and NSIS (`brew install llvm lld makensis`) |
| `task build:linux` | `scripts/build-linux.sh` | `.deb`, `.AppImage`, `.rpm` | Docker (OrbStack or Docker Desktop on macOS), the image comes from `packaging/Dockerfile` |
| `task build:all` | all of the above | everything | macOS with all of the above |

Local builds are ad hoc signed on macOS and unsigned on Windows. Set `APPLE_SIGNING_IDENTITY` (and `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` to notarize) before `build:macos` to sign with a real identity. `SKIP_FRONTEND=1` reuses an existing `build/` folder and `BUNDLES=app scripts/build-macos.sh` only builds the `.app` into `dist/macos`

The version shown in the app comes from git: the tag on `HEAD` (without the leading `v`), otherwise the short commit hash, otherwise `dev`. `BMM_VERSION=1.2.3` overrides it. Only tagged builds check GitHub for app updates

`RELEASE_VERSION=1.2.3` switches the scripts to release mode: the bundle version and the in app version become `1.2.3` and the artifacts get release names like `Balatro-Mod-Manager-1.2.3-macos-arm64.dmg`. Without it the bundle keeps the static version from `tauri.conf.json` and Tauri's default file names

### Publishing a release

Publishing a GitHub release runs `.github/workflows/release.yml`, which builds every platform with the scripts above, signs and notarizes the macOS app, uploads the assets and `checksums.txt` to the release and updates the Homebrew cask (stable releases only). Running the workflow by hand with a tag and `dry_run` builds, signs and notarizes without touching the release, the files stay attached to the workflow run. MSI installers are skipped for tags WiX cannot encode (anything other than `x.y.z` or `x.y.z-N`)

Cross compiling to Windows relies on a patched copy of `unrar_sys` in `src-tauri/vendor/unrar_sys` (see `PATCHED.md` there)

## Cleaning the Build

```sh
task clean
```

## Credits

- Efe "Skyline" ([@skyline69](https://github.com/skyline69)) created Balatro Mod Manager and maintained it until it was discontinued, this fork builds on his work
- The community run [balatro-mod-index](https://github.com/kasimeka/balatro-mod-index) and [Thunderstore](https://thunderstore.io/c/balatro/) provide the mod catalog
- Font by Daniel Linssen

## License

GPL-3.0, see [LICENSE](LICENSE)
