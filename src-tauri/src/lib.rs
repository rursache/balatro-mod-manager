mod assets;
mod catalog;
pub mod commands;
mod compat_helper;
mod install_plan;
mod links;
#[cfg(test)]
mod live_install_tests;
mod mod_index;
mod models;
mod multiplayer;
mod state;
mod thumb_queue;
mod thunderstore;
mod util;
#[cfg(target_os = "linux")]
pub mod wayland_session;

use std::path::PathBuf;

use tauri::{Emitter, Manager};
use tauri_plugin_window_state::StateFlags;

use bmm_lib::{
    database::Database, discord_rpc::DiscordRpcManager, errors::AppError, local_mod_detection,
};

use crate::models::{ModsChangedEvent, Payload};
use crate::state::AppState;
use crate::util::map_error;

#[tauri::command]
fn exit_application(app_handle: tauri::AppHandle) {
    app_handle.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            app.emit("single-instance", Payload { args: argv, cwd })
                .unwrap();
        }))
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_prevent_default::init())
        .setup(|app| {
            let db = map_error(Database::new())?;
            crate::thunderstore::set_enabled(db.is_thunderstore_enabled().unwrap_or(true));

            // Initialize Discord RPC manager (lightweight, no network calls)
            let discord_rpc = DiscordRpcManager::new();

            app.manage(AppState {
                db: std::sync::Mutex::new(db),
                discord_rpc: tokio::sync::Mutex::new(discord_rpc),
                thumbs: crate::thumb_queue::ThumbnailManager::new(),
            });

            // Spawn deferred startup tasks - these run after window is visible
            // to improve perceived startup performance
            let app_handle = app.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                use std::time::Duration;
                use tokio::time::sleep;

                // Small delay to let the window render first
                sleep(Duration::from_millis(100)).await;

                // Open a separate DB connection for background tasks
                let db = match Database::new() {
                    Ok(db) => db,
                    Err(e) => {
                        log::warn!("Deferred startup: failed to open DB: {e}");
                        return;
                    }
                };

                // One-time migration for 0.3.7: disable compat helper for all users
                if !db.is_compat_helper_037_migrated().unwrap_or(true) {
                    log::info!("Applying 0.3.7 migration: disabling compatibility helper");
                    if let Err(e) = db.set_compat_helper_enabled(false) {
                        log::warn!("Failed to disable compat helper during migration: {}", e);
                    }
                    if let Err(e) = db.set_compat_helper_037_migrated() {
                        log::warn!("Failed to mark 0.3.7 migration as complete: {}", e);
                    }
                }

                // One-time migration for Lovely 0.9.0 (Linux only)
                #[cfg(target_os = "linux")]
                {
                    match local_mod_detection::migrate_legacy_mods_dir() {
                        Ok(true) => {
                            log::info!("Successfully migrated mods to new Lovely 0.9.0 location")
                        }
                        Ok(false) => {}
                        Err(e) => log::warn!("Failed to migrate legacy mods directory: {}", e),
                    }
                }

                // Enable Discord RPC if configured
                let discord_rpc_enabled = db.is_discord_rpc_enabled().unwrap_or(true);
                if let Some(state) = app_handle.try_state::<AppState>()
                    && let Ok(rpc) = state.discord_rpc.try_lock()
                {
                    rpc.set_enabled(discord_rpc_enabled);
                }

                // Remove legacy GitHub-based local clone directory if it exists
                if let Some(cfg_dir) = dirs::config_dir() {
                    let legacy_repo = cfg_dir.join("Balatro").join("mod_index");
                    if legacy_repo.exists() {
                        match std::fs::remove_dir_all(&legacy_repo) {
                            Ok(()) => log::info!(
                                "Removed legacy GitHub repo directory: {}",
                                legacy_repo.display()
                            ),
                            Err(e) => log::warn!(
                                "Failed to remove legacy repo directory {}: {}",
                                legacy_repo.display(),
                                e
                            ),
                        }
                    }
                }
                crate::mod_index::migrate_legacy_caches();

                // Ensure the Mods directory exists
                if let Some(cfg_dir) = dirs::config_dir() {
                    let mods_dir = cfg_dir.join("Balatro").join("Mods");
                    if let Err(e) = std::fs::create_dir_all(&mods_dir) {
                        log::warn!(
                            "Failed to create mods directory at {}: {}",
                            mods_dir.display(),
                            e
                        );
                    }

                    // Clean up empty directories in Mods folder on startup
                    if let Ok(entries) = std::fs::read_dir(&mods_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_dir() {
                                // Skip known system directories
                                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                    let lower = name.to_lowercase();
                                    if lower.contains("lovely")
                                        || lower.starts_with('.')
                                        || name == "BMM-Compat"
                                    {
                                        continue;
                                    }
                                }
                                // Remove if empty (no files, only maybe empty subdirs)
                                if is_dir_empty_recursive(&path)
                                    && std::fs::remove_dir_all(&path).is_ok()
                                {
                                    log::info!(
                                        "Cleaned up empty directory on startup: {}",
                                        path.display()
                                    );
                                }
                            }
                        }
                    }
                }

                // Sync compat helper
                let compat_enabled = db.is_compat_helper_enabled().unwrap_or(false);
                if let Err(e) = crate::compat_helper::sync_compat_helper(compat_enabled) {
                    log::warn!("Failed to sync compatibility helper: {}", e);
                }

                // Check Lovely version
                match db.get_lovely_version() {
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => {
                        log::info!(
                            "lovely_version missing; UI will prompt to install/update Lovely"
                        );
                    }
                }

                log::debug!("Deferred startup tasks completed");
            });

            // Periodically validate the mod database in a very cheap, incremental sweep.
            // Uses adaptive sleep intervals - faster when mods are present, slower when idle.
            // Clone a handle that is 'static so we can emit events from the background task.
            let handle_for_events = app.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                use std::time::Duration;
                use tokio::time::sleep;

                const REINDEX_BATCH_SIZE: usize = 5; // small batch to keep cost negligible
                const ACTIVE_TICK_SECS: u64 = 2; // 2s when mods are installed
                const IDLE_TICK_SECS: u64 = 30; // 30s when no mods installed (saves CPU/battery)
                const STABLE_TICK_SECS: u64 = 10; // 10s when fingerprint hasn't changed recently
                const INITIAL_DELAY_SECS: u64 = 5; // Wait before first scan to let app finish loading

                // Initial delay to let the app finish startup before consuming CPU
                sleep(Duration::from_secs(INITIAL_DELAY_SECS)).await;

                // Snapshot of installed mods to sweep over between refreshes
                let mut snapshot: Vec<(String, String)> = Vec::new(); // (name, path)
                let mut cursor_idx: usize = 0;
                let mut consecutive_stable_checks: u32 = 0; // Track how long fingerprint has been stable

                // Open a dedicated DB connection for this background task.
                // Using a separate connection avoids borrowing app state and remains lightweight.
                let db = match Database::new() {
                    Ok(db) => db,
                    Err(e) => {
                        log::warn!("Auto reindex: failed to open DB: {}", e);
                        return;
                    }
                };

                // Lightweight fingerprint of Mods directory to detect additions/removals
                fn mods_dir_fingerprint() -> Option<u64> {
                    let config_dir = dirs::config_dir()?;
                    let mods_dir = config_dir.join("Balatro").join("Mods");
                    if !mods_dir.exists() {
                        return Some(0);
                    }
                    let mut sum: u64 = 1469598103934665603; // FNV offset basis
                    let rd = std::fs::read_dir(&mods_dir).ok()?;
                    for entry in rd.flatten() {
                        let path = entry.path();
                        if !path.is_dir() {
                            continue;
                        }
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            let lower = name.to_lowercase();
                            if lower.contains("lovely")
                                || lower.starts_with('.')
                                || matches!(lower.as_str(), ".git" | "node_modules" | "__macosx")
                            {
                                continue;
                            }
                            for b in lower.as_bytes() {
                                sum = sum.wrapping_mul(1099511628211).wrapping_add(*b as u64);
                            }
                            if let Ok(meta) = path.metadata()
                                && let Ok(mtime) = meta.modified()
                                && let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH)
                            {
                                sum = sum.wrapping_mul(1099511628211).wrapping_add(dur.as_secs());
                            }
                        }
                    }
                    Some(sum)
                }
                let mut last_fp = mods_dir_fingerprint();

                loop {
                    // Adaptive sleep: longer when idle/stable, shorter when active
                    let tick_secs = if snapshot.is_empty() {
                        IDLE_TICK_SECS
                    } else if consecutive_stable_checks > 5 {
                        STABLE_TICK_SECS
                    } else {
                        ACTIVE_TICK_SECS
                    };
                    sleep(Duration::from_secs(tick_secs)).await;

                    // Refresh snapshot when exhausted or empty
                    if cursor_idx >= snapshot.len() {
                        let mods: Vec<(String, String)> = match db
                            .get_installed_mods()
                            .map(|v| v.into_iter().map(|m| (m.name, m.path)).collect())
                        {
                            Ok(v) => v,
                            Err(e) => {
                                log::warn!("Auto reindex: failed to load mods: {}", e);
                                continue;
                            }
                        };
                        snapshot = mods;
                        cursor_idx = 0;
                    }

                    if snapshot.is_empty() {
                        // No mods installed - just check fingerprint occasionally
                        let cur_fp = mods_dir_fingerprint();
                        if cur_fp.is_some() && cur_fp != last_fp {
                            last_fp = cur_fp;
                            local_mod_detection::clear_detection_cache();
                            let _ = handle_for_events.emit(
                                "installed-mods-changed",
                                ModsChangedEvent {
                                    added: Vec::new(),
                                    removed: Vec::new(),
                                    full_refresh: true,
                                },
                            );
                            consecutive_stable_checks = 0;
                        } else {
                            consecutive_stable_checks = consecutive_stable_checks.saturating_add(1);
                        }
                        continue;
                    }

                    let end = (cursor_idx + REINDEX_BATCH_SIZE).min(snapshot.len());
                    let mut removed_mods: Vec<String> = Vec::new();
                    for (name, path) in &snapshot[cursor_idx..end] {
                        if !std::path::Path::new(path).exists() {
                            // Remove missing entry from DB
                            match db.remove_installed_mod(name) {
                                Ok(()) => {
                                    removed_mods.push(name.clone());
                                }
                                Err(e) => {
                                    log::warn!("Auto reindex: failed to remove '{}': {}", name, e)
                                }
                            }
                        }
                    }
                    cursor_idx = end;

                    // Detect Mods dir fingerprint changes (additions/removals/renames)
                    let cur_fp = mods_dir_fingerprint();
                    let fp_changed = cur_fp.is_some() && cur_fp != last_fp;
                    if fp_changed {
                        last_fp = cur_fp;
                        consecutive_stable_checks = 0;
                        // Clear cache so next detection reflects changes and notify UI
                        local_mod_detection::clear_detection_cache();

                        // Clean up empty directories in Mods folder
                        if let Some(config_dir) = dirs::config_dir() {
                            let mods_dir = config_dir.join("Balatro").join("Mods");
                            if let Ok(entries) = std::fs::read_dir(&mods_dir) {
                                for entry in entries.flatten() {
                                    let path = entry.path();
                                    if path.is_dir() {
                                        // Skip known system directories
                                        if let Some(name) =
                                            path.file_name().and_then(|n| n.to_str())
                                        {
                                            let lower = name.to_lowercase();
                                            if lower.contains("lovely")
                                                || lower.starts_with('.')
                                                || name == "BMM-Compat"
                                            {
                                                continue;
                                            }
                                        }
                                        // Remove if empty (no files, only maybe empty subdirs)
                                        if is_dir_empty_recursive(&path)
                                            && std::fs::remove_dir_all(&path).is_ok()
                                        {
                                            log::info!(
                                                "Cleaned up empty directory: {}",
                                                path.display()
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        // Emit with full_refresh since we don't know exactly what changed
                        let _ = handle_for_events.emit(
                            "installed-mods-changed",
                            ModsChangedEvent {
                                added: Vec::new(),
                                removed: Vec::new(),
                                full_refresh: true,
                            },
                        );
                    } else {
                        consecutive_stable_checks = consecutive_stable_checks.saturating_add(1);
                    }

                    if !removed_mods.is_empty() {
                        // Clear detection cache so next detection reflects changes
                        local_mod_detection::clear_detection_cache();
                        log::info!(
                            "Auto reindex: cleaned {} database entr{} (batch)",
                            removed_mods.len(),
                            if removed_mods.len() == 1 { "y" } else { "ies" }
                        );

                        // Notify UI with delta information
                        let _ = handle_for_events.emit(
                            "installed-mods-changed",
                            ModsChangedEvent {
                                added: Vec::new(),
                                removed: removed_mods,
                                full_refresh: false,
                            },
                        );
                    }
                }
            });

            let app_dir = app
                .path()
                .app_data_dir()
                .map_err(|_| AppError::DirNotFound(PathBuf::from("app data directory")))?;
            std::fs::create_dir_all(&app_dir).map_err(|e| AppError::DirCreate {
                path: app_dir.clone(),
                source: e.to_string(),
            })?;
            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
            }

            // Ensure the main window is visible/focused on startup even if a saved state
            // or window manager starts it hidden/minimized.
            match app.get_webview_window("main") {
                Some(window) => {
                    if let Err(e) = window.show() {
                        log::warn!("Failed to show main window: {e}");
                    }
                    if let Err(e) = window.unminimize() {
                        log::warn!("Failed to unminimize main window: {e}");
                    }
                    if let Err(e) = window.set_focus() {
                        log::warn!("Failed to focus main window: {e}");
                    }
                    #[cfg(target_os = "linux")]
                    clamp_window_to_monitor(&window);
                }
                None => log::warn!("Main window not found during setup; UI may remain hidden"),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::paths::find_steam_balatro,
            commands::paths::check_custom_balatro,
            commands::paths::check_existing_installation,
            commands::paths::get_balatro_path,
            commands::paths::set_balatro_path,
            commands::paths::get_mods_folder,
            commands::paths::open_directory,
            commands::install::launch_balatro,
            commands::system::check_steam_running,
            commands::system::check_balatro_running,
            commands::system::get_app_version,
            commands::install::get_installed_mods_from_db,
            commands::install::install_mod,
            commands::install::plan_catalog_install,
            commands::install::install_catalog_mod,
            commands::multiplayer::multiplayer_status,
            commands::multiplayer::multiplayer_plan,
            commands::multiplayer::multiplayer_install,
            commands::multiplayer::multiplayer_uninstall,
            commands::multiplayer::multiplayer_migrate_legacy,
            commands::install::add_installed_mod,
            commands::install::remove_installed_mod,
            commands::install::get_steamodded_versions,
            commands::install::install_steamodded_version,
            commands::install::install_talisman_version,
            commands::install::get_talisman_versions,
            commands::install::get_latest_steamodded_release,
            commands::paths::verify_path_exists,
            commands::paths::path_exists,
            commands::detection::check_mod_installation,
            commands::detection::refresh_mods_folder,
            commands::cache::save_mods_cache,
            commands::cache::load_mods_cache,
            commands::cache::clear_cache,
            commands::cache::save_versions_cache,
            commands::cache::load_versions_cache,
            commands::settings::get_lovely_console_status,
            commands::settings::set_lovely_console_status,
            commands::lovely::check_lovely_update,
            commands::lovely::update_lovely_to_latest,
            commands::lovely::is_lovely_installed,
            commands::settings::get_background_state,
            commands::settings::set_background_state,
            commands::settings::get_compat_helper_status,
            commands::settings::set_compat_helper_status,
            commands::settings::get_thunderstore_enabled,
            commands::settings::set_thunderstore_enabled,
            commands::settings::get_discord_rpc_status,
            commands::settings::set_discord_rpc_status,
            commands::settings::set_linux_prefix,
            commands::settings::get_linux_prefix,
            commands::settings::set_security_warning_acknowledged,
            commands::settings::is_security_warning_acknowledged,
            commands::settings::get_launch_mode,
            commands::settings::set_launch_mode,
            commands::cache::get_last_fetched,
            commands::cache::update_last_fetched,
            commands::repo::fetch_repo_mods,
            commands::repo::catalog_status,
            commands::repo::get_cached_thumbnail_by_title,
            commands::repo::get_cached_thumbnails_map,
            commands::repo::get_description_cached_or_remote,
            commands::repo::get_mod_requirements,
            commands::repo::get_mod_details,
            commands::repo::get_mod_repo_url,
            commands::repo::get_cached_description_by_title,
            commands::thumbnails::fetch_thumbnail,
            commands::thumbnails::cancel_thumbnail,
            commands::report::build_issue_report_url,
            commands::report::get_latest_log,
            commands::report::get_logs_folder,
            commands::mods::is_mod_enabled,
            commands::mods::toggle_mod_enabled,
            commands::mods::toggle_mods_enabled_batch,
            commands::mods::is_mod_enabled_by_path,
            commands::mods::toggle_mod_enabled_by_path,
            commands::mods::enabled_state_map,
            commands::cache::mod_update_available,
            commands::cache::mods_updates_map,
            commands::cache::mods_state_summary,
            commands::install::cascade_uninstall,
            commands::install::force_remove_mod,
            commands::install::get_dependents,
            commands::import::process_dropped_file,
            commands::import::process_mod_archive,
            commands::detection::get_detected_local_mods,
            commands::detection::reindex_mods,
            commands::detection::reconcile_orphan_mods,
            commands::detection::delete_manual_mod,
            commands::detection::backup_local_mod,
            commands::detection::restore_from_backup,
            commands::detection::remove_backup,
            commands::external::open_external_url,
            commands::init::get_app_init_data,
            commands::init::get_all_settings,
            commands::backup::create_backup,
            commands::backup::list_backups,
            commands::backup::restore_backup,
            commands::backup::delete_backup,
            commands::backup::get_backups_total_size,
            commands::backup::get_backups_directory,
            commands::backup::check_interrupted_restore,
            commands::backup::clear_interrupted_restore,
            commands::health::check_health,
            commands::health::reset_database,
            commands::health::clear_app_state,
            exit_application
        ])
        .build(tauri::generate_context!());

    match result {
        Ok(app) => {
            app.run(|app_handle, event| {
                if let tauri::RunEvent::Exit = event {
                    // Mark Wayland session as clean (Linux only)
                    #[cfg(target_os = "linux")]
                    crate::wayland_session::mark_clean();

                    // Checkpoint WAL on shutdown for a clean database state
                    if let Some(state) = app_handle.try_state::<AppState>()
                        && let Ok(db) = state.db.try_lock()
                    {
                        if let Err(e) = db.checkpoint() {
                            log::warn!("Failed to checkpoint database on exit: {}", e);
                        } else {
                            log::debug!("Database checkpointed on exit");
                        }
                    }
                }
            });
        }
        Err(e) => {
            log::error!("Failed to build application: {e}");
            log::logger().flush();
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "linux")]
fn clamp_window_to_monitor(window: &tauri::WebviewWindow) {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let monitor_size = monitor.size();
    let padding = 48u32;
    let max_w = monitor_size.width.saturating_sub(padding);
    let max_h = monitor_size.height.saturating_sub(padding);
    let Ok(window_size) = window.outer_size() else {
        return;
    };
    if window_size.width <= max_w && window_size.height <= max_h {
        return;
    }
    let new_size = tauri::PhysicalSize::new(max_w, max_h);
    if let Err(e) = window.set_size(tauri::Size::Physical(new_size)) {
        log::warn!("Failed to clamp window size: {e}");
        return;
    }
    let monitor_pos = monitor.position();
    let centered_x = monitor_pos.x + ((monitor_size.width - max_w) / 2) as i32;
    let centered_y = monitor_pos.y + ((monitor_size.height - max_h) / 2) as i32;
    let new_pos = tauri::PhysicalPosition::new(centered_x, centered_y);
    if let Err(e) = window.set_position(tauri::Position::Physical(new_pos)) {
        log::warn!("Failed to clamp window position: {e}");
    }
}

/// Check if a directory is empty (contains no files, only possibly empty subdirectories).
fn is_dir_empty_recursive(path: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_file() {
            return false;
        }
        if entry_path.is_dir() && !is_dir_empty_recursive(&entry_path) {
            return false;
        }
    }
    true
}
