#[cfg(target_os = "linux")]
use log::{info, warn};
#[cfg(target_os = "linux")]
use std::env;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::symlink;
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use std::process::Command;
#[cfg(target_os = "linux")]
use std::process::Stdio;
use std::time::Duration;

use crate::compat_helper;
use crate::install_plan::{self, InstallPlan, InstallSource, PlanContext, PlanStep};
use crate::state::AppState;
use crate::util::map_error;
#[cfg(target_os = "macos")]
use bmm_lib::errors::AppError;
use bmm_lib::local_mod_detection;
#[cfg(target_os = "macos")]
use bmm_lib::lovely;
#[cfg(target_os = "linux")]
use bmm_lib::lovely::{
    ensure_love_binary, ensure_lovely_so_exists, get_latest_lovely_version,
    merge_wine_dll_overrides, remove_installed_lovely,
};
#[cfg(any(target_os = "windows", target_os = "linux"))]
use bmm_lib::lovely::{ensure_win_injector, supports_vanilla_flag};
use bmm_lib::smods_installer::{ModInstaller, ModType};
use bmm_lib::{
    cache,
    database::{InstalledMod, SOURCE_INDEX, SOURCE_THUNDERSTORE},
};
use serde::Serialize;
#[cfg(target_os = "linux")]
use shell_words::split as split_shell_words;
use tauri::Emitter;

use crate::models::ModsChangedEvent;

/// Maximum number of retry attempts for mod installation when rate limited.
const INSTALL_MAX_RETRIES: u32 = 3;

async fn sync_compat_helper_after_mod_change(state: &tauri::State<'_, AppState>) {
    let enabled = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        db.is_compat_helper_enabled().unwrap_or(false)
    };
    if let Err(err) = compat_helper::sync_compat_helper(enabled) {
        log::warn!("Failed to sync compatibility helper after mod change: {err}");
    }
}

/// Emit installed-mods-changed with optional delta information.
fn emit_installed_mods_changed(
    app_handle: &tauri::AppHandle,
    added: Vec<String>,
    removed: Vec<String>,
) {
    let _ = app_handle.emit(
        "installed-mods-changed",
        ModsChangedEvent {
            added,
            removed,
            full_refresh: false,
        },
    );
}

fn refresh_mod_detection_cache() {
    local_mod_detection::clear_detection_cache();
}

/// Stores the Lovely version when a launch had to download Lovely, so the UI
/// does not offer an update for the release that was just installed
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn record_installed_lovely_version(
    state: &tauri::State<'_, AppState>,
    ensured: &bmm_lib::lovely::EnsuredLovely,
) {
    let Some(version) = &ensured.installed_version else {
        return;
    };
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    match db.set_lovely_version(version) {
        Ok(()) => log::info!("Recorded Lovely {version} installed during launch"),
        Err(e) => log::warn!("Failed to record Lovely version {version}: {e}"),
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn launch_balatro(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let (path_str, lovely_console_enabled, launch_mode) = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        let install_path = db
            .get_installation_path()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                AppError::InvalidState("No installation path set".to_string()).to_string()
            })?;
        let lovely_console = db.is_lovely_console_enabled().map_err(|e| e.to_string())?;
        let mode = db
            .get_launch_mode()
            .unwrap_or_else(|_| "modded".to_string());
        (install_path, lovely_console, mode)
    };

    let is_vanilla = launch_mode == "vanilla";
    let path = PathBuf::from(path_str);
    let balatro = bmm_lib::balamod::Balatro::from_custom_path(path.clone())
        .ok_or_else(|| "Stored Balatro path is no longer valid".to_string())?;

    let app_bundle = balatro
        .get_app_bundle_path()
        .ok_or_else(|| "Unable to locate Balatro app bundle".to_string())?;
    let balatro_executable = app_bundle.join("Contents/MacOS/love");
    let launch_root = balatro.path.clone();

    // Only get lovely path if launching in modded mode
    let lovely_path: Option<PathBuf> = if !is_vanilla {
        let ensured = map_error(lovely::ensure_lovely_exists().await)?;
        record_installed_lovely_version(&state, &ensured);
        Some(ensured.path)
    } else {
        None
    };

    if lovely_console_enabled {
        let command_line = if let Some(ref lovely) = lovely_path {
            format!(
                "cd '{}' && DYLD_INSERT_LIBRARIES='{}' '{}'",
                launch_root.display(),
                lovely.display(),
                balatro_executable.display()
            )
        } else {
            format!(
                "cd '{}' && '{}'",
                launch_root.display(),
                balatro_executable.display()
            )
        };

        let applescript = format!("tell application \"Terminal\" to do script \"{command_line}\"");

        Command::new("osascript")
            .arg("-e")
            .arg(applescript)
            .status()
            .map_err(|e| e.to_string())?;
    } else {
        let cmd = if let Some(ref lovely) = lovely_path {
            format!(
                "DYLD_INSERT_LIBRARIES='{}' '{}'",
                lovely.display(),
                balatro_executable.display()
            )
        } else {
            format!("'{}'", balatro_executable.display())
        };
        // Spawn the process without waiting so the UI doesn't block
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn strip_python_env(cmd: &mut Command) {
    // AppImage/runtime wrappers can leak Python env vars that break Proton's python runner.
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env_remove("PYTHONNOUSERSITE");
    cmd.env_remove("PYTHONUSERBASE");
}

#[cfg(target_os = "linux")]
fn strip_wrapper_env(cmd: &mut Command) {
    // Drop common AppImage/Snap wrappers that can poison Steam/Proton env.
    cmd.env_remove("APPIMAGE");
    cmd.env_remove("APPDIR");
    cmd.env_remove("SNAP");
    cmd.env_remove("SNAP_NAME");
    cmd.env_remove("SNAP_REVISION");
    cmd.env_remove("SNAP_INSTANCE_NAME");
    cmd.env_remove("SNAP_INSTANCE_KEY");
    cmd.env_remove("SNAP_ARCH");
    cmd.env_remove("SNAP_LIBRARY_PATH");
}

#[cfg(target_os = "linux")]
fn log_launch_env(label: &str) {
    let keys = [
        "FLATPAK_ID",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "SDL_VIDEODRIVER",
        "GDK_BACKEND",
        "MESA_GL_VERSION_OVERRIDE",
        "MESA_GLSL_VERSION_OVERRIDE",
        "VK_ICD_FILENAMES",
        "VK_DRIVER_FILES",
        "DRI_PRIME",
        "LIBGL_ALWAYS_SOFTWARE",
        "LIBGL_ALWAYS_INDIRECT",
    ];

    let mut dump = String::new();
    for key in keys {
        let val = std::env::var(key).unwrap_or_else(|_| "<unset>".to_string());
        dump.push_str(key);
        dump.push('=');
        dump.push_str(&val);
        dump.push('\n');
    }
    info!("Linux launch env ({label}):\n{dump}");
}

#[cfg(target_os = "linux")]
fn split_prefix_env(parts: &[String]) -> (Vec<(String, String)>, Vec<String>) {
    let mut envs = Vec::new();
    let mut rest = Vec::new();
    let mut in_env = true;

    for part in parts {
        if in_env
            && is_env_assignment(part)
            && let Some((key, value)) = part.split_once('=')
        {
            envs.push((key.to_string(), value.to_string()));
            continue;
        }
        in_env = false;
        rest.push(part.clone());
    }

    (envs, rest)
}

#[cfg(target_os = "linux")]
fn is_env_assignment(value: &str) -> bool {
    let Some((key, rhs)) = value.split_once('=') else {
        return false;
    };
    if key.is_empty() || rhs.is_empty() {
        return false;
    }
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(target_os = "linux")]
fn ensure_prefix_log_dir(envs: &[(String, String)]) -> Result<(), String> {
    let log_dir = envs
        .iter()
        .find(|(key, _)| key == "PROTON_LOG_DIR")
        .map(|(_, value)| value);
    let Some(log_dir) = log_dir else {
        return Ok(());
    };
    if log_dir.trim().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(log_dir)
        .map_err(|e| format!("Failed to create PROTON_LOG_DIR {log_dir}: {e}"))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_prefix_log_file() -> Result<std::fs::File, String> {
    let Some(config_dir) = dirs::config_dir() else {
        return Err("Failed to resolve config directory for prefix log".to_string());
    };
    let log_dir = config_dir.join("Balatro/logs");
    fs::create_dir_all(&log_dir)
        .map_err(|e| format!("Failed to create prefix log dir {}: {e}", log_dir.display()))?;
    let log_path = log_dir.join("prefix-launch.log");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("Failed to open prefix log file {}: {e}", log_path.display()))
}

#[cfg(target_os = "linux")]
fn validate_prefix_executable(exe: &str) -> Result<(), String> {
    if !exe.contains('/') {
        return Ok(());
    }
    let path = std::path::Path::new(exe);
    if !path.exists() {
        return Err(format!("Prefix executable not found: {}", path.display()));
    }
    if !path.is_file() {
        return Err(format!(
            "Prefix executable is not a file: {}",
            path.display()
        ));
    }
    let mode = path
        .metadata()
        .map_err(|e| format!("Failed to stat prefix executable: {e}"))?
        .permissions()
        .mode();
    if mode & 0o111 == 0 {
        return Err(format!(
            "Prefix executable is not executable: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_native_mod_dir_link() -> Result<(), String> {
    // Prefer host config/data paths even inside Flatpak, so mods/logs live on the host.
    let Some(home_dir) = dirs::home_dir() else {
        return Ok(());
    };
    let host_config = home_dir.join(".config").join("Balatro");
    let host_mods = host_config.join("Mods");

    let link_mods = |love_mods: PathBuf| -> Result<(), String> {
        if love_mods.exists() {
            if love_mods.is_symlink() {
                return Ok(());
            }
            warn!(
                "LOVE mods path already exists and is not a symlink: {}",
                love_mods.display()
            );
            return Ok(());
        }

        if let Some(parent) = love_mods.parent()
            && let Err(e) = fs::create_dir_all(parent)
        {
            return Err(format!(
                "Failed to create LOVE mods parent {}: {}",
                parent.display(),
                e
            ));
        }

        symlink(&host_mods, &love_mods).map_err(|e| {
            format!(
                "Failed to link LOVE mods dir {} -> {}: {}",
                love_mods.display(),
                host_mods.display(),
                e
            )
        })?;
        info!(
            "Linked LOVE mods dir to host: {} -> {}",
            love_mods.display(),
            host_mods.display()
        );
        Ok(())
    };

    // Ensure host mods dir exists
    if let Err(e) = fs::create_dir_all(&host_mods) {
        warn!(
            "Failed to create host mods dir {}: {}",
            host_mods.display(),
            e
        );
    }

    // Link both data and config locations that LOVE may use
    // LOVE uses game-specific subdirectories based on the identity in conf.lua
    // For Balatro, this is "Balatro", so mods should be in ~/.local/share/love/Balatro/Mods
    let love_mods_data = home_dir.join(".local/share/love/Balatro/Mods");
    let _ = link_mods(love_mods_data);
    let love_mods_config = host_config.join("love/Balatro/Mods");
    let _ = link_mods(love_mods_config);

    Ok(())
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn launch_balatro(state: tauri::State<'_, AppState>) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let (path_str, lovely_console_enabled, launch_mode, db_lovely_version) = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        let install_path = db
            .get_installation_path()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "No installation path set".to_string())?;
        let lovely_console = db.is_lovely_console_enabled().map_err(|e| e.to_string())?;
        let mode = db
            .get_launch_mode()
            .unwrap_or_else(|_| "modded".to_string());
        let version = db.get_lovely_version().ok().flatten();
        (install_path, lovely_console, mode, version)
    };

    // Get stored version, or fall back to latest from GitHub (outside mutex lock)
    let lovely_version = match db_lovely_version {
        Some(v) => Some(v),
        None => bmm_lib::lovely::get_latest_lovely_version().await.ok(),
    };

    let path = PathBuf::from(path_str);
    let is_vanilla = launch_mode == "vanilla";

    // Always ensure the injector exists so modded mode works after vanilla launch
    let ensured = ensure_win_injector(&path)
        .await
        .map_err(|e| format!("Failed to ensure Lovely injector: {e}"))?;
    record_installed_lovely_version(&state, &ensured);

    let mut cmd = Command::new(path.join("Balatro.exe"));

    // Respect the "Enable Lovely Console" setting on Windows by hiding the console
    // when disabled. If enabled, let the process manage its own console normally.
    if !lovely_console_enabled {
        // Ask Lovely to suppress its console and also prevent a console window
        // from being created for the process.
        cmd.arg("--disable-console");
        cmd.env("LOVELY_DISABLE_CONSOLE", "1");
        cmd.env("LOVELY_NO_CONSOLE", "1");
        cmd.env("LOVELY_CONSOLE", "0");
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    // Use Lovely's --vanilla flag to skip mod loading while keeping DLL in place
    // Only available in Lovely 0.9.0+
    if is_vanilla {
        if supports_vanilla_flag(lovely_version.as_deref()) {
            log::info!(
                "Adding --vanilla flag (Lovely version: {:?})",
                lovely_version
            );
            cmd.arg("--vanilla");
        } else {
            // If we can't determine version, assume latest Lovely supports --vanilla
            // This is better than silently launching with mods when user wants vanilla
            log::warn!(
                "Vanilla mode requested but Lovely version unknown ({:?}). Adding --vanilla flag anyway (assuming 0.9.0+)",
                lovely_version
            );
            cmd.arg("--vanilla");
        }
    }

    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
#[tauri::command]
pub async fn launch_balatro(_state: tauri::State<'_, AppState>) -> Result<(), String> {
    Err("Launching Balatro is not supported on this operating system".to_string())
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn launch_balatro(state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Prefer stored install path; fall back to discovered path if missing
    let install_path = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        match db.get_installation_path() {
            Ok(Some(p)) => Some(PathBuf::from(p)),
            _ => None,
        }
    };

    let path = install_path
        .or_else(|| {
            bmm_lib::finder::get_balatro_paths_cached()
                .into_iter()
                .next()
        })
        .ok_or_else(|| "No Balatro installation path is configured or detected".to_string())?;

    let (lovely_console_enabled, linux_prefix, launch_mode, db_lovely_version) = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        let lovely = db.is_lovely_console_enabled().map_err(|e| e.to_string())?;
        let prefix = db
            .get_linux_prefix()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let mode = db
            .get_launch_mode()
            .unwrap_or_else(|_| "modded".to_string());
        let version = db.get_lovely_version().ok().flatten();
        (lovely, prefix, mode, version)
    };

    // Get stored version, or fall back to latest from GitHub (outside mutex lock)
    let lovely_version = match db_lovely_version {
        Some(v) => Some(v),
        None => get_latest_lovely_version().await.ok(),
    };

    let is_vanilla = launch_mode == "vanilla";

    // Validate Balatro path
    let _balatro = bmm_lib::balamod::Balatro::from_custom_path(path.clone())
        .ok_or_else(|| "Stored Balatro path is no longer valid".to_string())?;

    let linux_prefix = linux_prefix.trim().to_string();
    if !linux_prefix.is_empty() {
        let parts = split_shell_words(&linux_prefix)
            .map_err(|e| format!("Invalid Linux prefix command: {e}"))?;
        if parts.is_empty() {
            return Err("Linux prefix command is empty".to_string());
        }
        let (envs, cmd_parts) = split_prefix_env(&parts);
        if cmd_parts.is_empty() {
            return Err("Linux prefix command missing executable".to_string());
        }
        validate_prefix_executable(&cmd_parts[0])?;
        ensure_prefix_log_dir(&envs)?;

        // Always ensure the injector exists so modded mode works after vanilla launch
        let ensured = ensure_win_injector(&path)
            .await
            .map_err(|e| format!("Failed to ensure Lovely injector: {e}"))?;
        record_installed_lovely_version(&state, &ensured);
        // winmm for Lovely 0.10+, version for an older install that is still in place
        let injector_stem = ensured
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("winmm")
            .to_string();

        bmm_lib::local_mod_detection::ensure_proton_mod_dir_link(Some(&path))?;

        let exe_path = find_executable_in_directory(&path)
            .ok_or_else(|| format!("No executable found in {}", path.display()))?;
        log_launch_env("pre-spawn (prefix)");
        info!(
            "Launching Balatro via Linux prefix (mode: {})\n  prefix_cmd={}\n  exe={}\n  cwd={}",
            launch_mode,
            linux_prefix,
            exe_path.display(),
            path.display()
        );

        let exe_arg = exe_path.to_string_lossy().to_string();
        let uses_placeholder = cmd_parts.iter().any(|part| part == "{exe}");
        let resolved_parts: Vec<String> = cmd_parts
            .iter()
            .map(|part| {
                if part == "{exe}" {
                    exe_arg.clone()
                } else {
                    part.clone()
                }
            })
            .collect();
        let is_steam_applaunch = resolved_parts.first().is_some_and(|first| {
            let last = first.rsplit('/').next().unwrap_or(first.as_str());
            (last == "steam" || last == "steam.sh")
                && resolved_parts.iter().any(|p| p == "-applaunch")
        });
        let mut launch_parts = resolved_parts.clone();
        let uses_flatpak_spawn = is_steam_applaunch && std::env::var_os("FLATPAK_ID").is_some();
        if uses_flatpak_spawn {
            // Flatpak sandbox can't see host steam directly; route via flatpak-spawn.
            if let Some(first) = launch_parts.first().cloned() {
                launch_parts = vec!["flatpak-spawn".to_string(), "--host".to_string(), first];
                launch_parts.extend(resolved_parts.iter().skip(1).cloned());
                info!("Flatpak detected; using flatpak-spawn --host for Steam launch");
            }
        }
        let append_exe = !uses_placeholder && !is_steam_applaunch;

        // Collect all environment variables that need to be passed to the launched process.
        // When using flatpak-spawn, these must be passed as --env=KEY=VALUE arguments
        // because cmd.env() only affects the flatpak-spawn process, not the host process.
        let mut launch_envs: Vec<(String, String)> = envs.clone();

        // Only set WINEDLLOVERRIDES for Lovely injection if in modded mode
        if !is_vanilla {
            let winedll_idx = launch_envs
                .iter()
                .position(|(key, _)| key.eq_ignore_ascii_case("WINEDLLOVERRIDES"));
            let existing = winedll_idx.map(|idx| launch_envs[idx].1.as_str());
            let merged = merge_wine_dll_overrides(existing, &injector_stem);
            match winedll_idx {
                None => {
                    info!("WINEDLLOVERRIDES not set; defaulting to {merged} for Lovely injection");
                    launch_envs.push(("WINEDLLOVERRIDES".to_string(), merged));
                }
                Some(idx) if launch_envs[idx].1 != merged => {
                    info!(
                        "WINEDLLOVERRIDES missing {injector_stem}; using {merged} for Lovely injection"
                    );
                    launch_envs[idx].1 = merged;
                }
                Some(_) => {}
            }
        } else {
            info!("Vanilla mode: skipping WINEDLLOVERRIDES injection");
        }

        // If using flatpak-spawn --host, inject env vars as --env=KEY=VALUE arguments
        // after --host but before the command. Otherwise, use cmd.env().
        if uses_flatpak_spawn && !launch_envs.is_empty() {
            // Find the position after "--host" to insert --env arguments
            if let Some(host_idx) = launch_parts.iter().position(|p| p == "--host") {
                let env_args: Vec<String> = launch_envs
                    .iter()
                    .map(|(k, v)| format!("--env={}={}", k, v))
                    .collect();
                // Insert env args after "--host"
                let insert_pos = host_idx + 1;
                for (i, arg) in env_args.into_iter().enumerate() {
                    launch_parts.insert(insert_pos + i, arg);
                }
                info!(
                    "Flatpak: passing {} env vars via --env arguments",
                    launch_envs.len()
                );
            }
        }

        info!(
            "Prefix exec: {} args: {:?} append_exe={}",
            launch_parts[0],
            &launch_parts[1..],
            append_exe
        );
        let mut cmd = Command::new(&launch_parts[0]);
        if launch_parts.len() > 1 {
            cmd.args(&launch_parts[1..]);
        }

        // Apply environment variables directly to the command when not using flatpak-spawn
        if !uses_flatpak_spawn {
            for (key, value) in &launch_envs {
                cmd.env(key, value);
            }
        }
        cmd.current_dir(&path);
        if append_exe {
            cmd.arg(exe_arg);
        }
        if !lovely_console_enabled {
            cmd.arg("--disable-console");
        }
        // Use Lovely's --vanilla flag to skip mod loading while keeping DLL in place
        // Only available in Lovely 0.9.0+
        if is_vanilla && supports_vanilla_flag(lovely_version.as_deref()) {
            cmd.arg("--vanilla");
            info!(
                "Adding --vanilla flag (Lovely version: {:?})",
                lovely_version
            );
        } else if is_vanilla {
            warn!(
                "Vanilla mode requested but Lovely version {:?} doesn't support --vanilla flag",
                lovely_version
            );
        }
        if let Ok(log_file) = open_prefix_log_file() {
            let stderr_file = match log_file.try_clone() {
                Ok(file) => file,
                Err(_) => {
                    // If clone fails, avoid capturing stderr rather than moving stdout handle.
                    cmd.stdout(Stdio::from(log_file));
                    return cmd
                        .spawn()
                        .map_err(|e| format!("Failed to launch via prefix: {e}"))
                        .map(|_| ());
                }
            };
            cmd.stdout(Stdio::from(log_file));
            cmd.stderr(Stdio::from(stderr_file));
        }
        cmd.spawn()
            .map_err(|e| format!("Failed to launch via prefix: {e}"))?;
        return Ok(());
    }

    // Ensure host mods map to LOVE's native mods dir
    ensure_native_mod_dir_link()?;

    // Only set up Lovely injection if in modded mode
    let lovely_so: Option<PathBuf> = if !is_vanilla {
        // Ensure Lovely's liblovely.so is present for native LOVE injection
        // Refresh Lovely if a newer version is available
        if let Ok(latest) = get_latest_lovely_version().await {
            let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(current) = db.get_lovely_version()
                && current.as_deref() != Some(latest.as_str())
            {
                let _ = db.set_lovely_version(&latest);
                // Drop the cached proxy DLLs too, the DB now claims the latest Lovely so
                // the update prompt that would replace a legacy version.dll never fires
                if let Err(e) = remove_installed_lovely(Some(path.as_path())) {
                    warn!("Failed to remove outdated Lovely files: {e}");
                }
            }
        }

        Some(
            ensure_lovely_so_exists(&path)
                .await
                .map_err(|e| format!("Failed to ensure liblovely.so: {e}"))?,
        )
    } else {
        info!("Vanilla mode: skipping Lovely injection setup");
        None
    };

    // Native LOVE launch (no Steam/Proton)
    let love_bin_env = env::var("BMM_LOVE_BIN").ok();
    let mut love_bin_path =
        PathBuf::from(love_bin_env.clone().unwrap_or_else(|| "love".to_string()));
    let mut love_lib_path: Option<PathBuf> = None;
    let mut love_available = Command::new(&love_bin_path)
        .arg("--version")
        .output()
        .is_ok();

    if !love_available {
        // Auto-download the LOVE tarball if not present on the system.
        match ensure_love_binary().await {
            Ok((bin, lib_dir)) => {
                love_bin_path = bin;
                love_lib_path = lib_dir;
                love_available = true;
            }
            Err(e) => {
                return Err(format!(
                    "LOVE is not installed and auto-download failed: {e}. Install love (e.g. sudo apt install love) or set BMM_LOVE_BIN."
                ));
            }
        }
    }

    if !love_available {
        return Err("LOVE is not installed or could not be downloaded automatically.".to_string());
    }

    // Ensure Balatro.exe is available as a .love zip for LOVE to load cleanly.
    let balatro_love = path.join("Balatro.love");
    let balatro_exe = path.join("Balatro.exe");
    if balatro_exe.exists() && !balatro_love.exists() {
        let _ = fs::copy(&balatro_exe, &balatro_love);
    }

    let mut love_cmd = Command::new(&love_bin_path);
    love_cmd.current_dir(&path).arg("Balatro.love");

    // Only set LD_PRELOAD for Lovely injection if in modded mode
    if let Some(ref lovely_so_path) = lovely_so {
        love_cmd.env("LD_PRELOAD", lovely_so_path);
    }

    // Use host config dir so mods/logs live outside the Flatpak sandbox.
    if let Some(home) = dirs::home_dir() {
        let host_config = home.join(".config").join("Balatro");
        let _ = fs::create_dir_all(&host_config);
        if env::var("FLATPAK_ID").is_ok() || env::var("XDG_CONFIG_HOME").is_err() {
            love_cmd.env("XDG_CONFIG_HOME", &host_config);
        }
    }
    // Nudge SDL to a usable video backend inside Flatpak without hard-forcing X11
    // when it's unavailable. If DISPLAY exists, prefer X11; otherwise fall back to Wayland.
    if env::var("SDL_VIDEODRIVER").is_err() && env::var("FLATPAK_ID").is_ok() {
        if env::var("DISPLAY").is_ok() {
            love_cmd.env("SDL_VIDEODRIVER", "x11");
        } else if env::var("WAYLAND_DISPLAY").is_ok() {
            love_cmd.env("SDL_VIDEODRIVER", "wayland");
        }
    }
    if let Some(ref lib_dir) = love_lib_path {
        love_cmd.env("LD_LIBRARY_PATH", lib_dir);
    }
    if !lovely_console_enabled {
        love_cmd.env("LOVELY_DISABLE_CONSOLE", "1");
        love_cmd.env("LOVELY_NO_CONSOLE", "1");
        love_cmd.env("LOVELY_CONSOLE", "0");
    }
    unsafe {
        love_cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    strip_python_env(&mut love_cmd);
    strip_wrapper_env(&mut love_cmd);
    log_launch_env("pre-spawn");
    info!(
        "Launching Balatro via LOVE (mode: {})\n  love_bin={}\n  love_lib={}\n  preload={}\n  cwd={}",
        launch_mode,
        love_bin_path.display(),
        love_lib_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        lovely_so
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        path.display()
    );
    let spawn_result = love_cmd.spawn();

    spawn_result.map_err(|e| format!("Failed to launch Balatro via native LOVE: {e}"))?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn find_executable_in_directory(dir: &PathBuf) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut executables: Vec<PathBuf> = Vec::new();

        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_file() && path.extension() == Some(std::ffi::OsStr::new("exe")) {
                executables.push(path);
            }
        }

        if executables.is_empty() {
            return None;
        }

        for exe in &executables {
            if let Some(file_name) = exe.file_name().and_then(|n| n.to_str())
                && file_name.to_lowercase().contains("balatro")
            {
                return Some(exe.clone());
            }
        }

        return Some(executables[0].clone());
    }

    None
}

#[tauri::command]
pub async fn get_steamodded_versions() -> Result<Vec<String>, String> {
    let installer = ModInstaller::new(ModType::Steamodded);
    installer
        .get_available_versions()
        .await
        .map(|versions| versions.into_iter().map(|v| v.to_string()).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn install_steamodded_version(
    state: tauri::State<'_, AppState>,
    version: String,
) -> Result<String, String> {
    let installer = ModInstaller::new(ModType::Steamodded);
    let path = installer
        .install_version(&version)
        .await
        .map_err(|e| e.to_string())?;
    sync_compat_helper_after_mod_change(&state).await;
    Ok(path)
}

#[tauri::command]
pub async fn get_talisman_versions() -> Result<Vec<String>, String> {
    let installer = ModInstaller::new(ModType::Talisman);
    installer
        .get_available_versions()
        .await
        .map(|versions| versions.into_iter().map(|v| v.to_string()).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_latest_steamodded_release() -> Result<String, String> {
    if let Ok(Some(versions)) = cache::load_versions_cache("steamodded")
        && !versions.is_empty()
    {
        let version = &versions[0];
        return Ok(format!(
            "https://github.com/Steamodded/smods/archive/refs/tags/{version}.zip"
        ));
    }

    let installer = ModInstaller::new(ModType::Steamodded);
    installer
        .get_latest_release()
        .await
        .map(|version| match installer.mod_type {
            ModType::Steamodded => {
                format!("https://github.com/Steamodded/smods/archive/refs/tags/{version}.zip")
            }
            _ => format!("https://github.com/Steamodded/smods/archive/refs/tags/{version}.zip"),
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn install_talisman_version(
    state: tauri::State<'_, AppState>,
    version: String,
) -> Result<String, String> {
    let installer = ModInstaller::new(ModType::Talisman);
    let path = installer
        .install_version(&version)
        .await
        .map_err(|e| e.to_string())?;
    sync_compat_helper_after_mod_change(&state).await;
    Ok(path)
}

#[tauri::command]
pub async fn get_dependents(mod_name: String) -> Result<Vec<String>, String> {
    let db = bmm_lib::database::Database::new().map_err(|e| e.to_string())?;
    let all_dependents = db.get_dependents(&mod_name).map_err(|e| e.to_string())?;
    let filtered: Vec<String> = all_dependents
        .into_iter()
        .filter(|d| d != &mod_name)
        .collect();
    Ok(filtered)
}

#[tauri::command]
pub async fn cascade_uninstall(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    root_mod: String,
) -> Result<(), String> {
    // Collect all mods to uninstall while holding the lock
    let mods_to_remove: Vec<(String, PathBuf)> = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut to_uninstall = vec![root_mod.clone()];
        let mut processed = std::collections::HashSet::new();
        let mut result = Vec::new();

        while let Some(current) = to_uninstall.pop() {
            if processed.contains(&current) {
                continue;
            }
            processed.insert(current.clone());

            let mod_details = map_error(db.get_mod_details(&current))?;
            let dependents = map_error(db.get_dependents(&current))?;
            to_uninstall.extend(dependents);

            let mod_path = PathBuf::from(&mod_details.path);
            result.push((current, mod_path));
        }
        result
    };

    // Perform async uninstalls without holding the lock
    for (mod_name, mod_path) in &mods_to_remove {
        if mod_path.exists() {
            map_error(bmm_lib::installer::uninstall_mod(mod_path.clone()).await)?;
        }
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        map_error(db.remove_installed_mod_by_name_or_path(mod_name, &mod_path.to_string_lossy()))?;
    }

    sync_compat_helper_after_mod_change(&state).await;
    refresh_mod_detection_cache();
    let removed_names: Vec<String> = mods_to_remove.iter().map(|(n, _)| n.clone()).collect();
    emit_installed_mods_changed(&app_handle, Vec::new(), removed_names);
    Ok(())
}

#[tauri::command]
pub async fn force_remove_mod(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    name: String,
    path: String,
) -> Result<(), String> {
    if !path.trim().is_empty() {
        let path_buf = PathBuf::from(&path);
        if path_buf.exists() {
            map_error(bmm_lib::installer::uninstall_mod(path_buf).await)?;
        }
    }
    {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        map_error(db.remove_installed_mod_by_name_or_path(&name, &path))?;
    }
    sync_compat_helper_after_mod_change(&state).await;
    refresh_mod_detection_cache();
    emit_installed_mods_changed(&app_handle, Vec::new(), vec![name]);
    Ok(())
}

#[tauri::command]
pub async fn remove_installed_mod(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    name: String,
    path: String,
) -> Result<(), String> {
    // Check for framework dependents while holding the lock
    {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());

        let is_framework = name.to_lowercase() == "steamodded" || name.to_lowercase() == "talisman";
        if is_framework {
            let all_dependents = map_error(db.get_dependents(&name))?;
            let real_deps: Vec<String> =
                all_dependents.into_iter().filter(|d| d != &name).collect();
            if !real_deps.is_empty() {
                return Err(format!(
                    "Use cascade_uninstall to remove {} with {} dependents",
                    name,
                    real_deps.len()
                ));
            }
        }
    }

    // Perform async uninstall without holding the lock
    if !path.trim().is_empty() {
        let path_buf = PathBuf::from(&path);
        if path_buf.exists() {
            map_error(bmm_lib::installer::uninstall_mod(path_buf).await)?;
        }
    }

    // Remove from database
    {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        map_error(db.remove_installed_mod_by_name_or_path(&name, &path))?;
    }

    sync_compat_helper_after_mod_change(&state).await;
    refresh_mod_detection_cache();
    emit_installed_mods_changed(&app_handle, Vec::new(), vec![name]);
    Ok(())
}

#[tauri::command]
pub async fn install_mod(
    state: tauri::State<'_, AppState>,
    url: String,
    folder_name: String,
) -> Result<PathBuf, String> {
    let folder_name_opt = if folder_name.is_empty() {
        None
    } else {
        Some(folder_name.clone())
    };

    // ts:// packages go through the Thunderstore installer for its layout fixes and
    // Namespace-Name folder, so folder_name does not apply to them
    let ts_target = if url.trim().starts_with(crate::catalog::TS_URL_SCHEME) {
        Some(crate::catalog::resolve_ts_target(&url).await?)
    } else {
        None
    };
    let resolved_url = match &ts_target {
        Some(target) => target.download_url.clone(),
        None => crate::catalog::resolve_install_url(&url).await?,
    };

    let (target, resolved_url, folder_name) = (ts_target.as_ref(), &resolved_url, &folder_name_opt);
    let path = with_install_retries(|| async move {
        match target {
            Some(target) => install_thunderstore_target(target).await,
            None => bmm_lib::installer::install_mod(resolved_url.clone(), folder_name.clone())
                .await
                .map_err(|e| e.to_string()),
        }
    })
    .await?;
    sync_compat_helper_after_mod_change(&state).await;
    Ok(path)
}

/// Runs an install, retrying with backoff while the host answers with a rate limit
async fn with_install_retries<T, F, Fut>(mut install: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut last_error: Option<String> = None;
    let mut delay = Duration::from_millis(500);
    for attempt in 1..=INSTALL_MAX_RETRIES {
        match install().await {
            Ok(value) => return Ok(value),
            Err(error_str) => {
                if is_rate_limit_error(&error_str) && attempt < INSTALL_MAX_RETRIES {
                    log::warn!(
                        "Mod install rate limited (attempt {}/{}), retrying in {:?}",
                        attempt,
                        INSTALL_MAX_RETRIES,
                        delay
                    );
                    last_error = Some(error_str);
                    tokio::time::sleep(delay).await;
                    delay = delay.saturating_mul(2);
                    continue;
                }
                return Err(error_str);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "Mod installation failed after retries".to_string()))
}

async fn install_thunderstore_target(target: &crate::catalog::TsTarget) -> Result<PathBuf, String> {
    let install = bmm_lib::installer::install_thunderstore_package(
        &target.download_url,
        &target.namespace,
        &target.name,
    )
    .await
    .map_err(|e| e.to_string())?;
    for warning in &install.warnings {
        log::warn!("{}: {}", install.folder_name, warning);
    }
    install.path.ok_or_else(|| {
        format!(
            "{} {} has no mod files to install",
            install.folder_name, target.version
        )
    })
}

/// Check if an error message indicates a rate limit (HTTP 429) error.
fn is_rate_limit_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate-limit")
        || lower.contains("ratelimit")
        || lower.contains("too many requests")
}

#[tauri::command]
pub async fn get_installed_mods_from_db(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstalledMod>, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.get_installed_mods())
}

#[tauri::command]
pub async fn add_installed_mod(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    name: String,
    path: String,
    dependencies: Vec<String>,
    current_version: String,
) -> Result<(), String> {
    let current_version = (!current_version.is_empty()).then_some(current_version);
    // Callers only record a mod after its install succeeded, so the folder the
    // previous row pointed at is now stale and safe to clean up
    let mods_dirs = installed_mods_dirs();
    let (_, removed) = record_install(
        &state.db,
        &name,
        Path::new(&path),
        &dependencies,
        current_version,
        RecordKind::Index,
        &mods_dirs,
    )
    .await?;
    if removed > 0 {
        sync_compat_helper_after_mod_change(&state).await;
    }

    refresh_mod_detection_cache();
    emit_installed_mods_changed(&app_handle, vec![name], Vec::new());
    Ok(())
}

fn installed_mods_dirs() -> Vec<PathBuf> {
    local_mod_detection::mod_dir_candidates()
        .or_else(|_| local_mod_detection::resolve_mods_dir_path().map(|p| vec![p]))
        .unwrap_or_default()
}

/// How an install is written to the database
#[derive(Debug, Clone, Copy)]
enum RecordKind<'a> {
    /// Index install that replaces the row with the same name
    Index,
    /// Thunderstore package, updates the row tracking `source_id` or adds one
    Thunderstore { source_id: &'a str },
    /// Moves the existing row `name` to another source and keeps its name
    Switch {
        name: &'a str,
        source: &'a str,
        source_id: Option<&'a str>,
    },
}

/// Records an install, then removes the folder the row pointed at before when the new
/// install lives elsewhere. Returns the row name used and how many folders were removed
async fn record_install(
    db: &std::sync::Mutex<bmm_lib::database::Database>,
    name: &str,
    path: &Path,
    dependencies: &[String],
    current_version: Option<String>,
    kind: RecordKind<'_>,
    mods_dirs: &[PathBuf],
) -> Result<(String, usize), String> {
    let path_str = path.to_string_lossy().to_string();
    let (recorded, previous_path, protected_paths) = {
        let db = db.lock().unwrap_or_else(|e| e.into_inner());
        let installed = map_error(db.get_installed_mods())?;
        let (recorded, previous) = match kind {
            RecordKind::Index => {
                let previous = installed.iter().find(|m| m.name == name);
                map_error(db.add_installed_mod(name, &path_str, dependencies, current_version))?;
                (name.to_string(), previous)
            }
            RecordKind::Thunderstore { source_id } => {
                let previous = installed.iter().find(|m| {
                    m.source == SOURCE_THUNDERSTORE
                        && m.source_id
                            .as_deref()
                            .is_some_and(|id| id.eq_ignore_ascii_case(source_id))
                });
                let recorded = map_error(db.add_installed_mod_with_source(
                    name,
                    &path_str,
                    dependencies,
                    current_version,
                    SOURCE_THUNDERSTORE,
                    Some(source_id),
                ))?;
                (recorded, previous)
            }
            RecordKind::Switch {
                name: row,
                source,
                source_id,
            } => {
                let previous = installed.iter().find(|m| m.name == row);
                map_error(db.switch_installed_mod_source(
                    row,
                    &path_str,
                    dependencies,
                    current_version,
                    source,
                    source_id,
                ))?;
                (row.to_string(), previous)
            }
        };
        let previous_path = previous.map(|m| PathBuf::from(&m.path));
        let protected_paths: Vec<PathBuf> = installed
            .iter()
            .filter(|m| m.name != recorded && !m.path.trim().is_empty())
            .map(|m| PathBuf::from(&m.path))
            .collect();
        (recorded, previous_path, protected_paths)
    };

    if mods_dirs.is_empty() {
        return Ok((recorded, 0));
    }
    let cleanup_name = recorded.clone();
    let new_path = path.to_path_buf();
    let mods_dirs = mods_dirs.to_vec();
    let cleanup = tauri::async_runtime::spawn_blocking(move || {
        bmm_lib::installer::cleanup_replaced_install(
            &cleanup_name,
            previous_path.as_deref(),
            &new_path,
            &mods_dirs,
            &protected_paths,
        )
    })
    .await;
    let removed = match cleanup {
        Ok(removed) => {
            if !removed.is_empty() {
                log::info!("Removed {} stale install(s) of {recorded}", removed.len());
            }
            removed.len()
        }
        Err(e) => {
            log::warn!("Cleanup after installing {recorded} failed: {e}");
            0
        }
    };
    Ok((recorded, removed))
}

/// Catalog, listing and local state the planner works from
struct PlanInputs {
    catalog: Vec<crate::models::ArchiveModItem>,
    packages: Vec<crate::thunderstore::TsPackage>,
    installed: Vec<InstalledMod>,
    lovely_version: Option<String>,
    steamodded_installed: bool,
    steamodded_version: Option<String>,
    talisman_installed: bool,
}

impl PlanInputs {
    fn context(&self) -> PlanContext<'_> {
        PlanContext {
            catalog: &self.catalog,
            packages: &self.packages,
            installed: &self.installed,
            lovely_version: self.lovely_version.clone(),
            steamodded_installed: self.steamodded_installed,
            steamodded_version: self.steamodded_version.clone(),
            talisman_installed: self.talisman_installed,
        }
    }
}

async fn load_plan_inputs(state: &tauri::State<'_, AppState>) -> Result<PlanInputs, String> {
    let (catalog, packages) = tokio::join!(
        crate::catalog::catalog_items(),
        crate::thunderstore::cached_packages()
    );
    let catalog = catalog?;
    let packages = packages.unwrap_or_else(|e| {
        log::warn!("Planning without Thunderstore packages: {e}");
        Vec::new()
    });
    let (installed, lovely_version) = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        (
            map_error(db.get_installed_mods())?,
            db.get_lovely_version().ok().flatten(),
        )
    };
    let mods_dir = local_mod_detection::resolve_mods_dir_path().ok();
    let (steamodded_installed, steamodded_version) =
        install_plan::installed_steamodded(&installed, mods_dir.as_deref());
    let talisman_installed = installed
        .iter()
        .any(|m| m.name.eq_ignore_ascii_case("talisman") && Path::new(&m.path).exists())
        || ModInstaller::new(ModType::Talisman).is_installed();
    Ok(PlanInputs {
        catalog,
        packages,
        installed,
        lovely_version,
        steamodded_installed,
        steamodded_version,
        talisman_installed,
    })
}

fn parse_source(source: Option<&str>) -> Result<Option<InstallSource>, String> {
    source
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(InstallSource::parse)
        .transpose()
}

/// What installing a catalog entry involves: the source used, the packages to download
/// in order, dependencies already satisfied, and the Lovely and Steamodded checks
#[tauri::command]
pub async fn plan_catalog_install(
    state: tauri::State<'_, AppState>,
    id: String,
    version: Option<String>,
    source: Option<String>,
) -> Result<InstallPlan, String> {
    let source = parse_source(source.as_deref())?;
    let inputs = load_plan_inputs(&state).await?;
    install_plan::plan_install(&inputs.context(), &id, version.as_deref(), source)
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledCatalogMod {
    pub name: String,
    pub path: PathBuf,
    pub version: String,
    pub source: InstallSource,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogInstallResult {
    pub plan: InstallPlan,
    pub installed: Vec<InstalledCatalogMod>,
    pub warnings: Vec<String>,
}

/// Installs a catalog entry with its Thunderstore dependencies. With `installSteamodded`
/// a missing or too old Steamodded is updated to the latest release first, Lovely is left
/// to the existing update flow
#[tauri::command]
pub async fn install_catalog_mod(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
    version: Option<String>,
    source: Option<String>,
    install_steamodded: Option<bool>,
) -> Result<CatalogInstallResult, String> {
    let source = parse_source(source.as_deref())?;
    let plan = {
        let inputs = load_plan_inputs(&state).await?;
        install_plan::plan_install(&inputs.context(), &id, version.as_deref(), source)?
    };
    let mods_dirs = installed_mods_dirs();
    let mut run = PlanRun {
        installed: Vec::new(),
        warnings: plan.warnings.clone(),
        removed: 0,
    };
    let result = run_plan(
        &state.db,
        &plan,
        install_steamodded.unwrap_or(false),
        &mods_dirs,
        &mut run,
    )
    .await;

    if !run.installed.is_empty() || run.removed > 0 {
        sync_compat_helper_after_mod_change(&state).await;
        refresh_mod_detection_cache();
        emit_installed_mods_changed(
            &app_handle,
            run.installed.iter().map(|m| m.name.clone()).collect(),
            Vec::new(),
        );
    }
    for warning in &run.warnings {
        log::warn!("Installing {}: {warning}", plan.mod_title);
    }
    result?;
    Ok(CatalogInstallResult {
        plan,
        installed: run.installed,
        warnings: run.warnings,
    })
}

struct PlanRun {
    installed: Vec<InstalledCatalogMod>,
    warnings: Vec<String>,
    removed: usize,
}

async fn run_plan(
    db: &std::sync::Mutex<bmm_lib::database::Database>,
    plan: &InstallPlan,
    install_steamodded: bool,
    mods_dirs: &[PathBuf],
    run: &mut PlanRun,
) -> Result<(), String> {
    if install_steamodded && plan.steamodded.as_ref().is_some_and(|s| !s.satisfied) {
        install_steamodded_release(db, "", mods_dirs, run).await?;
    }

    let mut names_by_package: std::collections::HashMap<String, String> = plan
        .satisfied
        .iter()
        .map(|s| (s.full_name.to_ascii_lowercase(), s.tracked_name.clone()))
        .collect();

    for step in &plan.steps {
        match step.source {
            InstallSource::Index if step.catalog_id == crate::catalog::STEAMODDED_ID => {
                install_steamodded_release(db, &step.version, mods_dirs, run).await?;
            }
            InstallSource::Index => install_index_step(db, step, mods_dirs, run).await?,
            InstallSource::Thunderstore => {
                if let Some((full_name, name)) =
                    install_thunderstore_step(db, step, &names_by_package, mods_dirs, run).await?
                {
                    names_by_package.insert(full_name.to_ascii_lowercase(), name);
                }
            }
        }
    }
    Ok(())
}

async fn install_steamodded_release(
    db: &std::sync::Mutex<bmm_lib::database::Database>,
    tag: &str,
    mods_dirs: &[PathBuf],
    run: &mut PlanRun,
) -> Result<(), String> {
    let installer = ModInstaller::new(ModType::Steamodded);
    let tag = if tag.trim().is_empty() {
        installer
            .get_latest_release()
            .await
            .map_err(|e| e.to_string())?
    } else {
        tag.trim().to_string()
    };
    let installer = &installer;
    let tag_ref = tag.as_str();
    let path = with_install_retries(|| async move {
        installer
            .install_version(tag_ref)
            .await
            .map_err(|e| e.to_string())
    })
    .await?;
    let path = PathBuf::from(path);
    let (name, removed) = record_install(
        db,
        "Steamodded",
        &path,
        &[],
        Some(tag.clone()),
        RecordKind::Index,
        mods_dirs,
    )
    .await?;
    run.removed += removed;
    run.installed.push(InstalledCatalogMod {
        name,
        path,
        version: tag,
        source: InstallSource::Index,
        source_id: None,
    });
    Ok(())
}

fn framework_dependencies(step: &PlanStep) -> Vec<String> {
    let mut deps = Vec::new();
    if step.requires_steamodded {
        deps.push("Steamodded".to_string());
    }
    if step.requires_talisman && step.source == InstallSource::Index {
        deps.push("Talisman".to_string());
    }
    deps
}

async fn install_index_step(
    db: &std::sync::Mutex<bmm_lib::database::Database>,
    step: &PlanStep,
    mods_dirs: &[PathBuf],
    run: &mut PlanRun,
) -> Result<(), String> {
    let url = crate::catalog::resolve_install_url(&step.download_url).await?;
    let folder = Some(step.folder_name.clone()).filter(|f| !f.trim().is_empty());
    let (url_ref, folder_ref) = (&url, &folder);
    let path = with_install_retries(|| async move {
        bmm_lib::installer::install_mod(url_ref.clone(), folder_ref.clone())
            .await
            .map_err(|e| e.to_string())
    })
    .await?;
    let kind = match (&step.tracked_name, &step.tracked_source) {
        (Some(row), Some(source)) if source != SOURCE_INDEX => RecordKind::Switch {
            name: row,
            source: SOURCE_INDEX,
            source_id: None,
        },
        _ => RecordKind::Index,
    };
    let version = (!step.version.is_empty()).then(|| step.version.clone());
    let (name, removed) = record_install(
        db,
        &step.title,
        &path,
        &framework_dependencies(step),
        version,
        kind,
        mods_dirs,
    )
    .await?;
    run.removed += removed;
    run.installed.push(InstalledCatalogMod {
        name,
        path,
        version: step.version.clone(),
        source: InstallSource::Index,
        source_id: None,
    });
    Ok(())
}

/// Returns the package and the row name it was recorded under, None for a package
/// without mod files
async fn install_thunderstore_step(
    db: &std::sync::Mutex<bmm_lib::database::Database>,
    step: &PlanStep,
    names_by_package: &std::collections::HashMap<String, String>,
    mods_dirs: &[PathBuf],
    run: &mut PlanRun,
) -> Result<Option<(String, String)>, String> {
    let full_name = step
        .full_name
        .as_deref()
        .ok_or_else(|| format!("{} has no Thunderstore package", step.title))?;
    let (namespace, name) = crate::thunderstore::split_full_name(full_name)
        .ok_or_else(|| format!("invalid package name '{full_name}'"))?;
    let url = step.download_url.as_str();
    let install = with_install_retries(|| async move {
        bmm_lib::installer::install_thunderstore_package(url, namespace, name)
            .await
            .map_err(|e| e.to_string())
    })
    .await?;
    run.warnings.extend(
        install
            .warnings
            .iter()
            .map(|w| format!("{} {}: {w}", step.title, step.version)),
    );
    let Some(path) = install.path else {
        run.warnings.push(format!(
            "{} {} has no mod files, only its dependencies were installed",
            step.title, step.version
        ));
        return Ok(None);
    };

    let mut dependencies: Vec<String> = Vec::new();
    for dep in &step.requires {
        let tracked = match names_by_package.get(&dep.to_ascii_lowercase()) {
            Some(name) => Some(name.clone()),
            None => {
                let db = db.lock().unwrap_or_else(|e| e.into_inner());
                map_error(db.get_installed_by_source_id(dep))?.map(|m| m.name)
            }
        };
        if let Some(name) = tracked
            && !dependencies.contains(&name)
        {
            dependencies.push(name);
        }
    }
    dependencies.extend(framework_dependencies(step));

    let kind = match (&step.tracked_name, &step.tracked_source) {
        (Some(row), Some(source)) if source != SOURCE_THUNDERSTORE => RecordKind::Switch {
            name: row,
            source: SOURCE_THUNDERSTORE,
            source_id: Some(full_name),
        },
        _ => RecordKind::Thunderstore {
            source_id: full_name,
        },
    };
    let (recorded, removed) = record_install(
        db,
        &step.title,
        &path,
        &dependencies,
        Some(step.version.clone()),
        kind,
        mods_dirs,
    )
    .await?;
    run.removed += removed;
    run.installed.push(InstalledCatalogMod {
        name: recorded.clone(),
        path,
        version: step.version.clone(),
        source: InstallSource::Thunderstore,
        source_id: Some(full_name.to_string()),
    });
    Ok(Some((full_name.to_string(), recorded)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_rate_limit_error_429() {
        assert!(is_rate_limit_error("HTTP 429 Too Many Requests"));
        assert!(is_rate_limit_error("Download URL not reachable (HTTP 429)"));
        assert!(is_rate_limit_error("Error 429"));
    }

    #[test]
    fn test_is_rate_limit_error_phrases() {
        assert!(is_rate_limit_error("Rate limit exceeded"));
        assert!(is_rate_limit_error("rate-limit error"));
        assert!(is_rate_limit_error("ratelimited"));
        assert!(is_rate_limit_error("Too many requests, please wait"));
    }

    #[test]
    fn test_is_rate_limit_error_false() {
        assert!(!is_rate_limit_error("Connection timeout"));
        assert!(!is_rate_limit_error("HTTP 404 Not Found"));
        assert!(!is_rate_limit_error("Network error"));
        assert!(!is_rate_limit_error(""));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_is_env_assignment_valid() {
        assert!(is_env_assignment("FOO=bar"));
        assert!(is_env_assignment("WINEDLLOVERRIDES=winmm=n,b"));
        assert!(is_env_assignment("_VAR=value"));
        assert!(is_env_assignment("VAR123=something"));
        assert!(is_env_assignment("A=B"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_is_env_assignment_invalid() {
        assert!(!is_env_assignment("notassignment"));
        assert!(!is_env_assignment("=value")); // empty key
        assert!(!is_env_assignment("KEY=")); // empty value
        assert!(!is_env_assignment("123VAR=value")); // starts with digit
        assert!(!is_env_assignment("-VAR=value")); // starts with dash
        assert!(!is_env_assignment(""));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_split_prefix_env_basic() {
        let parts = vec![
            "FOO=bar".to_string(),
            "BAZ=qux".to_string(),
            "steam".to_string(),
            "-applaunch".to_string(),
            "2379780".to_string(),
        ];
        let (envs, rest) = split_prefix_env(&parts);
        assert_eq!(envs.len(), 2);
        assert_eq!(envs[0], ("FOO".to_string(), "bar".to_string()));
        assert_eq!(envs[1], ("BAZ".to_string(), "qux".to_string()));
        assert_eq!(rest, vec!["steam", "-applaunch", "2379780"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_split_prefix_env_no_envs() {
        let parts = vec!["steam".to_string(), "-applaunch".to_string()];
        let (envs, rest) = split_prefix_env(&parts);
        assert!(envs.is_empty());
        assert_eq!(rest, vec!["steam", "-applaunch"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_split_prefix_env_all_envs() {
        let parts = vec!["A=1".to_string(), "B=2".to_string()];
        let (envs, rest) = split_prefix_env(&parts);
        assert_eq!(envs.len(), 2);
        assert!(rest.is_empty());
    }

    fn temp_db(dir: &Path) -> std::sync::Mutex<bmm_lib::database::Database> {
        std::sync::Mutex::new(
            bmm_lib::database::Database::open_at(&dir.join("bmm_storage.db")).expect("db"),
        )
    }

    fn make_mod(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("main.lua"), "-- mod").unwrap();
    }

    #[test]
    fn source_switch_keeps_the_row_and_removes_the_old_folder() {
        let td = tempfile::tempdir().unwrap();
        let mods = td.path().join("Mods");
        let old = mods.join("JokerDisplay");
        let new = mods.join("nh6574-JokerDisplay");
        make_mod(&old);
        std::fs::write(old.join(".lovelyignore"), "").unwrap();
        make_mod(&new);
        let db = temp_db(td.path());
        db.lock()
            .unwrap()
            .add_installed_mod(
                "JokerDisplay",
                &old.to_string_lossy(),
                &["Steamodded".to_string()],
                Some("d3c022d".into()),
            )
            .unwrap();

        let (name, removed) = tauri::async_runtime::block_on(record_install(
            &db,
            "JokerDisplay",
            &new,
            &[],
            Some("2.0.5".into()),
            RecordKind::Switch {
                name: "JokerDisplay",
                source: SOURCE_THUNDERSTORE,
                source_id: Some("nh6574-JokerDisplay"),
            },
            std::slice::from_ref(&mods),
        ))
        .expect("record");

        assert_eq!(name, "JokerDisplay");
        assert_eq!(removed, 1);
        assert!(!old.exists());
        assert!(new.join(".lovelyignore").exists(), "stays disabled");
        let rows = db.lock().unwrap().get_installed_mods().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, SOURCE_THUNDERSTORE);
        assert_eq!(rows[0].source_id.as_deref(), Some("nh6574-JokerDisplay"));
        assert_eq!(rows[0].current_version.as_deref(), Some("2.0.5"));
        assert_eq!(rows[0].path, new.to_string_lossy());
    }

    #[test]
    fn thunderstore_installs_get_their_own_row_next_to_an_index_mod() {
        let td = tempfile::tempdir().unwrap();
        let mods = td.path().join("Mods");
        let index_dir = mods.join("Pokermon");
        let ts_dir = mods.join("PokermonPlus-Pokermon");
        make_mod(&index_dir);
        make_mod(&ts_dir);
        let db = temp_db(td.path());
        db.lock()
            .unwrap()
            .add_installed_mod("Pokermon", &index_dir.to_string_lossy(), &[], None)
            .unwrap();

        let record = |version: &str| {
            tauri::async_runtime::block_on(record_install(
                &db,
                "Pokermon",
                &ts_dir,
                &["Pokermon".to_string()],
                Some(version.into()),
                RecordKind::Thunderstore {
                    source_id: "PokermonPlus-Pokermon",
                },
                std::slice::from_ref(&mods),
            ))
            .expect("record")
        };
        assert_eq!(record("1.1.0"), ("Pokermon (PokermonPlus)".to_string(), 0));
        // An update finds the row by package and keeps the name
        assert_eq!(record("1.2.0"), ("Pokermon (PokermonPlus)".to_string(), 0));
        assert!(index_dir.exists(), "the index mod is protected");
        let rows = db.lock().unwrap().get_installed_mods().unwrap();
        assert_eq!(rows.len(), 2);
        let ts = rows
            .iter()
            .find(|r| r.source == SOURCE_THUNDERSTORE)
            .unwrap();
        assert_eq!(ts.current_version.as_deref(), Some("1.2.0"));
    }

    #[test]
    fn test_emit_functions_exist() {
        // Simple test to verify the helper functions compile and exist
        // The actual emit functionality requires Tauri runtime
        fn _check_refresh_exists() {
            super::refresh_mod_detection_cache();
        }
    }
}
