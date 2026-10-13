//! Backup and restore functionality for mod snapshots.
//!
//! This module provides automatic and manual backup capabilities for the Mods folder,
//! allowing users to restore to previous states after updates, uninstalls, or other changes.

use crate::database::{Database, InstalledMod};
use crate::errors::AppError;
use chrono::{DateTime, Utc};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use tar::{Archive, Builder};
use tokio::fs;

/// Maximum number of automatic backups to retain.
const MAX_AUTO_BACKUPS: usize = 5;

/// Minimum interval between auto-backups in seconds (debounce).
const AUTO_BACKUP_DEBOUNCE_SECS: i64 = 60;

/// The trigger that caused a backup to be created.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupTrigger {
    /// Backup created before updating a mod.
    AutoUpdate,
    /// Backup created before uninstalling a mod.
    AutoUninstall,
    /// Backup created before bulk enable/disable operations.
    AutoBulk,
    /// Backup created manually by the user.
    Manual,
}

impl BackupTrigger {
    /// Returns true if this is an automatic backup trigger.
    pub fn is_auto(&self) -> bool {
        matches!(
            self,
            BackupTrigger::AutoUpdate | BackupTrigger::AutoUninstall | BackupTrigger::AutoBulk
        )
    }

    /// Returns a human-readable description of the trigger.
    pub fn description(&self) -> &'static str {
        match self {
            BackupTrigger::AutoUpdate => "Before updating mod",
            BackupTrigger::AutoUninstall => "Before uninstalling mod",
            BackupTrigger::AutoBulk => "Before bulk operation",
            BackupTrigger::Manual => "Manual backup",
        }
    }
}

/// Metadata about a backup snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMetadata {
    /// Unique identifier for the backup.
    pub id: String,
    /// When the backup was created.
    pub created_at: DateTime<Utc>,
    /// What triggered the backup creation.
    pub trigger: BackupTrigger,
    /// User-provided name (for manual backups).
    pub name: Option<String>,
    /// Number of mods in the snapshot.
    pub mod_count: usize,
    /// Total size of the backup in bytes.
    pub size_bytes: u64,
    /// Lovely version at the time of backup.
    pub lovely_version: Option<String>,
}

/// Database snapshot containing installed mod records.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabaseSnapshot {
    /// List of installed mods at backup time.
    installed_mods: Vec<ModRecord>,
    /// List of enabled states (paths with .lovelyignore files).
    disabled_mods: Vec<String>,
}

/// A mod record for the database snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModRecord {
    name: String,
    path: String,
    dependencies: Vec<String>,
    current_version: Option<String>,
}

impl From<InstalledMod> for ModRecord {
    fn from(m: InstalledMod) -> Self {
        ModRecord {
            name: m.name,
            path: m.path,
            dependencies: m.dependencies,
            current_version: m.current_version,
        }
    }
}

/// Get the backups directory path.
pub fn get_backups_dir() -> Result<PathBuf, AppError> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("config directory")))?;
    Ok(config_dir.join("Balatro").join("backups"))
}

// Backup ids come from the frontend and must name a direct child of backups_dir
fn backup_dir_for(backups_dir: &Path, backup_id: &str) -> Result<PathBuf, AppError> {
    let mut components = Path::new(backup_id).components();
    let single_normal = matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(part)), None) if part == backup_id
    );
    if !single_normal || backup_id.contains(['/', '\\', ':', '\0']) {
        return Err(AppError::InvalidState(format!(
            "Invalid backup id '{}'",
            backup_id
        )));
    }
    Ok(backups_dir.join(backup_id))
}

/// Create a backup of the current mods folder and database state.
///
/// # Arguments
/// * `trigger` - What triggered this backup
/// * `name` - Optional user-provided name (for manual backups)
/// * `context` - Optional context info (e.g., mod name being updated)
///
/// # Returns
/// Metadata about the created backup, or an error.
pub async fn create_backup(
    trigger: BackupTrigger,
    name: Option<String>,
    context: Option<String>,
) -> Result<BackupMetadata, AppError> {
    let mods_dir = crate::mods_dir();
    let backups_dir = get_backups_dir()?;

    // Check if we should debounce auto-backups
    if trigger.is_auto()
        && let Ok(backups) = list_backups().await
        && let Some(latest) = backups.first()
    {
        let elapsed = Utc::now()
            .signed_duration_since(latest.created_at)
            .num_seconds();
        if elapsed < AUTO_BACKUP_DEBOUNCE_SECS && latest.trigger.is_auto() {
            log::debug!(
                "Skipping auto-backup: last backup was {}s ago (debounce: {}s)",
                elapsed,
                AUTO_BACKUP_DEBOUNCE_SECS
            );
            return Ok(latest.clone());
        }
    }

    // Create backups directory if it doesn't exist
    fs::create_dir_all(&backups_dir)
        .await
        .map_err(|e| AppError::DirCreate {
            path: backups_dir.clone(),
            source: e.to_string(),
        })?;

    // Generate backup ID and directory name
    let now = Utc::now();
    let trigger_str = match trigger {
        BackupTrigger::AutoUpdate => "auto_update",
        BackupTrigger::AutoUninstall => "auto_uninstall",
        BackupTrigger::AutoBulk => "auto_bulk",
        BackupTrigger::Manual => "manual",
    };

    let id = format!("{}_{}", now.format("%Y%m%d_%H%M%S"), trigger_str);
    let backup_dir = backups_dir.join(&id);

    fs::create_dir_all(&backup_dir)
        .await
        .map_err(|e| AppError::DirCreate {
            path: backup_dir.clone(),
            source: e.to_string(),
        })?;

    // Get database state
    let db = Database::new()?;
    let installed_mods = db.get_installed_mods()?;
    let mod_count = installed_mods.len();
    let lovely_version = db.get_lovely_version().ok().flatten();

    // Find disabled mods (those with .lovelyignore files)
    let disabled_mods = find_disabled_mods(&mods_dir).await?;

    // Create database snapshot
    let db_snapshot = DatabaseSnapshot {
        installed_mods: installed_mods.into_iter().map(ModRecord::from).collect(),
        disabled_mods,
    };

    let db_snapshot_path = backup_dir.join("database.json");
    let db_json = serde_json::to_string_pretty(&db_snapshot)?;
    fs::write(&db_snapshot_path, &db_json)
        .await
        .map_err(|e| AppError::InvalidState(format!("Failed to write database snapshot: {}", e)))?;

    // Create compressed tar archive of mods folder (CPU-intensive, run in blocking task)
    let archive_path = backup_dir.join("mods.tar.gz");
    let mods_dir_clone = mods_dir.clone();
    let archive_path_clone = archive_path.clone();
    tokio::task::spawn_blocking(move || {
        create_mods_archive_sync(&mods_dir_clone, &archive_path_clone)
    })
    .await
    .map_err(|e| AppError::InvalidState(format!("Archive task failed: {}", e)))??;

    // Calculate total size
    let archive_size = fs::metadata(&archive_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let db_size = fs::metadata(&db_snapshot_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let size_bytes = archive_size + db_size;

    // Build display name
    let display_name = match (&name, &context) {
        (Some(n), _) => Some(n.clone()),
        (None, Some(ctx)) => Some(format!("{} {}", trigger.description(), ctx)),
        (None, None) => None,
    };

    // Create metadata
    let metadata = BackupMetadata {
        id: id.clone(),
        created_at: now,
        trigger,
        name: display_name,
        mod_count,
        size_bytes,
        lovely_version,
    };

    // Write metadata file
    let metadata_path = backup_dir.join("metadata.json");
    let metadata_json = serde_json::to_string_pretty(&metadata)?;
    fs::write(&metadata_path, &metadata_json)
        .await
        .map_err(|e| AppError::InvalidState(format!("Failed to write backup metadata: {}", e)))?;

    // Enforce retention policy for auto-backups
    enforce_retention().await?;

    log::info!(
        "Created backup '{}': {} mods, {} bytes",
        id,
        mod_count,
        size_bytes
    );

    Ok(metadata)
}

/// Find mods that have .lovelyignore files (disabled mods).
async fn find_disabled_mods(mods_dir: &Path) -> Result<Vec<String>, AppError> {
    let mut disabled = Vec::new();

    if !mods_dir.exists() {
        return Ok(disabled);
    }

    let mut entries = fs::read_dir(mods_dir)
        .await
        .map_err(|e| AppError::InvalidState(format!("Failed to read mods directory: {}", e)))?;

    while let Some(entry) = entries.next_entry().await.transpose() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            let ignore_file = path.join(".lovelyignore");
            if ignore_file.exists()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                disabled.push(name.to_string());
            }
        }
    }

    Ok(disabled)
}

/// Create a compressed tar archive of the mods folder (synchronous, CPU-intensive).
fn create_mods_archive_sync(mods_dir: &Path, archive_path: &Path) -> Result<(), AppError> {
    let file = File::create(archive_path)
        .map_err(|e| AppError::InvalidState(format!("Failed to create archive file: {}", e)))?;

    // Use fast compression (level 1) for speed
    let encoder = GzEncoder::new(file, Compression::fast());
    let mut builder = Builder::new(encoder);

    if mods_dir.exists() {
        // Add all contents of mods directory to the archive
        let entries = std::fs::read_dir(mods_dir)
            .map_err(|e| AppError::InvalidState(format!("Failed to read mods directory: {}", e)))?;

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();

            // Skip hidden files and known non-mod directories
            if let Some(name_str) = name.to_str() {
                let lower = name_str.to_lowercase();
                if lower.starts_with('.')
                    || lower.contains("lovely")
                    || matches!(lower.as_str(), ".git" | "node_modules" | "__macosx")
                {
                    continue;
                }
            }

            if path.is_dir() {
                builder.append_dir_all(&name, &path).map_err(|e| {
                    AppError::InvalidState(format!("Failed to add directory to archive: {}", e))
                })?;
            } else if path.is_file() {
                builder.append_path_with_name(&path, &name).map_err(|e| {
                    AppError::InvalidState(format!("Failed to add file to archive: {}", e))
                })?;
            }
        }
    }

    builder
        .finish()
        .map_err(|e| AppError::InvalidState(format!("Failed to finish archive: {}", e)))?;

    Ok(())
}

/// List all backups, sorted by creation time (newest first).
pub async fn list_backups() -> Result<Vec<BackupMetadata>, AppError> {
    let backups_dir = get_backups_dir()?;

    if !backups_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(&backups_dir)
        .await
        .map_err(|e| AppError::InvalidState(format!("Failed to read backups directory: {}", e)))?;

    let mut backups = Vec::new();

    while let Some(entry) = entries.next_entry().await.transpose() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let metadata_path = path.join("metadata.json");
        if !metadata_path.exists() {
            continue;
        }

        match fs::read_to_string(&metadata_path).await {
            Ok(content) => match serde_json::from_str::<BackupMetadata>(&content) {
                Ok(metadata) => backups.push(metadata),
                Err(e) => {
                    log::warn!(
                        "Failed to parse backup metadata at {:?}: {}",
                        metadata_path,
                        e
                    );
                }
            },
            Err(e) => {
                log::warn!(
                    "Failed to read backup metadata at {:?}: {}",
                    metadata_path,
                    e
                );
            }
        }
    }

    // Sort by creation time, newest first
    backups.sort_by_key(|b| std::cmp::Reverse(b.created_at));

    Ok(backups)
}

/// Restore a backup by ID.
///
/// This will:
/// 1. Validate the backup integrity
/// 2. Remove current mods folder contents
/// 3. Extract the backup archive
/// 4. Restore database state
/// 5. Restore enabled/disabled states
pub async fn restore_backup(backup_id: &str) -> Result<(), AppError> {
    let backups_dir = get_backups_dir()?;
    let backup_dir = backup_dir_for(&backups_dir, backup_id)?;

    if !backup_dir.exists() {
        return Err(AppError::InvalidState(format!(
            "Backup '{}' not found",
            backup_id
        )));
    }

    let archive_path = backup_dir.join("mods.tar.gz");
    let db_snapshot_path = backup_dir.join("database.json");

    // Validate backup integrity
    if !archive_path.exists() {
        return Err(AppError::InvalidState(format!(
            "Backup '{}' is corrupted: missing mods archive",
            backup_id
        )));
    }

    if !db_snapshot_path.exists() {
        return Err(AppError::InvalidState(format!(
            "Backup '{}' is corrupted: missing database snapshot",
            backup_id
        )));
    }

    // Read database snapshot
    let db_content = fs::read_to_string(&db_snapshot_path)
        .await
        .map_err(|e| AppError::InvalidState(format!("Failed to read database snapshot: {}", e)))?;

    let db_snapshot: DatabaseSnapshot = serde_json::from_str(&db_content)
        .map_err(|e| AppError::InvalidState(format!("Failed to parse database snapshot: {}", e)))?;

    let mods_dir = crate::mods_dir();

    // Create a marker file to indicate restore is in progress
    let restore_marker = mods_dir
        .parent()
        .unwrap_or(&mods_dir)
        .join(".restore_in_progress");

    fs::write(&restore_marker, backup_id)
        .await
        .map_err(|e| AppError::InvalidState(format!("Failed to create restore marker: {}", e)))?;

    // Extract to a temporary directory first (atomic approach)
    let temp_dir = mods_dir
        .parent()
        .unwrap_or(&mods_dir)
        .join(".mods_restore_temp");

    // Clean up any previous failed restore
    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| AppError::DirCreate {
            path: temp_dir.clone(),
            source: e.to_string(),
        })?;

    // Extract archive to temp directory (CPU-intensive, run in blocking task)
    let archive_path_clone = archive_path.clone();
    let temp_dir_clone = temp_dir.clone();
    tokio::task::spawn_blocking(move || {
        extract_mods_archive_sync(&archive_path_clone, &temp_dir_clone)
    })
    .await
    .map_err(|e| AppError::InvalidState(format!("Extract task failed: {}", e)))??;

    // Clear current mods folder (except Lovely-related files)
    clear_mods_folder(&mods_dir).await?;

    // Move extracted mods to mods folder
    move_restored_mods(&temp_dir, &mods_dir).await?;

    // Restore database state
    let db = Database::new()?;
    restore_database_state(&db, &db_snapshot)?;

    // Restore enabled/disabled states
    restore_enabled_states(&mods_dir, &db_snapshot.disabled_mods).await?;

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir).await;
    let _ = fs::remove_file(&restore_marker).await;

    // Clear mod detection cache to reflect new state
    crate::local_mod_detection::clear_detection_cache();

    log::info!("Successfully restored backup '{}'", backup_id);

    Ok(())
}

/// Extract the mods archive to a directory (synchronous, CPU-intensive).
fn extract_mods_archive_sync(archive_path: &Path, dest_dir: &Path) -> Result<(), AppError> {
    let file = File::open(archive_path)
        .map_err(|e| AppError::InvalidState(format!("Failed to open archive: {}", e)))?;

    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    archive
        .unpack(dest_dir)
        .map_err(|e| AppError::InvalidState(format!("Failed to extract archive: {}", e)))?;

    Ok(())
}

/// Clear the mods folder, preserving Lovely-related files.
async fn clear_mods_folder(mods_dir: &Path) -> Result<(), AppError> {
    if !mods_dir.exists() {
        return Ok(());
    }

    let mut entries = fs::read_dir(mods_dir)
        .await
        .map_err(|e| AppError::InvalidState(format!("Failed to read mods directory: {}", e)))?;

    while let Some(entry) = entries.next_entry().await.transpose() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        let name = entry.file_name();

        // Skip Lovely-related files and hidden files
        if let Some(name_str) = name.to_str() {
            let lower = name_str.to_lowercase();
            if lower.contains("lovely") || lower.starts_with('.') {
                continue;
            }
        }

        if path.is_dir() {
            fs::remove_dir_all(&path).await.map_err(|e| {
                AppError::InvalidState(format!("Failed to remove directory {:?}: {}", path, e))
            })?;
        } else {
            fs::remove_file(&path).await.map_err(|e| {
                AppError::InvalidState(format!("Failed to remove file {:?}: {}", path, e))
            })?;
        }
    }

    Ok(())
}

/// Move restored mods from temp directory to mods folder.
async fn move_restored_mods(temp_dir: &Path, mods_dir: &Path) -> Result<(), AppError> {
    if !temp_dir.exists() {
        return Ok(());
    }

    let mut entries = fs::read_dir(temp_dir)
        .await
        .map_err(|e| AppError::InvalidState(format!("Failed to read temp directory: {}", e)))?;

    while let Some(entry) = entries.next_entry().await.transpose() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let src_path = entry.path();
        let dest_path = mods_dir.join(entry.file_name());

        if src_path.is_dir() {
            // Use rename if possible (same filesystem), otherwise copy
            if fs::rename(&src_path, &dest_path).await.is_err() {
                let src_clone = src_path.clone();
                let dest_clone = dest_path.clone();
                tokio::task::spawn_blocking(move || {
                    copy_dir_recursive_sync(&src_clone, &dest_clone)
                })
                .await
                .map_err(|e| AppError::InvalidState(format!("Copy task failed: {}", e)))??;
                let _ = fs::remove_dir_all(&src_path).await;
            }
        } else if fs::rename(&src_path, &dest_path).await.is_err() {
            fs::copy(&src_path, &dest_path)
                .await
                .map_err(|e| AppError::InvalidState(format!("Failed to copy file: {}", e)))?;
            let _ = fs::remove_file(&src_path).await;
        }
    }

    Ok(())
}

/// Recursively copy a directory (synchronous, for spawn_blocking).
fn copy_dir_recursive_sync(src: &Path, dest: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(dest).map_err(|e| AppError::DirCreate {
        path: dest.to_path_buf(),
        source: e.to_string(),
    })?;

    let entries = std::fs::read_dir(src).map_err(|e| {
        AppError::InvalidState(format!("Failed to read directory {:?}: {}", src, e))
    })?;

    for entry in entries.flatten() {
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive_sync(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)
                .map_err(|e| AppError::InvalidState(format!("Failed to copy file: {}", e)))?;
        }
    }

    Ok(())
}

/// Restore the database state from a snapshot.
fn restore_database_state(db: &Database, snapshot: &DatabaseSnapshot) -> Result<(), AppError> {
    // Clear existing installed mods
    for existing in db.get_installed_mods()? {
        db.remove_installed_mod(&existing.name)?;
    }

    // Add mods from snapshot
    for mod_record in &snapshot.installed_mods {
        db.add_installed_mod(
            &mod_record.name,
            &mod_record.path,
            &mod_record.dependencies,
            mod_record.current_version.clone(),
        )?;
    }

    Ok(())
}

/// Restore enabled/disabled states for mods.
async fn restore_enabled_states(mods_dir: &Path, disabled_mods: &[String]) -> Result<(), AppError> {
    // First, remove all .lovelyignore files
    if mods_dir.exists() {
        let mut entries = fs::read_dir(mods_dir)
            .await
            .map_err(|e| AppError::InvalidState(format!("Failed to read mods directory: {}", e)))?;

        while let Some(entry) = entries.next_entry().await.transpose() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.is_dir() {
                let ignore_file = path.join(".lovelyignore");
                let _ = fs::remove_file(&ignore_file).await;
            }
        }
    }

    // Add .lovelyignore files for disabled mods
    for mod_name in disabled_mods {
        let mod_path = mods_dir.join(mod_name);
        if mod_path.exists() && mod_path.is_dir() {
            let ignore_file = mod_path.join(".lovelyignore");
            fs::write(&ignore_file, "").await.map_err(|e| {
                AppError::InvalidState(format!(
                    "Failed to create .lovelyignore for {}: {}",
                    mod_name, e
                ))
            })?;
        }
    }

    Ok(())
}

/// Delete a backup by ID.
pub async fn delete_backup(backup_id: &str) -> Result<(), AppError> {
    let backups_dir = get_backups_dir()?;
    let backup_dir = backup_dir_for(&backups_dir, backup_id)?;

    if !backup_dir.exists() {
        return Err(AppError::InvalidState(format!(
            "Backup '{}' not found",
            backup_id
        )));
    }

    fs::remove_dir_all(&backup_dir)
        .await
        .map_err(|e| AppError::InvalidState(format!("Failed to delete backup: {}", e)))?;

    log::info!("Deleted backup '{}'", backup_id);

    Ok(())
}

/// Enforce the retention policy for automatic backups.
/// Keeps only the most recent MAX_AUTO_BACKUPS automatic backups.
pub async fn enforce_retention() -> Result<(), AppError> {
    let backups = list_backups().await?;

    // Filter to only auto backups
    let auto_backups: Vec<_> = backups.iter().filter(|b| b.trigger.is_auto()).collect();

    // Delete oldest auto backups if we exceed the limit
    if auto_backups.len() > MAX_AUTO_BACKUPS {
        for backup in auto_backups.iter().skip(MAX_AUTO_BACKUPS) {
            if let Err(e) = delete_backup(&backup.id).await {
                log::warn!("Failed to delete old backup '{}': {}", backup.id, e);
            }
        }
    }

    Ok(())
}

/// Get the total size of all backups in bytes.
pub async fn get_total_backups_size() -> Result<u64, AppError> {
    let backups = list_backups().await?;
    Ok(backups.iter().map(|b| b.size_bytes).sum())
}

/// Check if a restore was interrupted and return the backup ID if so.
pub fn check_interrupted_restore() -> Option<String> {
    let config_dir = dirs::config_dir()?;
    let restore_marker = config_dir.join("Balatro").join(".restore_in_progress");

    if restore_marker.exists() {
        std::fs::read_to_string(&restore_marker).ok()
    } else {
        None
    }
}

/// Clear the interrupted restore marker.
pub fn clear_interrupted_restore_marker() -> Result<(), AppError> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("config directory")))?;
    let restore_marker = config_dir.join("Balatro").join(".restore_in_progress");

    if restore_marker.exists() {
        std::fs::remove_file(&restore_marker).map_err(|e| {
            AppError::InvalidState(format!("Failed to remove restore marker: {}", e))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_backup_dir_for_rejects_traversal() {
        let backups_dir = Path::new("/backups");
        assert_eq!(
            backup_dir_for(backups_dir, "20240101_000000_manual").unwrap(),
            backups_dir.join("20240101_000000_manual")
        );
        for id in ["", ".", "..", "../x", "a/b", "/abs", "a\\b", "C:x", "x\0"] {
            assert!(backup_dir_for(backups_dir, id).is_err(), "{id:?}");
        }
    }

    #[test]
    fn test_backup_trigger_is_auto() {
        assert!(BackupTrigger::AutoUpdate.is_auto());
        assert!(BackupTrigger::AutoUninstall.is_auto());
        assert!(BackupTrigger::AutoBulk.is_auto());
        assert!(!BackupTrigger::Manual.is_auto());
    }

    #[test]
    fn test_backup_trigger_description() {
        assert_eq!(
            BackupTrigger::AutoUpdate.description(),
            "Before updating mod"
        );
        assert_eq!(BackupTrigger::Manual.description(), "Manual backup");
    }

    #[test]
    fn test_mod_record_from_installed_mod() {
        let installed = InstalledMod {
            name: "TestMod".to_string(),
            path: "/path/to/mod".to_string(),
            dependencies: vec!["Steamodded".to_string()],
            current_version: Some("1.0.0".to_string()),
            orphaned: false,
        };

        let record = ModRecord::from(installed);
        assert_eq!(record.name, "TestMod");
        assert_eq!(record.path, "/path/to/mod");
        assert_eq!(record.dependencies, vec!["Steamodded"]);
        assert_eq!(record.current_version, Some("1.0.0".to_string()));
    }

    #[test]
    fn test_find_disabled_mods() {
        let temp_dir = TempDir::new().unwrap();
        let mods_dir = temp_dir.path();

        // Create some mod directories
        let mod1 = mods_dir.join("EnabledMod");
        let mod2 = mods_dir.join("DisabledMod");
        std::fs::create_dir_all(&mod1).unwrap();
        std::fs::create_dir_all(&mod2).unwrap();

        // Add .lovelyignore to disabled mod
        std::fs::write(mod2.join(".lovelyignore"), "").unwrap();

        // Use tokio runtime for async test
        let rt = tokio::runtime::Runtime::new().unwrap();
        let disabled = rt.block_on(find_disabled_mods(mods_dir)).unwrap();
        assert_eq!(disabled, vec!["DisabledMod"]);
    }

    #[test]
    fn test_create_and_extract_archive() {
        let temp_dir = TempDir::new().unwrap();
        let mods_dir = temp_dir.path().join("mods");
        let archive_path = temp_dir.path().join("test.tar.gz");
        let extract_dir = temp_dir.path().join("extracted");

        // Create test mod structure
        let test_mod = mods_dir.join("TestMod");
        std::fs::create_dir_all(&test_mod).unwrap();
        std::fs::write(test_mod.join("main.lua"), "-- test mod").unwrap();
        std::fs::write(test_mod.join("config.json"), "{}").unwrap();

        // Create archive
        create_mods_archive_sync(&mods_dir, &archive_path).unwrap();
        assert!(archive_path.exists());

        // Extract archive
        std::fs::create_dir_all(&extract_dir).unwrap();
        extract_mods_archive_sync(&archive_path, &extract_dir).unwrap();

        // Verify extracted contents
        let extracted_mod = extract_dir.join("TestMod");
        assert!(extracted_mod.exists());
        assert!(extracted_mod.join("main.lua").exists());
        assert!(extracted_mod.join("config.json").exists());
    }
}
