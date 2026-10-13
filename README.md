# [![Balatro Mod Manager](images/title.svg)](#)

Balatro Mod Manager is a standalone tool made for [Balatro](https://store.steampowered.com/app/2379780/Balatro/) that makes finding, downloading, and installing mods easy

Maintained by [Radu Ursache](https://radu.ursache.ro) ([@rursache](https://github.com/rursache)). Originally created by Efe "Skyline" ([@skyline69](https://github.com/skyline69))

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

The original Balatro Mod Manager by Efe "Skyline" was discontinued and its servers were shut down. Radu Ursache picked it up and keeps it going in this fork

- The mod catalog now comes straight from [kasimeka/balatro-mod-index](https://github.com/kasimeka/balatro-mod-index), so it no longer depends on the old servers
- Thunderstore support is coming
- Releases, issues and updates live in [rursache/balatro-mod-manager](https://github.com/rursache/balatro-mod-manager)

Mods are listed by submitting them to the [index repository](https://github.com/kasimeka/balatro-mod-index), see [CONTRIBUTING.md](CONTRIBUTING.md)

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

Balatro Mod Manager is available for Windows, macOS, and Linux. The installer is standalone and does not require any external libraries.

> Note: The Balatro Mod Manager is **NOT** compatible with the Xbox Gamepass version of Balatro

[Download the Balatro Mod Manager installer here](https://github.com/rursache/balatro-mod-manager/releases/latest).

Scroll down to find **▸Assets** and download the right version of the installer for your system.

- Windows: `Balatro.Mod.Manager_…_x64-setup.exe` or `Balatro.Mod.Manager_…_x64_en-US.msi`
- macOS: `Balatro.Mod.Manager_…_universal.dmg`
- Linux: Flatpak (recommended), AUR, or `Balatro.Mod.Manager_…_amd64.AppImage`

### Installing on macOS

The macOS build is signed ad-hoc (not notarized by Apple), so Gatekeeper blocks the first launch. After installing once, the app opens normally on every subsequent launch.

1. Mount the `.dmg` and drag **Balatro Mod Manager** into `/Applications`.
2. Try to open it. macOS will refuse with either *"Apple cannot verify..."* or *"is damaged and can't be opened"*. Close the dialog.
3. Pick one of the following bypasses:

   **Option A, System Settings (macOS 14 and newer, no Terminal):**
   - Open **System Settings → Privacy & Security**.
   - Scroll to the **Security** section. You should see a line about Balatro Mod Manager being blocked, with an **Open Anyway** button. Click it and confirm.
   - If no button appears (macOS 15+ sometimes hides it for "damaged" bundles), use Option B.

   **Option B, Terminal one-liner (works on every macOS version):**
   ```bash
   xattr -dr com.apple.quarantine "/Applications/Balatro Mod Manager.app"
   ```
   Then double-click the app normally.

   **Option C, older macOS (13 and below):** right-click the app → **Open** → confirm in the dialog.

Why this happens: shipping notarized builds requires an active Apple Developer Program membership ($99/year). The app is ad-hoc signed so it runs on Apple Silicon, but Apple's notarization service is what would silence Gatekeeper. The friction is one-time per install.

### Arch Linux (AUR)

Install using your preferred AUR helper:

```bash
yay -S balatro-mod-manager-bin
# or
paru -S balatro-mod-manager-bin
```

### Debian / Ubuntu

Download the `.deb` from the [latest release](https://github.com/rursache/balatro-mod-manager/releases/latest) and install:

```bash
sudo apt install ./Balatro.Mod.Manager_*_amd64.deb
```

## Flatpak (Steam Deck/Linux)

> You need [flatpak-builder](https://docs.flatpak.org/en/latest/flatpak-builder.html) for this.

- Run from a local checkout:
  ```bash
  git clone https://github.com/rursache/balatro-mod-manager.git
  cd balatro-mod-manager
  ```
- Install runtimes once (GNOME 50 + toolchain extensions):
  ```bash
  flatpak install org.gnome.Platform//50 org.gnome.Sdk//50 \
    org.freedesktop.Sdk.Extension.node20//25.08 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08
  ```
- Build + bundle from this repo:
  ```bash
  flatpak-builder --force-clean --repo=repo build-dir packaging/flatpak/io.balatro.ModManager.json
  flatpak build-bundle repo balatro-mod-manager.flatpak io.balatro.ModManager master
  ```
- Install/run (on the Deck or any Flatpak host):
  ```bash
  flatpak install --user balatro-mod-manager.flatpak
  flatpak run io.balatro.ModManager
  ```
  AppImage/Deb/RPM still land in `target/release/bundle/` during the Flatpak build if you need them.

# [![Build](images/build.svg)](#build-prerequisites)

Alternatively, if you would prefer to build Balatro Mod Manager yourself instead of downloading the [prebuilt installer](https://github.com/rursache/balatro-mod-manager/releases/latest), Balatro Mod Manager can be compiled from source using the instructions below.

## Build Prerequisites

- [Rust via rustup](https://rustup.rs/) (for the backend)
- [Bun](https://bun.sh/) (for the frontend, also provides the Tauri CLI through `bun tauri`)
- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS
- [Task](https://taskfile.dev/) (for running task commands)
  - **Arch Linux**: `sudo pacman -S go-task` (package is named `go-task` to avoid conflicts)
  - **Fedora**: `sudo dnf install go-task`
  - **Ubuntu/Debian**: Install via the [official script](https://taskfile.dev/installation/#install-script) or Snap: `sudo snap install task --classic`
  - **macOS**: `brew install go-task`
  - **Windows**: `choco install go-task` or `winget install Task.Task`

## Automatic Installation

### For Windows

open Powershell & run this command:

```powershell
iwr https://raw.githubusercontent.com/rursache/balatro-mod-manager/main/scripts/install.ps1 -useb | iex
```

### For macOS

run this command:

```bash
curl -sL https://raw.githubusercontent.com/rursache/balatro-mod-manager/main/scripts/install.sh | bash
```

### For Linux (Flatpak)

From a local checkout (respects your current branch/changes):

```bash
./scripts/linux-install.sh
```

If you want to install from the latest GitHub main without cloning manually, pass `--clone`:

```bash
curl -sL https://raw.githubusercontent.com/rursache/balatro-mod-manager/main/scripts/linux-install.sh | bash -s -- --clone
```

The script now prefers the latest GitHub Release Flatpak if available, and falls back to a local build if not.

Launch after install:

```bash
flatpak run io.balatro.ModManager
```

Linux builds use Flatpak, so you’ll need `flatpak` and `flatpak-builder` installed.
On Linux the manager currently supports the Steam version of Balatro only.

## Manual Installation

1. Clone the repository & install bun's dependencies:
   ```sh
   git clone https://github.com/rursache/balatro-mod-manager.git
   cd balatro-mod-manager && bun install
   ```
2. Build for your platform (see [Building Releases](#building-releases))
   ```sh
   task build:macos   # or build:windows, build:linux
   ```

## Running the Project

### Development Mode

To start the project in development mode, use the provided taskfile:

1. Run the debug target:
   ```sh
   task debug
   ```

> Linux/Wayland: on Wayland sessions the app now prefers X11 (XWayland) to avoid compositor protocol errors. Set `BMM_ALLOW_WAYLAND=1` before running if you want to keep native Wayland.

### Building Releases

Every platform has one script in `scripts/` that writes its artifacts to `dist/<platform>/`. The Taskfile wraps them, and CI is meant to call the same scripts

| Task | Script | Output | Host requirements |
|---|---|---|---|
| `task build:macos` (`ARCH=universal\|arm64\|x86_64`) | `scripts/build-macos.sh` | `.dmg` and zipped `.app` | macOS with Xcode, `rustup target add aarch64-apple-darwin x86_64-apple-darwin` |
| `task build:windows` | `scripts/build-windows.sh` | NSIS `-setup.exe` (plus `.msi` on Windows) | Windows natively, or macOS/Linux with `rustup target add x86_64-pc-windows-msvc`, `cargo install --locked cargo-xwin`, LLVM, LLD and NSIS (`brew install llvm lld makensis`) |
| `task build:linux` | `scripts/build-linux.sh` | `.deb`, `.AppImage`, `.rpm` | Docker (OrbStack or Docker Desktop on macOS), the image comes from `packaging/Dockerfile` |
| `task build:all` | all of the above | everything | macOS with all of the above |

Builds are ad hoc signed on macOS and unsigned on Windows. Set `APPLE_SIGNING_IDENTITY` (and `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` to notarize) before `build:macos` to sign with a real identity. `SKIP_FRONTEND=1` reuses an existing `build/` folder

Cross compiling to Windows relies on a patched copy of `unrar_sys` in `src-tauri/vendor/unrar_sys` (see `PATCHED.md` there)

## Cleaning the Build

To clean the build files, use the provided taskfile:

1. Run the clean target:
   ```sh
   task clean
   ```

## Credits

- Efe "Skyline" ([@skyline69](https://github.com/skyline69)) created Balatro Mod Manager and maintained it until it was discontinued. This fork builds on his work
- The mod catalog is the community run [balatro-mod-index](https://github.com/kasimeka/balatro-mod-index)
- Font by Daniel Linssen

## License

GPL-3.0, see [LICENSE](LICENSE)
