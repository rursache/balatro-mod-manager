use std::path::PathBuf;

use serde::Serialize;

#[cfg(target_os = "macos")]
use bmm_lib::errors::AppError;
use bmm_lib::lovely;

use crate::state::AppState;
use crate::util::map_error;

/// Combined app initialization data returned in a single IPC call.
/// Replaces multiple separate calls: get_app_version, check_existing_installation,
/// is_security_warning_acknowledged, is_lovely_installed, check_lovely_update, get_launch_mode.
#[derive(Serialize)]
pub struct AppInitData {
    pub version: String,
    pub existing_installation: Option<String>,
    pub security_acknowledged: bool,
    pub lovely_installed: bool,
    pub lovely_update_available: Option<String>,
    pub launch_mode: String,
}

/// Single IPC call that returns all data needed to initialize the app.
/// This batches 6 separate calls into 1, reducing startup IPC overhead.
#[tauri::command]
pub async fn get_app_init_data(state: tauri::State<'_, AppState>) -> Result<AppInitData, String> {
    // 1. App version (compile-time constant, no DB access needed)
    let version = env!("CARGO_PKG_VERSION").to_string();

    // 2-5. Extract all DB values in one scoped block
    let (existing_installation, security_acknowledged, lovely_installed, launch_mode) = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());

        // 2. Check existing installation
        let existing_installation = if let Some(path) = db.get_installation_path()? {
            let path_buf = PathBuf::from(&path);
            if bmm_lib::balamod::Balatro::from_custom_path(path_buf).is_some() {
                Some(path)
            } else {
                map_error(db.remove_installation_path())?;
                None
            }
        } else {
            None
        };

        // 3. Security warning acknowledged
        let security_acknowledged = map_error(db.is_security_warning_acknowledged())?;

        // 4. Is lovely installed
        let lovely_installed = check_lovely_installed_inner(&db, &existing_installation)?;

        // 5. Launch mode
        let launch_mode = map_error(db.get_launch_mode())?;

        (
            existing_installation,
            security_acknowledged,
            lovely_installed,
            launch_mode,
        )
    }; // guard dropped here

    // 6. Check lovely update (network call)
    let lovely_update_available = check_lovely_update_inner(&state).await?;

    Ok(AppInitData {
        version,
        existing_installation,
        security_acknowledged,
        lovely_installed,
        lovely_update_available,
        launch_mode,
    })
}

/// Combined settings data returned in a single IPC call.
/// Replaces separate calls: get_discord_rpc_status, get_lovely_console_status,
/// get_background_state, get_compat_helper_status, get_linux_prefix, get_launch_mode.
#[derive(Serialize)]
pub struct AllSettings {
    pub discord_rpc: bool,
    pub lovely_console: bool,
    pub background_enabled: bool,
    pub compat_helper: bool,
    pub linux_prefix: String,
    pub launch_mode: String,
    pub analytics_enabled: bool,
}

/// Single IPC call that returns all settings for the Settings page.
/// This batches 6 separate calls into 1.
#[tauri::command]
pub async fn get_all_settings(state: tauri::State<'_, AppState>) -> Result<AllSettings, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());

    let discord_rpc = db.is_discord_rpc_enabled().map_err(|e| e.to_string())?;
    let lovely_console = map_error(db.is_lovely_console_enabled())?;
    let background_enabled = map_error(db.get_background_enabled())?;
    let compat_helper = map_error(db.is_compat_helper_enabled())?;
    let linux_prefix = db
        .get_linux_prefix()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let launch_mode = map_error(db.get_launch_mode())?;
    let analytics_enabled = db.is_analytics_enabled().map_err(|e| e.to_string())?;

    Ok(AllSettings {
        discord_rpc,
        lovely_console,
        background_enabled,
        compat_helper,
        linux_prefix,
        launch_mode,
        analytics_enabled,
    })
}

// Internal helpers to avoid code duplication

fn check_lovely_installed_inner(
    db: &bmm_lib::database::Database,
    existing_installation: &Option<String>,
) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = (db, existing_installation);
        let config_dir = dirs::config_dir()
            .ok_or_else(|| AppError::DirNotFound(PathBuf::from("config directory")).to_string())?;
        let lovely_path = config_dir
            .join("Balatro")
            .join("bins")
            .join(lovely::MAC_DYLIB);
        Ok(lovely::injector_artifact_exists(&lovely_path))
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        // Linux keeps cached copies that survive Steam file verification
        #[cfg(target_os = "linux")]
        {
            if let Some(bins_dir) = dirs::config_dir().map(|c| c.join("Balatro/bins")) {
                let cached_dll = lovely::win_injector_installed(&bins_dir);
                let cached_so = lovely::injector_artifact_exists(&bins_dir.join(lovely::LINUX_SO));
                if cached_dll || cached_so {
                    log::debug!(
                        "Found cached Lovely: injector dll={cached_dll}, liblovely.so={cached_so}"
                    );
                    return Ok(true);
                }
            }
        }

        // Use existing installation if already validated, then the DB path, then detection
        let db_path = match existing_installation {
            Some(path) => Some(path.clone()),
            None => db.get_installation_path().map_err(|e| e.to_string())?,
        };
        let Some(dir) = lovely::resolve_game_dir(db_path.as_deref()) else {
            log::info!("No Balatro paths found for Lovely detection");
            return Ok(false);
        };

        let dll_state = lovely::detect_win_injector(&dir);
        log::debug!("Lovely injector state at {}: {dll_state:?}", dir.display());
        #[cfg(target_os = "linux")]
        {
            if lovely::injector_artifact_exists(&dir.join(lovely::LINUX_SO)) {
                return Ok(true);
            }
        }
        Ok(dll_state != lovely::WinInjectorState::None)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (db, existing_installation);
        Ok(true)
    }
}

async fn check_lovely_update_inner(
    state: &tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = state;
        return Ok(None);
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        // Load latest from GitHub
        let latest = lovely::get_latest_lovely_version()
            .await
            .map_err(|e| e.to_string())?;

        // Compare to DB-stored version
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        match db.get_lovely_version() {
            Ok(Some(installed)) => {
                if installed.trim() != latest {
                    Ok(Some(latest))
                } else {
                    Ok(None)
                }
            }
            Ok(None) => Ok(Some(latest)), // Missing setting implies update/reinstall needed
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmm_lib::database::Database;
    use std::ffi::OsStr;
    use tempfile::tempdir;

    fn set_var(key: &str, val: impl AsRef<OsStr>) {
        unsafe { std::env::set_var(key, val) };
    }

    fn remove_var(key: &str) {
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn test_app_init_data_serialization() {
        // Verify struct can be serialized to JSON
        let data = AppInitData {
            version: "0.3.8".to_string(),
            existing_installation: Some("/path/to/balatro".to_string()),
            security_acknowledged: true,
            lovely_installed: true,
            lovely_update_available: None,
            launch_mode: "modded".to_string(),
        };

        let json = serde_json::to_string(&data).expect("should serialize");
        assert!(json.contains("\"version\":\"0.3.8\""));
        assert!(json.contains("\"security_acknowledged\":true"));
        assert!(json.contains("\"lovely_installed\":true"));
        assert!(json.contains("\"launch_mode\":\"modded\""));
    }

    #[test]
    fn test_all_settings_serialization() {
        // Verify struct can be serialized to JSON
        let settings = AllSettings {
            discord_rpc: true,
            lovely_console: false,
            background_enabled: true,
            compat_helper: false,
            linux_prefix: "steam -applaunch 2379780".to_string(),
            launch_mode: "modded".to_string(),
            analytics_enabled: true,
        };

        let json = serde_json::to_string(&settings).expect("should serialize");
        assert!(json.contains("\"discord_rpc\":true"));
        assert!(json.contains("\"lovely_console\":false"));
        assert!(json.contains("\"background_enabled\":true"));
        assert!(json.contains("\"compat_helper\":false"));
        assert!(json.contains("\"linux_prefix\":\"steam -applaunch 2379780\""));
        assert!(json.contains("\"analytics_enabled\":true"));
    }

    #[test]
    fn test_check_lovely_installed_inner_no_installation() {
        // Set up temp environment
        let td = tempdir().unwrap();
        let original_cfg = std::env::var_os("XDG_CONFIG_HOME");
        let original_home = std::env::var_os("HOME");
        set_var("XDG_CONFIG_HOME", td.path());
        if cfg!(target_os = "macos") {
            set_var("HOME", td.path());
        }

        let db = Database::new().expect("db");

        // With no installation path, should return false (no lovely found)
        let result = check_lovely_installed_inner(&db, &None);
        assert!(result.is_ok());
        // On non-configured systems, lovely is not installed
        // (the specific result depends on platform)

        // Cleanup
        match original_cfg {
            Some(val) => set_var("XDG_CONFIG_HOME", val),
            None => remove_var("XDG_CONFIG_HOME"),
        }
        match original_home {
            Some(val) => set_var("HOME", val),
            None => remove_var("HOME"),
        }
    }

    #[test]
    fn test_all_settings_defaults() {
        // Set up temp environment
        let td = tempdir().unwrap();
        let original_cfg = std::env::var_os("XDG_CONFIG_HOME");
        let original_home = std::env::var_os("HOME");
        set_var("XDG_CONFIG_HOME", td.path());
        if cfg!(target_os = "macos") {
            set_var("HOME", td.path());
        }

        let db = Database::new().expect("db");

        // Test default values from a fresh database
        let discord_rpc = db.is_discord_rpc_enabled().unwrap();
        let lovely_console = db.is_lovely_console_enabled().unwrap();
        let background = db.get_background_enabled().unwrap();
        let compat_helper = db.is_compat_helper_enabled().unwrap();
        let linux_prefix = db.get_linux_prefix().unwrap().unwrap_or_default();
        let launch_mode = db.get_launch_mode().unwrap();

        // Verify we can read all settings without errors
        // Use the values to ensure they're valid (suppress unused warnings)
        let _ = (
            discord_rpc,
            lovely_console,
            background,
            compat_helper,
            linux_prefix,
        );
        assert!(!launch_mode.is_empty()); // launch mode should have a default

        // Cleanup
        match original_cfg {
            Some(val) => set_var("XDG_CONFIG_HOME", val),
            None => remove_var("XDG_CONFIG_HOME"),
        }
        match original_home {
            Some(val) => set_var("HOME", val),
            None => remove_var("HOME"),
        }
    }
}
