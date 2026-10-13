# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Balatro Mod Manager is a standalone desktop application for managing mods for the game Balatro. Built with Tauri 2, it combines a Rust backend with a SvelteKit frontend to provide a native cross-platform experience.

**Tech Stack:**

- **Backend**: Rust (Tauri 2.9+)
- **Frontend**: SvelteKit 2 + Svelte 5 + TypeScript
- **Build Tool**: Bun (package manager), Task (task runner)
- **Database**: SQLite (via rusqlite, bundled)
- **Platforms**: Windows, macOS (Apple Silicon only), Linux (Flatpak)

## Development Commands

### Initial Setup

```bash
bun install --allow-scripts    # Install frontend dependencies
task bootstrap                 # Install all dependencies (frontend + backend)
```

### Development

```bash
task debug                     # Start development server (alias: task dev)
task dev:web                   # Start web-only dev server (Vite, without Tauri)
```

### Building

```bash
# Platform-specific builds (automatically selects based on OS)
task release-windows           # Windows only
task build:macos               # macOS arm64 .app and .dmg into dist/macos (ARCH=universal|x86_64 also work)
task release-linux             # Linux (produces AppImage/Deb/RPM)

# Flatpak builds (Linux)
task flatpak:build             # Build Flatpak repo from local checkout
task flatpak:bundle            # Create .flatpak bundle
task flatpak:install           # Install bundle to user profile
task flatpak:run               # Run the installed Flatpak app
task flatpak:all               # Build, bundle, install, and run
```

### Testing & Linting

```bash
task check                     # Full typecheck (frontend) + lint + test (Rust)
task test                      # Run Rust tests for both crates
task fmt                       # Format Rust and frontend code
task fmt-rust                  # Format Rust code only
```

**Note**: Tests must run with `--test-threads=1` to avoid database lock conflicts (already configured in CI and task commands).

### Dependency Management

```bash
task update-frontend-deps      # Update Bun/npm dependencies
task update-backend-deps       # Update Cargo dependencies (requires cargo-edit)
task update-dependencies       # Update both frontend and backend
```

### Cleanup

```bash
task clean                     # Clean Rust build artifacts
task clean-ui                  # Clean frontend build artifacts (.svelte-kit, build, dist)
task flatpak:clean             # Remove Flatpak build artifacts
```

## Architecture

### Crate Structure

The project uses a **workspace structure** with two Rust crates:

1. **`src-tauri/`** (main binary: `balatro-mod-manager`)
   - Tauri application entry point
   - Exposes Tauri commands to the frontend
   - Manages application state, window lifecycle, and plugins
   - Contains `commands/` module with organized command handlers

2. **`src-tauri/bmm-lib/`** (library: `bmm-lib`)
   - Core business logic (platform-agnostic)
   - Modules: `database`, `installer`, `finder`, `lovely`, `balamod`, `discord_rpc`, `cache`, `local_mod_detection`, `smods_installer`, `mod_collections`, `logging`
   - Used by the main Tauri app but testable independently

### Frontend Architecture

**SvelteKit SSG (Static Site Generation)**

- Uses `adapter-static` for prerendering (no server-side runtime)
- Routes: `/` (setup/picker) and `/main` (main app view)
- All routes prerendered at build time (configured in `svelte.config.js`)

**State Management**

- Svelte stores in `src/stores/`:
  - `modStore.ts`: Mod catalog, install states, pagination, uninstall dialogs
  - `collections.ts`: Mod collections (curated sets)
  - `descriptions.ts`: Mod descriptions cache
  - `modCache.ts`: Mod image/thumbnail cache
  - `ui.ts`: UI state (sidebar, view modes, settings)
  - `update.ts`: App update notifications
- LocalStorage for persistence (mod catalog, thumbnails, settings)

**Key Frontend Patterns**

- Components in `src/components/` and `src/components/viewblock/`
- Tauri invoke wrapper in `src/utils/tauriInvoke.ts` for type-safe backend calls
- Image caching system in `src/utils/image-cache.ts` and background thumbnail queue

### Backend Architecture

**Tauri Commands** (`src-tauri/src/commands/`)

- Organized by domain: `paths`, `install`, `system`, `detection`, `cache`, `settings`, `lovely`, `repo`, `thumbnails`, `mods`, `import`, `external`, `report`
- All commands registered in `src-tauri/src/lib.rs` via `invoke_handler!`

**Core Library Modules** (`src-tauri/bmm-lib/src/`)

- `database.rs`: SQLite schema, migrations, CRUD operations
- `finder.rs`: Detects Balatro installation paths (Steam, custom, Linux prefix)
- `installer.rs`: Mod installation/uninstallation logic
- `lovely.rs`: Lovely (mod loader) installer and version management
- `balamod.rs`: Balamod (alternative mod loader) support
- `smods_installer.rs`: Steamodded/Talisman installer
- `local_mod_detection.rs`: Scans Mods folder, detects untracked mods
- `discord_rpc.rs`: Discord Rich Presence integration
- `mod_collections.rs`: Curated mod collection support
- `cache.rs`: Binary cache for remote mod index
- `logging.rs`: Centralized logging setup

**Background Tasks**

- Auto-reindex loop: Periodically validates installed mods, detects folder changes, emits `installed-mods-changed` events
- Thumbnail queue: Background downloads and caching of mod images

**Database Schema**

- SQLite database in app data directory
- Tables: `mods` (installed mods), `settings` (key-value config), `mod_collections`, `lovely_versions`, etc.
- Migrations handled in `database.rs`

### Platform-Specific Notes

**macOS**

- Releases ship arm64 only; universal builds (`scripts/build-macos.sh universal`) need both `aarch64-apple-darwin` and `x86_64-apple-darwin` targets
- `MACOSX_DEPLOYMENT_TARGET=11.0` (minimum macOS Big Sur)

**Linux**

- Flatpak is the recommended distribution method
- Requires `flatpak-builder` and runtimes: GNOME 47, Node 20, Rust stable
- Linux builds detect Steam via Flatpak or native paths
- Wayland compositor issues: app defaults to X11 (XWayland); set `BMM_ALLOW_WAYLAND=1` to override

**Windows**

- Target: `x86_64-pc-windows-msvc`
- Windows builds are unsigned

## Key Workflows

### Adding a New Tauri Command

1. Add command function to appropriate module in `src-tauri/src/commands/` (e.g., `install.rs`)
2. Register command in `src-tauri/src/lib.rs` in the `invoke_handler!` macro
3. Call from frontend using `invoke()` from `@tauri-apps/api/core` or the wrapper in `src/utils/tauriInvoke.ts`

### Modifying the Database Schema

1. Update schema in `src-tauri/bmm-lib/src/database.rs`
2. Add migration logic in `Database::new()` or dedicated migration function
3. Bump schema version if necessary
4. Test with `cargo test -p bmm-lib -- --test-threads=1`

### Adding Frontend State

1. Define store in `src/stores/` (use `writable()` from `svelte/store`)
2. Add persistence logic if needed (localStorage pattern in `modStore.ts`)
3. Import and use in components via `$storeStore` syntax

### Running Tests

- **Frontend**: No automated tests currently; use `bun run check` for type checking
- **Backend**: `cargo test -p bmm-lib -- --test-threads=1` and `cargo test -p balatro-mod-manager -- --test-threads=1`
- **CI**: GitHub Actions runs all checks (see `.github/workflows/ci.yml`)

## Important Constraints

- **Single-threaded tests**: Database tests must use `--test-threads=1` to avoid SQLite lock conflicts
- **No Xbox Game Pass support**: Balatro Mod Manager is incompatible with the Xbox Game Pass version of Balatro
- **Flatpak sandbox**: Linux builds run in Flatpak sandbox; file access limited to XDG directories and Steam paths
- **SvelteKit prerendering**: All routes must be listed in `svelte.config.js` `prerender.entries` or risk navigation issues in production builds

## Debugging

- **Development mode**: `task debug` opens DevTools automatically (macOS/Linux/Windows)
- **Logs**: Check `src-tauri/bmm-lib/src/logging.rs` for log configuration; logs stored in platform app data directory
- **Database inspection**: SQLite DB in `<app_data_dir>/balatro-mod-manager/` (use `sqlite3` CLI or DB browser)
- **Tauri IPC**: Use browser DevTools Console to inspect `window.__TAURI__` and `invoke()` calls

## External Integrations

- **Mod Index**: Fetches mod catalog from remote repository (binary cache format)
- **Discord RPC**: Optional Discord Rich Presence (enable/disable in settings)
- **Steam**: Detects Steam library folders via `libraryfolders.vdf` parsing
- **Lovely**: Mod loader injector (bundled/downloaded as needed)
