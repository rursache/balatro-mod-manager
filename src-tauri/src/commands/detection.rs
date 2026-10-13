use std::path::{Path, PathBuf};

use crate::models::ModsChangedEvent;
use crate::state::AppState;
use crate::util::map_error;
use bmm_lib::{cache, database::Database, errors::AppError, local_mod_detection};
use serde_json::json;
use tauri::Emitter;

#[tauri::command]
pub async fn check_mod_installation(mod_type: String) -> Result<bool, String> {
    let db = map_error(bmm_lib::database::Database::new())?;
    let installed_mods = map_error(db.get_installed_mods())?;

    let cached_mods = match cache::load_cache() {
        Ok(Some((mods, _))) => mods,
        _ => Vec::new(),
    };
    let detected_mods = local_mod_detection::detect_manual_mods_cached(&db, &cached_mods)?;

    let mod_name = mod_type.as_str();
    match mod_name {
        "Steamodded" | "Talisman" => Ok(installed_mods.iter().any(|m| m.name == mod_name)
            || detected_mods.iter().any(|m| m.name == mod_name)),
        _ => Err(AppError::InvalidState("Invalid mod type".to_string()).to_string()),
    }
}

#[tauri::command]
pub async fn refresh_mods_folder(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let installed = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        map_error(db.get_installed_mods())?
    }; // guard dropped here

    for m in installed {
        let mod_dir = PathBuf::from(&m.path);
        if tokio::fs::metadata(&mod_dir).await.is_ok() {
            let mut entries = tokio::fs::read_dir(&mod_dir)
                .await
                .map_err(|e| format!("Failed to read mod directory: {e}"))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| format!("Failed to read entry: {e}"))?
            {
                let path = entry.path();
                if tokio::fs::metadata(&path)
                    .await
                    .map(|m| m.is_dir())
                    .unwrap_or(false)
                {
                    let ignore_file_path = path.join(".lovelyignore");
                    if tokio::fs::metadata(&ignore_file_path).await.is_ok() {
                        tokio::fs::remove_file(&ignore_file_path)
                            .await
                            .map_err(|e| AppError::FileWrite {
                                path: path.clone(),
                                source: e.to_string(),
                            })?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_detected_local_mods(
    _state: tauri::State<'_, AppState>,
) -> Result<Vec<local_mod_detection::DetectedMod>, String> {
    let db = Database::new().map_err(|e| e.to_string())?;
    let cached_mods = match cache::load_cache() {
        Ok(Some((mods, _))) => mods,
        _ => Vec::new(),
    };
    local_mod_detection::detect_manual_mods_cached(&db, &cached_mods)
}

/// Reindexes mods by syncing the database with the filesystem.
/// Returns (files_removed, db_entries_cleaned). Currently we only clean DB entries.
#[tauri::command]
pub async fn reindex_mods(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(usize, usize), String> {
    let db = state.db.lock().unwrap_or_else(|e| e.into_inner());

    let result = reindex_db(&db);
    match &result {
        Ok((_files, cleaned)) if *cleaned > 0 => {
            // Best-effort event notify; use full_refresh since we don't track individual removals
            let _ = app_handle.emit(
                "installed-mods-changed",
                ModsChangedEvent {
                    added: Vec::new(),
                    removed: Vec::new(),
                    full_refresh: true,
                },
            );
        }
        _ => {}
    }
    map_error(result)
}

/// Result of reconciling installed mods against the remote index.
#[derive(serde::Serialize)]
pub struct OrphanReconcileResult {
    pub skipped: bool,
    pub changed: Vec<String>,
    pub orphan_total: usize,
}

/// Reconcile the orphaned flag for installed mods against the latest remote
/// index. Marks installed mods missing from `remote_titles` as orphaned, and
/// clears the flag for mods that have reappeared.
///
/// A safety threshold mirrors the frontend prune guard: a tiny incoming set
/// (likely an API hiccup) must not cause every installed mod to be flagged.
#[tauri::command]
pub async fn reconcile_orphan_mods(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    remote_titles: Vec<String>,
) -> Result<OrphanReconcileResult, String> {
    let mut db = state.db.lock().unwrap_or_else(|e| e.into_inner());
    let installed = map_error(db.get_installed_mods())?;
    let non_orphan_count = installed.iter().filter(|m| !m.orphaned).count();

    // Safety: skip when the remote returned nothing, or returned far fewer
    // entries than we have tracked. Mirrors frontend mergeIncomingMods guard.
    let threshold = std::cmp::max(10usize, non_orphan_count / 2);
    if remote_titles.is_empty() || remote_titles.len() < threshold {
        log::info!(
            "reconcile_orphan_mods: skipped (remote={}, tracked_non_orphan={}, threshold={})",
            remote_titles.len(),
            non_orphan_count,
            threshold
        );
        return Ok(OrphanReconcileResult {
            skipped: true,
            changed: Vec::new(),
            orphan_total: installed.iter().filter(|m| m.orphaned).count(),
        });
    }

    let present: std::collections::HashSet<String> = remote_titles.into_iter().collect();
    let changed = map_error(db.reconcile_orphaned_mods(&present))?;
    let orphan_total = map_error(db.get_installed_mods())?
        .iter()
        .filter(|m| m.orphaned)
        .count();

    if !changed.is_empty() {
        log::info!(
            "reconcile_orphan_mods: {} mod(s) changed orphan state, {} total orphaned",
            changed.len(),
            orphan_total
        );
        let _ = app_handle.emit(
            "installed-mods-changed",
            ModsChangedEvent {
                added: Vec::new(),
                removed: Vec::new(),
                full_refresh: true,
            },
        );
    }

    Ok(OrphanReconcileResult {
        skipped: false,
        changed,
        orphan_total,
    })
}

/// Internal helper to perform the actual reindexing logic.
/// Returns (files_removed, db_entries_cleaned). Currently we only clean DB entries.
pub fn reindex_db(db: &Database) -> Result<(usize, usize), AppError> {
    let installed = db.get_installed_mods()?;
    let mut cleaned_entries = 0usize;
    for m in installed {
        let path = PathBuf::from(&m.path);
        if !path.exists() {
            db.remove_installed_mod(&m.name)?;
            cleaned_entries += 1;
        }
    }

    // Clear detection cache so next detection reflects changes
    local_mod_detection::clear_detection_cache();

    Ok((0, cleaned_entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use tempfile::tempdir;

    fn set_var(key: &str, val: impl AsRef<OsStr>) {
        unsafe { std::env::set_var(key, val) };
    }

    fn remove_var(key: &str) {
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn ensure_strictly_inside_rejects_mods_dir_and_escapes() {
        let td = tempdir().unwrap();
        let mods_dir = td.path().join("Mods");
        let mod_dir = mods_dir.join("SomeMod");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::create_dir_all(td.path().join("outside")).unwrap();

        assert!(ensure_strictly_inside(&mod_dir, &mods_dir).is_ok());
        for path in [
            mods_dir.clone(),
            mods_dir.join("."),
            mod_dir.join(".."),
            mods_dir.join("..").join("outside"),
            td.path().to_path_buf(),
        ] {
            assert!(
                ensure_strictly_inside(&path, &mods_dir).is_err(),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn reindex_db_removes_missing_paths() {
        // Redirect config dir to temp
        let td = tempdir().unwrap();
        let original_cfg = std::env::var_os("XDG_CONFIG_HOME");
        let original_home = std::env::var_os("HOME");
        set_var("XDG_CONFIG_HOME", td.path());
        if cfg!(target_os = "macos") {
            set_var("HOME", td.path());
        }

        let db = Database::new().expect("db");

        // Create one existing and one missing mod path
        let mods_dir = dirs::config_dir().unwrap().join("Balatro").join("Mods");
        std::fs::create_dir_all(&mods_dir).unwrap();
        let existing = mods_dir.join("Exists");
        std::fs::create_dir_all(&existing).unwrap();
        let missing = mods_dir.join("Missing"); // do not create

        db.add_installed_mod("Existing", existing.to_string_lossy().as_ref(), &[], None)
            .unwrap();
        db.add_installed_mod("Missing", missing.to_string_lossy().as_ref(), &[], None)
            .unwrap();

        let (_files, cleaned) = reindex_db(&db).expect("reindex");
        assert_eq!(cleaned, 1);

        let remaining = db.get_installed_mods().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "Existing");

        // restore env
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

#[tauri::command]
pub async fn delete_manual_mod(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let metadata = tokio::fs::metadata(&path).await;
    if metadata.is_err() {
        return Err(format!(
            "Invalid path '{}': Path doesn't exist",
            path.display()
        ));
    }
    let config_dir =
        dirs::config_dir().ok_or_else(|| "Could not find config directory".to_string())?;
    let mods_dir = config_dir.join("Balatro").join("Mods");

    ensure_strictly_inside(&path, &mods_dir)?;

    if metadata.unwrap().is_dir() {
        tokio::fs::remove_dir_all(&path)
            .await
            .map_err(|e| format!("Failed to remove directory: {e}"))?
    } else {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| format!("Failed to remove file: {e}"))?
    }
    Ok(())
}

fn ensure_strictly_inside(path: &Path, mods_dir: &Path) -> Result<(), String> {
    let canonicalized_path = std::fs::canonicalize(path)
        .map_err(|e| format!("Failed to canonicalize path {}: {}", path.display(), e))?;
    let canonicalized_mods_dir = std::fs::canonicalize(mods_dir)
        .map_err(|e| format!("Failed to canonicalize mods directory: {e}"))?;
    if canonicalized_path == canonicalized_mods_dir
        || !canonicalized_path.starts_with(&canonicalized_mods_dir)
    {
        return Err(format!(
            "Path is outside of the mods directory: {}",
            path.display()
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn backup_local_mod(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let metadata = tokio::fs::metadata(&path).await;
    if metadata.is_err() {
        return Err(format!("Path doesn't exist: {}", path.display()));
    }

    let backup_dir = get_backup_dir_async().await?;
    let backup_id = format!(
        "backup_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| format!("Failed to get timestamp: {e}"))?
            .as_millis()
    );
    let backup_path = backup_dir.join(backup_id);

    tokio::fs::create_dir_all(&backup_path)
        .await
        .map_err(|e| format!("Failed to create backup directory: {e}"))?;

    if metadata.unwrap().is_dir() {
        copy_dir_all_async(&path, &backup_path.join(path.file_name().unwrap()))
            .await
            .map_err(|e| format!("Failed to copy mod to backup: {e}"))?;
    } else {
        tokio::fs::copy(&path, backup_path.join(path.file_name().unwrap()))
            .await
            .map_err(|e| format!("Failed to copy mod file to backup: {e}"))?;
    }

    let metadata = json!({
        "original_path": path.to_string_lossy().to_string(),
        "backup_time": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    });

    tokio::fs::write(
        backup_path.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)
            .map_err(|e| format!("Failed to serialize metadata: {e}"))?,
    )
    .await
    .map_err(|e| format!("Failed to write metadata: {e}"))?;

    Ok(())
}

#[tauri::command]
pub async fn restore_from_backup(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let backup_dir = get_backup_dir_async().await?;

    let mut latest_backup = None;
    let mut latest_time = 0;
    let mut entries = tokio::fs::read_dir(&backup_dir)
        .await
        .map_err(|e| format!("Failed to read backup directory: {e}"))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read backup entry: {e}"))?
    {
        let metadata_path = entry.path().join("metadata.json");
        if tokio::fs::metadata(&metadata_path).await.is_ok() {
            let metadata: serde_json::Value = serde_json::from_str(
                &tokio::fs::read_to_string(&metadata_path)
                    .await
                    .map_err(|e| format!("Failed to read metadata file: {e}"))?,
            )
            .map_err(|e| format!("Failed to parse metadata: {e}"))?;
            if let Some(original_path) = metadata.get("original_path").and_then(|v| v.as_str())
                && original_path == path.to_string_lossy()
                && let Some(backup_time) = metadata.get("backup_time").and_then(|v| v.as_u64())
                && backup_time > latest_time
            {
                latest_time = backup_time;
                latest_backup = Some(entry.path());
            }
        }
    }

    let backup_path = latest_backup.ok_or_else(|| "No backup found for this path".to_string())?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create parent directory: {e}"))?;
    }
    let mut backup_entries = tokio::fs::read_dir(&backup_path)
        .await
        .map_err(|e| format!("Failed to read backup directory: {e}"))?;
    while let Some(entry) = backup_entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read backup entry: {e}"))?
    {
        let file_name = entry.file_name();
        if file_name == "metadata.json" {
            continue;
        }
        let dest_path = path.parent().unwrap().join(&file_name);
        let file_type = entry
            .file_type()
            .await
            .map_err(|e| format!("Failed to get file type: {e}"))?;
        if file_type.is_dir() {
            copy_dir_all_async(&entry.path(), &dest_path)
                .await
                .map_err(|e| format!("Failed to restore directory from backup: {e}"))?;
        } else {
            tokio::fs::copy(entry.path(), &dest_path)
                .await
                .map_err(|e| format!("Failed to restore file from backup: {e}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn remove_backup(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let backup_dir = get_backup_dir_async().await?;
    let mut entries = tokio::fs::read_dir(&backup_dir)
        .await
        .map_err(|e| format!("Failed to read backup directory: {e}"))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read backup entry: {e}"))?
    {
        let metadata_path = entry.path().join("metadata.json");
        if tokio::fs::metadata(&metadata_path).await.is_ok() {
            let metadata: serde_json::Value = serde_json::from_str(
                &tokio::fs::read_to_string(&metadata_path)
                    .await
                    .map_err(|e| format!("Failed to read metadata file: {e}"))?,
            )
            .map_err(|e| format!("Failed to parse metadata: {e}"))?;
            if let Some(original_path) = metadata.get("original_path").and_then(|v| v.as_str())
                && original_path == path.to_string_lossy()
            {
                tokio::fs::remove_dir_all(entry.path())
                    .await
                    .map_err(|e| format!("Failed to remove backup: {e}"))?;
            }
        }
    }
    Ok(())
}

async fn get_backup_dir_async() -> Result<PathBuf, String> {
    let temp_dir = std::env::temp_dir().join("balatro_mod_manager_backups");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| format!("Failed to create backup directory: {e}"))?;
    Ok(temp_dir)
}

async fn copy_dir_all_async(src: &Path, dst: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dst).await?;
    let mut entries = tokio::fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let ty = entry.file_type().await?;
        let path = entry.path();
        if ty.is_dir() {
            Box::pin(copy_dir_all_async(
                &path,
                &dst.join(path.file_name().unwrap()),
            ))
            .await?;
        } else {
            tokio::fs::copy(&path, dst.join(path.file_name().unwrap())).await?;
        }
    }
    Ok(())
}
