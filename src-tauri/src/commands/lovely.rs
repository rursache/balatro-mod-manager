#[cfg(target_os = "macos")]
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use bmm_lib::errors::AppError;
use bmm_lib::lovely;

use crate::state::AppState;

/// Check whether Lovely is currently installed/present on this system.
/// - macOS: checks for `~/Library/Application Support/Balatro/bins/liblovely.dylib` (via config dir)
/// - Windows/Linux (Proton/Wine): checks for `winmm.dll` (or a legacy `version.dll`) in the Balatro game directory
#[tauri::command]
pub async fn is_lovely_installed(_state: tauri::State<'_, AppState>) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
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
        // Prefer database install path if present, then the first detected Balatro path
        let db_path = {
            let db = _state.db.lock().unwrap_or_else(|e| e.into_inner());
            db.get_installation_path().map_err(|e| e.to_string())?
        };
        let Some(dir) = lovely::resolve_game_dir(db_path.as_deref()) else {
            return Ok(false);
        };

        // Linux can also use liblovely.so for native LOVE
        #[cfg(target_os = "linux")]
        {
            if lovely::injector_artifact_exists(&dir.join(lovely::LINUX_SO)) {
                return Ok(true);
            }
        }
        Ok(lovely::win_injector_installed(&dir))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        // Other targets: Lovely injector not managed; do not warn.
        Ok(true)
    }
}

#[tauri::command]
pub async fn check_lovely_update(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        // Unsupported target; skip.
        return Ok(None);
    }

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

/// Installs the latest Lovely release over the current one. The download is
/// pinned to the tag that gets recorded and the old injector is only replaced
/// once the new one is fully written
#[tauri::command]
pub async fn update_lovely_to_latest(state: tauri::State<'_, AppState>) -> Result<String, String> {
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        return Err(
            "Lovely injection is only supported on Windows, macOS, and Linux/Proton.".into(),
        );
    }

    let latest = lovely::get_latest_lovely_version()
        .await
        .map_err(|e| e.to_string())?;

    let db_path = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        db.get_installation_path().ok().flatten()
    };
    let game_dir = lovely::resolve_game_dir(db_path.as_deref());

    let installed = lovely::install_lovely(Some(&latest), game_dir.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    log::info!("Installed Lovely {latest} at {}", installed.display());

    // Persist version
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db.set_lovely_version(&latest).map_err(|e| e.to_string())?;

    Ok(latest)
}
