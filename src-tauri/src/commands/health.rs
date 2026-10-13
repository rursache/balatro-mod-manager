//! Health checks and state recovery commands.
//!
//! This module provides commands to verify the integrity of the app's
//! persistent state (database, cache, config) and recover from corruption.

use crate::state::AppState;
use serde::Serialize;
use std::path::PathBuf;

/// Result of a health check
#[derive(Debug, Clone, Serialize)]
pub struct HealthCheckResult {
    /// Overall health status
    pub healthy: bool,
    /// Individual check results
    pub checks: Vec<HealthCheck>,
    /// Recovery actions available if unhealthy
    pub recovery_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthCheck {
    pub name: String,
    pub status: HealthStatus,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Ok,
    Warning,
    Error,
}

/// Perform health checks on all app state
#[tauri::command]
pub async fn check_health(state: tauri::State<'_, AppState>) -> Result<HealthCheckResult, String> {
    let mut checks = Vec::new();
    let mut recovery_actions = Vec::new();

    // Check 1: Database connectivity
    let db_check = check_database(&state);
    if matches!(db_check.status, HealthStatus::Error) {
        recovery_actions.push("reset_database".to_string());
    }
    checks.push(db_check);

    // Check 2: Game path validity
    let path_check = check_game_path(&state);
    if matches!(path_check.status, HealthStatus::Error) {
        recovery_actions.push("reconfigure_path".to_string());
    }
    checks.push(path_check);

    // Check 3: Cache integrity
    let cache_check = check_cache().await;
    if matches!(
        cache_check.status,
        HealthStatus::Error | HealthStatus::Warning
    ) {
        recovery_actions.push("clear_cache".to_string());
    }
    checks.push(cache_check);

    // Check 4: Mods directory
    let mods_check = check_mods_directory(&state);
    if matches!(mods_check.status, HealthStatus::Warning) {
        recovery_actions.push("reindex_mods".to_string());
    }
    checks.push(mods_check);

    let healthy = checks.iter().all(|c| matches!(c.status, HealthStatus::Ok));

    Ok(HealthCheckResult {
        healthy,
        checks,
        recovery_actions,
    })
}

fn check_database(state: &tauri::State<'_, AppState>) -> HealthCheck {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());

    // Try a simple query to verify database is working
    match db.get_installed_mods() {
        Ok(_) => HealthCheck {
            name: "database".to_string(),
            status: HealthStatus::Ok,
            message: "Database is accessible".to_string(),
        },
        Err(e) => HealthCheck {
            name: "database".to_string(),
            status: HealthStatus::Error,
            message: format!("Database error: {}", e),
        },
    }
}

fn check_game_path(state: &tauri::State<'_, AppState>) -> HealthCheck {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());

    match db.get_installation_path() {
        Ok(Some(path)) => {
            let path = PathBuf::from(&path);
            if path.exists() {
                // Check for Balatro executable
                let has_exe = path.join("Balatro.exe").exists()
                    || path.join("Balatro.app").exists()
                    || path.join("Balatro.x86_64").exists();

                if has_exe {
                    HealthCheck {
                        name: "game_path".to_string(),
                        status: HealthStatus::Ok,
                        message: format!("Game found at {}", path.display()),
                    }
                } else {
                    HealthCheck {
                        name: "game_path".to_string(),
                        status: HealthStatus::Warning,
                        message: "Game path exists but Balatro executable not found".to_string(),
                    }
                }
            } else {
                HealthCheck {
                    name: "game_path".to_string(),
                    status: HealthStatus::Error,
                    message: format!("Game path no longer exists: {}", path.display()),
                }
            }
        }
        Ok(None) => HealthCheck {
            name: "game_path".to_string(),
            status: HealthStatus::Warning,
            message: "No game path configured".to_string(),
        },
        Err(e) => HealthCheck {
            name: "game_path".to_string(),
            status: HealthStatus::Error,
            message: format!("Failed to read game path: {}", e),
        },
    }
}

async fn check_cache() -> HealthCheck {
    let config_dir = match dirs::config_dir() {
        Some(dir) => dir,
        None => {
            return HealthCheck {
                name: "cache".to_string(),
                status: HealthStatus::Warning,
                message: "Could not locate config directory".to_string(),
            };
        }
    };

    let cache_dir = config_dir.join("Balatro").join("mod_index_cache");
    let assets_dir = config_dir.join("Balatro").join("mod_assets");

    let cache_exists = cache_dir.exists();
    let assets_exists = assets_dir.exists();

    // Try to read a cache file to verify it's not corrupted
    if cache_exists {
        let cache_files: Vec<_> = std::fs::read_dir(&cache_dir)
            .map(|entries| entries.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();

        for entry in cache_files {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        if serde_json::from_str::<serde_json::Value>(&content).is_err() {
                            return HealthCheck {
                                name: "cache".to_string(),
                                status: HealthStatus::Warning,
                                message: format!(
                                    "Cache file corrupted: {}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ),
                            };
                        }
                    }
                    Err(_) => {
                        return HealthCheck {
                            name: "cache".to_string(),
                            status: HealthStatus::Warning,
                            message: "Cache file unreadable".to_string(),
                        };
                    }
                }
            }
        }
    }

    if cache_exists || assets_exists {
        HealthCheck {
            name: "cache".to_string(),
            status: HealthStatus::Ok,
            message: "Cache is valid".to_string(),
        }
    } else {
        HealthCheck {
            name: "cache".to_string(),
            status: HealthStatus::Ok,
            message: "No cache present (will be created on first use)".to_string(),
        }
    }
}

fn check_mods_directory(state: &tauri::State<'_, AppState>) -> HealthCheck {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());

    match db.get_installation_path() {
        Ok(Some(path)) => {
            let mods_dir = PathBuf::from(&path).join("Mods");
            if mods_dir.exists() {
                // Count mods in directory vs database
                let fs_count = std::fs::read_dir(&mods_dir)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| e.path().is_dir())
                            .count()
                    })
                    .unwrap_or(0);

                let db_count = db.get_installed_mods().map(|m| m.len()).unwrap_or(0);

                if fs_count == db_count {
                    HealthCheck {
                        name: "mods_directory".to_string(),
                        status: HealthStatus::Ok,
                        message: format!("{} mods tracked", db_count),
                    }
                } else {
                    HealthCheck {
                        name: "mods_directory".to_string(),
                        status: HealthStatus::Warning,
                        message: format!(
                            "Mods directory has {} folders but {} tracked in database",
                            fs_count, db_count
                        ),
                    }
                }
            } else {
                HealthCheck {
                    name: "mods_directory".to_string(),
                    status: HealthStatus::Ok,
                    message: "Mods directory does not exist yet".to_string(),
                }
            }
        }
        _ => HealthCheck {
            name: "mods_directory".to_string(),
            status: HealthStatus::Warning,
            message: "Cannot check mods directory without game path".to_string(),
        },
    }
}

/// Reset the database to a fresh state
#[tauri::command]
pub async fn reset_database(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());

    // Remove all installed mods one by one
    let mods = db.get_installed_mods().map_err(|e| e.to_string())?;
    for m in mods {
        db.remove_installed_mod(&m.name)
            .map_err(|e| e.to_string())?;
    }

    log::info!("Database reset completed");
    Ok(())
}

/// Wipe transient on-disk caches that can drift out of sync and cause the
/// frontend to crash on hydration (mod index cache, thumbnails, details
/// cache). Installed mods, settings, and Lovely state stay intact because
/// they live in the SQLite database, not in these caches.
#[tauri::command]
pub async fn clear_app_state() -> Result<(), String> {
    crate::catalog::reset_cache().await;
    let config_dir = dirs::config_dir().ok_or_else(|| "config directory not found".to_string())?;
    let balatro = config_dir.join("Balatro");

    let targets = [
        balatro.join("mod_index_cache"),
        balatro.join("mod_assets"),
        balatro.join("mod_details"),
    ];

    let mut removed = 0usize;
    for path in targets.iter() {
        if path.exists() {
            match std::fs::remove_dir_all(path) {
                Ok(()) => {
                    removed += 1;
                    log::info!("clear_app_state: removed {}", path.display());
                }
                Err(e) => {
                    log::warn!(
                        "clear_app_state: failed to remove {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
    }
    log::info!("clear_app_state: cleared {} cache directory(ies)", removed);
    Ok(())
}
