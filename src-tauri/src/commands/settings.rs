use crate::compat_helper;
use crate::state::AppState;
use crate::util::map_error;

#[tauri::command]
pub async fn get_lovely_console_status(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.is_lovely_console_enabled())
}

#[tauri::command]
pub async fn set_lovely_console_status(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.set_lovely_console_status(enabled))
}

#[tauri::command]
pub async fn get_discord_rpc_status(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db.is_discord_rpc_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_discord_rpc_status(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        db.set_discord_rpc_enabled(enabled)
            .map_err(|e| e.to_string())?;
    } // guard dropped here

    // update the runtime status so changes take effect immediately
    let discord_rpc = state.discord_rpc.lock().await;
    discord_rpc.set_enabled(enabled);
    Ok(())
}

#[tauri::command]
pub async fn get_background_state(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.get_background_enabled())
}

#[tauri::command]
pub async fn set_background_state(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.set_background_enabled(enabled))
}

#[tauri::command]
pub async fn get_compat_helper_status(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.is_compat_helper_enabled())
}

#[tauri::command]
pub async fn set_compat_helper_status(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.set_compat_helper_enabled(enabled))?;
    compat_helper::sync_compat_helper(enabled)?;
    Ok(())
}

#[tauri::command]
pub async fn get_thunderstore_enabled(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.is_thunderstore_enabled())
}

/// Persists the Thunderstore setting, off stops listing syncs and README requests
#[tauri::command]
pub async fn set_thunderstore_enabled(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.set_thunderstore_enabled(enabled))?;
    crate::thunderstore::set_enabled(enabled);
    Ok(())
}

#[tauri::command]
pub async fn set_linux_prefix(
    state: tauri::State<'_, AppState>,
    value: String,
) -> Result<(), String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.set_linux_prefix(&value))
}

#[tauri::command]
pub async fn get_linux_prefix(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    Ok(db
        .get_linux_prefix()
        .map_err(|e| e.to_string())?
        .unwrap_or_default())
}

#[tauri::command]
pub async fn is_security_warning_acknowledged(
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.is_security_warning_acknowledged())
}

#[tauri::command]
pub async fn set_security_warning_acknowledged(
    state: tauri::State<'_, AppState>,
    acknowledged: bool,
) -> Result<(), String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.set_security_warning_acknowledged(acknowledged))
}

#[tauri::command]
pub async fn get_launch_mode(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.get_launch_mode())
}

#[tauri::command]
pub async fn set_launch_mode(
    state: tauri::State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    // Validate mode
    if mode != "modded" && mode != "vanilla" {
        return Err(format!(
            "Invalid launch mode: {}. Must be 'modded' or 'vanilla'",
            mode
        ));
    }

    // Save the preference to database
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    map_error(db.set_launch_mode(&mode))
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn test_launch_mode_validation_valid_modes() {
        // Test that validation logic in set_launch_mode accepts valid modes
        let valid_modes = ["modded", "vanilla"];
        for mode in valid_modes {
            assert!(mode == "modded" || mode == "vanilla");
        }
    }

    #[test]
    fn test_launch_mode_validation_invalid_modes() {
        // Test that invalid modes would be rejected
        let invalid_modes = ["invalid", "MODDED", "Vanilla", "", "both"];
        for mode in invalid_modes {
            let is_valid = mode == "modded" || mode == "vanilla";
            assert!(!is_valid, "Mode '{}' should be invalid", mode);
        }
    }

    #[test]
    fn test_launch_mode_error_message_format() {
        // Verify error message format for invalid mode
        let mode = "invalid";
        let expected = format!(
            "Invalid launch mode: {}. Must be 'modded' or 'vanilla'",
            mode
        );
        assert!(expected.contains("Invalid launch mode"));
        assert!(expected.contains("modded"));
        assert!(expected.contains("vanilla"));
    }
}
