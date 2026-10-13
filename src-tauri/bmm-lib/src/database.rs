//! SQLite database for storing app settings and installed mod metadata.
//!
//! This module provides a [`Database`] struct that handles all persistent storage
//! for the mod manager, including:
//! - Installed mod tracking
//! - User settings (Discord RPC, lovely console, launch mode, etc.)
//! - Installation paths and version information
//!
//! The database uses SQLite with automatic schema migrations.

use crate::errors::AppError;
use rusqlite::Connection;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

/// Handle to the SQLite database storing app settings and mod metadata.
///
/// All database operations are synchronous. For async contexts, wrap access
/// in a tokio Mutex or similar synchronization primitive.
pub struct Database {
    conn: Connection,
}

/// Represents an installed mod stored in the database.
#[derive(Serialize)]
pub struct InstalledMod {
    /// The display name of the mod.
    pub name: String,
    /// Filesystem path where the mod is installed.
    pub path: String,
    /// List of dependency mod names.
    pub dependencies: Vec<String>,
    /// Currently installed version string, if known.
    pub current_version: Option<String>,
    /// True when the mod is no longer present in the remote index.
    /// The mod still exists locally; this flag is set by reconciliation
    /// after a successful catalog fetch.
    #[serde(default)]
    pub orphaned: bool,
}

impl Database {
    const CURRENT_DB_VERSION: (u32, u32) = (1, 3); // Update this when schema changes

    /// Converts a version tuple to a string representation.
    fn version_to_string(v: (u32, u32)) -> String {
        format!("{}.{}", v.0, v.1)
    }

    /// Parses a version string into a tuple.
    fn parse_version(s: &str) -> Option<(u32, u32)> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() == 2 {
            let major = parts[0].parse().ok()?;
            let minor = parts[1].parse().ok()?;
            Some((major, minor))
        } else {
            None
        }
    }

    /// Creates a new Database instance, opening or creating the SQLite file.
    ///
    /// The database is stored in the platform's config directory under `Balatro/`.
    /// Automatic schema migrations are performed if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be found, the database
    /// cannot be opened, or migrations fail.
    pub fn new() -> Result<Self, AppError> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| AppError::DirNotFound(PathBuf::from("config directory")))?;
        let balatro_dir = config_dir.join("Balatro");
        let storage_path = resolve_storage_path(&balatro_dir);

        // Create directory if it doesn't exist (only for the primary config path)
        if let Some(parent) = storage_path.parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::DirNotFound(format!("Failed to create config directory: {e}").into())
            })?;
        }

        // Try to open the database with a retry mechanism
        let mut retry_count = 0;
        let max_retries = 3;

        while retry_count < max_retries {
            // Try to open or create the database
            let conn_result = if storage_path.exists() {
                Connection::open(&storage_path)
            } else {
                // Create a new database
                let conn = Connection::open(&storage_path)
                    .map_err(|e| AppError::DatabaseInit(e.to_string()))?;
                Self::initialize_database(&conn)?;
                Ok(conn)
            };

            match conn_result {
                Ok(conn) => {
                    // Ensure WAL mode and performance pragmas are set on every connection.
                    // This handles existing databases that may not have WAL enabled yet.
                    Self::configure_connection(&conn)?;

                    // Check if database needs migration
                    if Self::needs_migration(&conn)? {
                        // Close the connection before migration
                        drop(conn);

                        // Perform migration
                        Self::migrate_database(&storage_path)?;

                        // Reopen the database after migration
                        let conn = Connection::open(&storage_path)
                            .map_err(|e| AppError::DatabaseInit(e.to_string()))?;
                        Self::configure_connection(&conn)?;
                        return Ok(Database { conn });
                    }

                    return Ok(Database { conn });
                }
                Err(e) => {
                    if retry_count == max_retries - 1 {
                        return Err(AppError::DatabaseInit(format!(
                            "Failed to open database after {max_retries} attempts: {e}"
                        )));
                    }

                    log::warn!(
                        "Failed to open database, retrying ({}/{}): {}",
                        retry_count + 1,
                        max_retries,
                        e
                    );
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    retry_count += 1;
                }
            }
        }

        Err(AppError::DatabaseInit(
            "Failed to open database after maximum retries".to_string(),
        ))
    }

    /// Configure connection with performance and reliability pragmas.
    /// This ensures WAL mode and other optimizations are applied to every connection.
    fn configure_connection(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA wal_autocheckpoint=500;
             PRAGMA busy_timeout=5000;",
        )
        .map_err(|e| AppError::DatabaseInit(format!("Failed to configure database: {e}")))?;
        Ok(())
    }

    /// Force a WAL checkpoint to merge the WAL file into the main database.
    /// Call this on app shutdown for a cleaner database state.
    pub fn checkpoint(&self) -> Result<(), AppError> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| AppError::DatabaseInit(format!("Failed to checkpoint WAL: {e}")))?;
        Ok(())
    }

    /// Get a setting value using cached prepared statement.
    /// Returns None if the setting doesn't exist.
    fn get_setting(&self, setting: &str) -> Result<Option<String>, AppError> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT value FROM settings WHERE setting = ?1")?;
        let mut rows = stmt.query([setting])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Set a setting value using cached prepared statement.
    fn set_setting(&self, setting: &str, value: &str) -> Result<(), AppError> {
        let mut stmt = self
            .conn
            .prepare_cached("INSERT OR REPLACE INTO settings (setting, value) VALUES (?1, ?2)")?;
        stmt.execute([setting, value])?;
        Ok(())
    }

    // Check if database needs migration
    fn needs_migration(conn: &Connection) -> Result<bool, AppError> {
        // First check if the version table exists
        let has_version_setting = match conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='settings'",
            [],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(count) if count > 0 => {
                // Now check if the version setting exists
                match conn.query_row(
                    "SELECT COUNT(*) FROM settings WHERE setting = 'db_version'",
                    [],
                    |row| row.get::<_, i64>(0),
                ) {
                    Ok(count) => count > 0,
                    Err(_) => false,
                }
            }
            _ => false,
        };

        // If no version in settings, this is an old database needing migration
        if !has_version_setting {
            return Ok(true);
        }

        // Check if the version matches
        match conn.query_row(
            "SELECT value FROM settings WHERE setting = 'db_version'",
            [],
            |row| row.get::<_, String>(0),
        ) {
            Ok(version) => match Self::parse_version(&version) {
                Some(v) => Ok(v < Self::CURRENT_DB_VERSION),
                None => Ok(true), // Invalid version format, needs migration
            },
            Err(_) => Ok(true), // If we can't get the version, assume migration is needed
        }
    }

    fn migrate_database(db_path: &PathBuf) -> Result<(), AppError> {
        // Create a temporary database path
        let temp_db_path = db_path.with_file_name("bmm_storage_new.db");

        // If the old database exists but we can't access it, try with a retry mechanism
        let max_retries = 3;
        let mut retry_count = 0;

        while retry_count < max_retries {
            // Open connections to both databases
            let old_conn_result = Connection::open(db_path);

            if let Err(e) = old_conn_result {
                if retry_count == max_retries - 1 {
                    return Err(AppError::DatabaseInit(format!(
                        "Failed to open old database after {max_retries} retries: {e}"
                    )));
                }

                // Wait before retrying
                std::thread::sleep(std::time::Duration::from_millis(500));
                retry_count += 1;
                continue;
            }

            let old_conn = old_conn_result.unwrap();
            let new_conn = Connection::open(&temp_db_path).map_err(|e| {
                AppError::DatabaseInit(format!("Failed to create new database: {e}"))
            })?;

            // Initialize the new database with current schema
            Self::initialize_database(&new_conn)?;

            // Migrate data
            Self::migrate_settings(&old_conn, &new_conn)?;
            Self::migrate_installed_mods(&old_conn, &new_conn)?;

            // IMPORTANT: Explicitly close connections before file operations
            drop(old_conn);
            drop(new_conn);

            // Add a small delay to ensure all handles are released
            std::thread::sleep(std::time::Duration::from_millis(100));

            // Try to backup the old database - fail if we can't
            let backup_path = db_path.with_extension("db.bak");

            // Remove old backup if it exists
            if backup_path.exists()
                && let Err(e) = std::fs::remove_file(&backup_path)
            {
                log::warn!("Failed to remove old backup file: {e}");
            }

            // Backup is mandatory - don't proceed without it
            std::fs::rename(db_path, &backup_path).map_err(|e| {
                // Try to clean up the new database we created
                let _ = std::fs::remove_file(&temp_db_path);
                AppError::DatabaseInit(format!(
                    "Failed to backup database before migration: {e}. \
                     Migration aborted to prevent data loss."
                ))
            })?;

            // Replace with the new one
            match std::fs::rename(&temp_db_path, db_path) {
                Ok(_) => return Ok(()),
                Err(e) => {
                    return Err(AppError::DatabaseInit(format!(
                        "Failed to install new database: {e}"
                    )));
                }
            }
        }

        Err(AppError::DatabaseInit(
            "Failed to access database after maximum retries".to_string(),
        ))
    }

    fn migrate_settings(old_conn: &Connection, new_conn: &Connection) -> Result<(), AppError> {
        // Check if settings table exists in old database
        let has_settings = match old_conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='settings'",
            [],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(count) => count > 0,
            Err(_) => false,
        };

        if !has_settings {
            return Ok(()); // No settings to migrate
        }

        // Get all settings except db_version (which will be set by initialize_database)
        let mut stmt = match old_conn
            .prepare("SELECT setting, value FROM settings WHERE setting != 'db_version'")
        {
            Ok(stmt) => stmt,
            Err(_) => return Ok(()), // If query fails, just continue
        };

        for (setting, value) in stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .flatten()
        {
            new_conn.execute(
                "INSERT OR REPLACE INTO settings (setting, value) VALUES (?1, ?2)",
                [&setting, &value],
            )?;
        }

        if let Ok(value) = old_conn.query_row(
            "SELECT value FROM settings WHERE setting = 'security_warning_acknowledged'",
            [],
            |row| row.get::<_, String>(0),
        ) {
            new_conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('security_warning_acknowledged', ?1)",
            [&value],
        )?;
        }

        Ok(())
    }
    // Migrate installed mods from old database to new one
    fn migrate_installed_mods(
        old_conn: &Connection,
        new_conn: &Connection,
    ) -> Result<(), AppError> {
        // Check if installed_mods table exists in old database
        let has_installed_mods = match old_conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='installed_mods'",
            [],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(count) => count > 0,
            Err(_) => false,
        };

        if !has_installed_mods {
            return Ok(()); // No mods to migrate
        }

        // Get all installed mods
        let mut stmt = match old_conn
            .prepare("SELECT name, path, dependencies, current_version FROM installed_mods")
        {
            Ok(stmt) => stmt,
            Err(_) => return Ok(()), // If query fails, just continue
        };

        for (name, path, dependencies, current_version) in stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .flatten()
        {
            new_conn.execute(
            "INSERT INTO installed_mods (name, path, dependencies, current_version) VALUES (?1, ?2, ?3, ?4)",
            [&name, &path, &dependencies, &current_version.unwrap_or_default()],
        )?;
        }

        Ok(())
    }

    fn initialize_database(conn: &Connection) -> Result<(), AppError> {
        // Enable WAL mode for better concurrent read performance and reliability.
        // WAL mode allows readers to not block writers and provides better crash recovery.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA wal_autocheckpoint=500;
             PRAGMA busy_timeout=5000;",
        )
        .map_err(|e| AppError::DatabaseInit(format!("Failed to enable WAL mode: {e}")))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
                setting TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| AppError::DatabaseInit(e.to_string()))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS installed_mods (
                name TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                dependencies TEXT NOT NULL DEFAULT '[]',
                current_version TEXT,
                orphaned INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .map_err(|e| AppError::DatabaseInit(e.to_string()))?;

        // Create index on path for faster lookups when checking mod installation
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_installed_mods_path ON installed_mods(path)",
            [],
        )
        .map_err(|e| AppError::DatabaseInit(e.to_string()))?;

        // Create index on name for faster lookups by mod name
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_installed_mods_name ON installed_mods(name)",
            [],
        )
        .map_err(|e| AppError::DatabaseInit(e.to_string()))?;

        // Set the database version
        conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('db_version', ?1)",
            [&Self::version_to_string(Self::CURRENT_DB_VERSION)],
        )
        .map_err(|e| AppError::DatabaseInit(e.to_string()))?;

        conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('security_warning_acknowledged', ?1)",
            ["no"],
        )
        .map_err(|e| AppError::DatabaseInit(e.to_string()))?;

        Ok(())
    }

    pub fn get_mod_details(&self, mod_name: &str) -> Result<InstalledMod, AppError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, path, dependencies, current_version, orphaned FROM installed_mods WHERE name = ?1",
        )?;

        let mut rows = stmt.query([mod_name])?;

        if let Some(row) = rows.next()? {
            Ok(InstalledMod {
                name: row.get(0)?,
                path: row.get(1)?,
                dependencies: serde_json::from_str(&row.get::<_, String>(2)?)?,
                current_version: row.get(3)?,
                orphaned: row.get::<_, i64>(4)? != 0,
            })
        } else {
            Err(AppError::InvalidState(format!("Mod {mod_name} not found")))
        }
    }

    pub fn set_discord_rpc_enabled(&self, enabled: bool) -> Result<(), AppError> {
        let value = if enabled { "enabled" } else { "disabled" };
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('discord_rpc', ?1)",
            [value],
        )?;
        Ok(())
    }

    pub fn is_discord_rpc_enabled(&self) -> Result<bool, AppError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE setting = 'discord_rpc'")?;
        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            Ok(row.get::<_, String>(0)? == "enabled")
        } else {
            // Default to enabled if setting doesn't exist yet
            self.set_discord_rpc_enabled(true)?;
            Ok(true)
        }
    }

    pub fn set_last_fetched(&self, timestamp: u64) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('last_fetched', ?1)",
            [timestamp.to_string()],
        )?;
        Ok(())
    }

    pub fn get_last_fetched(&self) -> Result<u64, AppError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE setting = 'last_fetched'")?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let val: String = row.get(0)?;
            val.parse()
                .map_err(|_| AppError::InvalidState("Invalid timestamp format".to_string()))
        } else {
            Ok(0)
        }
    }

    pub fn get_installed_mods(&self) -> Result<Vec<InstalledMod>, AppError> {
        // Use prepare_cached for frequently called queries to avoid re-parsing SQL
        let mut stmt = self.conn.prepare_cached(
            "SELECT name, path, dependencies, current_version, orphaned FROM installed_mods",
        )?;
        let mut mods = Vec::new();
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            mods.push(InstalledMod {
                name: row.get(0)?,
                path: row.get(1)?,
                dependencies: serde_json::from_str(&row.get::<_, String>(2)?)?,
                current_version: row.get(3)?,
                orphaned: row.get::<_, i64>(4)? != 0,
            });
        }

        Ok(mods)
    }

    pub fn add_installed_mod(
        &self,
        name: &str,
        path: &str,
        dependencies: &[String],
        current_version: Option<String>,
    ) -> Result<(), AppError> {
        let deps_json = serde_json::to_string(dependencies)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO installed_mods (name, path, dependencies, current_version) VALUES (?1, ?2, ?3, ?4)",
            [name, path, &deps_json, &current_version.unwrap_or_default()],
        )?;
        Ok(())
    }

    pub fn get_dependents(&self, mod_name: &str) -> Result<Vec<String>, AppError> {
        // Use prepare_cached for frequently called queries
        let mut stmt = self.conn.prepare_cached(
            "SELECT name FROM installed_mods
            WHERE EXISTS (
                SELECT 1 FROM json_each(dependencies)
                WHERE TRIM(json_each.value, '\"') = ?1
            )",
        )?;

        let mut rows = stmt.query([mod_name])?;
        let mut dependents = Vec::new();

        while let Some(row) = rows.next()? {
            dependents.push(row.get(0)?);
        }

        Ok(dependents)
    }

    /// Get all mods that depend on any of the given mod names.
    /// More efficient than calling get_dependents() for each mod individually.
    pub fn get_dependents_batch(
        &self,
        mod_names: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<String>>, AppError> {
        use std::collections::HashMap;

        if mod_names.is_empty() {
            return Ok(HashMap::new());
        }

        // Build a map of mod_name -> list of dependents
        let mut result: HashMap<String, Vec<String>> = mod_names
            .iter()
            .map(|&n| (n.to_string(), Vec::new()))
            .collect();

        // Query all installed mods once
        let all_mods = self.get_installed_mods()?;

        for installed_mod in all_mods {
            for dep in &installed_mod.dependencies {
                if let Some(dependents) = result.get_mut(dep) {
                    dependents.push(installed_mod.name.clone());
                }
            }
        }

        Ok(result)
    }

    pub fn remove_installed_mod(&self, name: &str) -> Result<(), AppError> {
        self.conn
            .execute("DELETE FROM installed_mods WHERE name = ?1", [name])?;
        Ok(())
    }

    pub fn remove_installed_mod_by_name_or_path(
        &self,
        name: &str,
        path: &str,
    ) -> Result<(), AppError> {
        let removed = self.conn.execute(
            "DELETE FROM installed_mods WHERE lower(name) = lower(?1)",
            [name],
        )?;
        if removed == 0 && !path.trim().is_empty() {
            self.conn
                .execute("DELETE FROM installed_mods WHERE path = ?1", [path])?;
        }
        Ok(())
    }

    /// Reconcile the orphaned flag for every installed mod given the set of
    /// titles currently present in the remote index. Names not found in
    /// `present` get `orphaned = 1`; names found get `orphaned = 0`.
    /// Comparison is case-insensitive. Runs inside a single transaction.
    /// Returns the list of mod names whose orphaned state changed.
    pub fn reconcile_orphaned_mods(
        &mut self,
        present: &std::collections::HashSet<String>,
    ) -> Result<Vec<String>, AppError> {
        let lowered: std::collections::HashSet<String> =
            present.iter().map(|s| s.to_lowercase()).collect();

        let tx = self.conn.transaction()?;
        let mut changed: Vec<String> = Vec::new();
        {
            let mut stmt = tx.prepare("SELECT name, orphaned FROM installed_mods")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (name, prev) in rows {
                let should_be_orphan = !lowered.contains(&name.to_lowercase());
                if should_be_orphan != prev {
                    let flag: i64 = if should_be_orphan { 1 } else { 0 };
                    tx.execute(
                        "UPDATE installed_mods SET orphaned = ?1 WHERE name = ?2",
                        rusqlite::params![flag, name],
                    )?;
                    changed.push(name);
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn get_installation_path(&self) -> Result<Option<String>, AppError> {
        self.get_setting("installation_path")
    }

    pub fn set_installation_path(&self, path: &str) -> Result<(), AppError> {
        self.set_setting("installation_path", path)
    }

    pub fn remove_installation_path(&self) -> Result<(), AppError> {
        self.conn.execute(
            "DELETE FROM settings WHERE setting = 'installation_path'",
            [],
        )?;
        Ok(())
    }

    pub fn get_lovely_version(&self) -> Result<Option<String>, AppError> {
        self.get_setting("lovely_version")
    }

    pub fn set_lovely_version(&self, version: &str) -> Result<(), AppError> {
        self.set_setting("lovely_version", version)
    }

    pub fn get_last_installed_version(&self, mod_name: &str) -> Result<String, AppError> {
        // Use prepare_cached for frequently called queries
        let mut stmt = self
            .conn
            .prepare_cached("SELECT current_version FROM installed_mods WHERE name = ?1")?;
        let mut rows = stmt.query([mod_name])?;

        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(String::new())
        }
    }

    pub fn set_last_installed_version(
        &self,
        mod_name: &str,
        version: &str,
    ) -> Result<(), AppError> {
        self.conn.execute(
            "UPDATE installed_mods SET current_version = ?1 WHERE name = ?2",
            [version, mod_name],
        )?;
        Ok(())
    }

    pub fn set_background_enabled(&self, enabled: bool) -> Result<(), AppError> {
        let enabled: &str = if enabled { "enabled" } else { "disabled" };
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('background_enabled', ?1)",
            [enabled],
        )?;
        Ok(())
    }

    pub fn get_background_enabled(&self) -> Result<bool, AppError> {
        Ok(self.get_setting("background_enabled")?.as_deref() == Some("enabled"))
    }

    pub fn set_compat_helper_enabled(&self, enabled: bool) -> Result<(), AppError> {
        let value = if enabled { "enabled" } else { "disabled" };
        self.set_setting("compat_helper", value)
    }

    pub fn is_compat_helper_enabled(&self) -> Result<bool, AppError> {
        Ok(self.get_setting("compat_helper")?.as_deref() == Some("enabled"))
    }

    /// Check if the 0.3.7 compat helper migration has been applied.
    pub fn is_compat_helper_037_migrated(&self) -> Result<bool, AppError> {
        Ok(self.get_setting("compat_helper_037_migrated")?.is_some())
    }

    /// Mark the 0.3.7 compat helper migration as complete.
    pub fn set_compat_helper_037_migrated(&self) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('compat_helper_037_migrated', 'true')",
            [],
        )?;
        Ok(())
    }

    fn enable_lovely_console(&self) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('lovely_console', 'enabled')",
            [],
        )?;
        Ok(())
    }

    fn disable_lovely_console(&self) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('lovely_console', 'disabled')",
            [],
        )?;
        Ok(())
    }

    pub fn set_lovely_console_status(&self, enabled: bool) -> Result<(), AppError> {
        if enabled {
            self.enable_lovely_console()
        } else {
            self.disable_lovely_console()
        }
    }

    pub fn is_lovely_console_enabled(&self) -> Result<bool, AppError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE setting = 'lovely_console'")?;
        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            Ok(row.get::<_, String>(0)? == "enabled")
        } else {
            Ok(false)
        }
    }

    pub fn set_linux_prefix(&self, value: &str) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('linux_prefix', ?1)",
            [value],
        )?;
        Ok(())
    }

    pub fn get_linux_prefix(&self) -> Result<Option<String>, AppError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE setting = 'linux_prefix'")?;
        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn set_security_warning_acknowledged(&self, acknowledged: bool) -> Result<(), AppError> {
        let value = if acknowledged { "yes" } else { "no" };
        self.conn.execute(
        "INSERT OR REPLACE INTO settings (setting, value) VALUES ('security_warning_acknowledged', ?1)",
        [value],
    )?;
        Ok(())
    }

    pub fn is_security_warning_acknowledged(&self) -> Result<bool, AppError> {
        let mut stmt = self.conn.prepare(
            "SELECT value FROM settings WHERE setting = 'security_warning_acknowledged'",
        )?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(row.get::<_, String>(0)? == "yes")
        } else {
            // Default to not acknowledged
            Ok(false)
        }
    }

    /// Set the launch mode preference ("modded" or "vanilla")
    pub fn set_launch_mode(&self, mode: &str) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (setting, value) VALUES ('launch_mode', ?1)",
            [mode],
        )?;
        Ok(())
    }

    /// Get the launch mode preference. Defaults to "modded" if not set.
    pub fn get_launch_mode(&self) -> Result<String, AppError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE setting = 'launch_mode'")?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(row.get::<_, String>(0)?)
        } else {
            Ok("modded".to_string())
        }
    }
}

fn resolve_storage_path(primary_balatro_dir: &Path) -> PathBuf {
    let primary = primary_balatro_dir.join("bmm_storage.db");

    // Detect database created by the Flatpak build (config dir lives under ~/.var/app/io.balatro.ModManager)
    let flatpak_db = dirs::home_dir()
        .map(|home| home.join(".var/app/io.balatro.ModManager/config/Balatro/bmm_storage.db"));

    if primary.exists() {
        return primary;
    }

    if let Some(fp_path) = flatpak_db
        && fp_path.exists()
    {
        log::info!(
            "Using existing database from Flatpak install at {}",
            fp_path.display()
        );
        return fp_path;
    }

    primary
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn create_memory_db() -> Result<Database, AppError> {
        let conn =
            Connection::open_in_memory().map_err(|e| AppError::DatabaseInit(e.to_string()))?;
        Database::initialize_database(&conn)?;
        Ok(Database { conn })
    }

    #[test]

    fn test_installed_mods_crud() -> Result<(), AppError> {
        let db = create_memory_db()?;

        // Add with empty dependencies
        db.add_installed_mod("TestMod", "/path/to/mod", &[], None)?;
        let mods = db.get_installed_mods()?;
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "TestMod");
        assert!(mods[0].dependencies.is_empty()); // Verify dependencies

        // Add with dependencies
        let deps = vec!["Steamodded".to_string()];
        db.add_installed_mod("DependentMod", "/another/path", &deps, None)?;
        let mods = db.get_installed_mods()?;
        assert_eq!(mods[1].dependencies, deps);

        db.remove_installed_mod("TestMod")?;
        assert_eq!(db.get_installed_mods()?.len(), 1);

        Ok(())
    }

    #[test]
    fn test_installation_path_management() -> Result<(), AppError> {
        let db = create_memory_db()?;

        assert!(db.get_installation_path()?.is_none());
        db.set_installation_path("/games/balatro")?;
        assert_eq!(db.get_installation_path()?, Some("/games/balatro".into()));

        db.remove_installation_path()?;
        assert!(db.get_installation_path()?.is_none());

        Ok(())
    }

    #[test]
    fn test_mod_details() -> Result<(), AppError> {
        let db = create_memory_db()?;
        let deps = vec!["Steamodded".to_string()];

        db.add_installed_mod("TestMod", "/path/to/mod", &deps, None)?;

        let details = db.get_mod_details("TestMod")?;
        assert_eq!(details.name, "TestMod");
        assert_eq!(details.path, "/path/to/mod");
        assert_eq!(details.dependencies, deps);

        Ok(())
    }

    #[test]
    fn test_get_mod_details_not_found() -> Result<(), AppError> {
        let db = create_memory_db()?;
        let result = db.get_mod_details("NonExistentMod");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_remove_nonexistent_mod_succeeds() -> Result<(), AppError> {
        let db = create_memory_db()?;
        // Removing a mod that doesn't exist should not error
        let result = db.remove_installed_mod("NonExistentMod");
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_duplicate_mod_replaces() -> Result<(), AppError> {
        let db = create_memory_db()?;
        db.add_installed_mod("TestMod", "/path/v1", &[], Some("1.0".to_string()))?;
        db.add_installed_mod("TestMod", "/path/v2", &[], Some("2.0".to_string()))?;

        let mods = db.get_installed_mods()?;
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].path, "/path/v2");
        Ok(())
    }

    #[test]
    fn test_get_dependents_batch() -> Result<(), AppError> {
        let db = create_memory_db()?;

        db.add_installed_mod("Steamodded", "/path/steamodded", &[], None)?;
        db.add_installed_mod("ModA", "/path/moda", &["Steamodded".to_string()], None)?;
        db.add_installed_mod(
            "ModB",
            "/path/modb",
            &["Steamodded".to_string(), "Talisman".to_string()],
            None,
        )?;
        db.add_installed_mod("Talisman", "/path/talisman", &[], None)?;

        let deps = db.get_dependents_batch(&["Steamodded", "Talisman"])?;

        assert_eq!(deps.get("Steamodded").unwrap().len(), 2); // ModA and ModB
        assert_eq!(deps.get("Talisman").unwrap().len(), 1); // ModB

        Ok(())
    }

    #[test]
    fn test_reconcile_orphaned_mods() -> Result<(), AppError> {
        use std::collections::HashSet;
        let mut db = create_memory_db()?;

        db.add_installed_mod("Alpha", "/p/alpha", &[], None)?;
        db.add_installed_mod("Beta", "/p/beta", &[], None)?;
        db.add_installed_mod("Gamma", "/p/gamma", &[], None)?;

        // Initially no orphans.
        for m in db.get_installed_mods()? {
            assert!(!m.orphaned, "{} should not be orphaned", m.name);
        }

        // Remote has Alpha + Gamma only -> Beta becomes orphan.
        let mut present: HashSet<String> = HashSet::new();
        present.insert("Alpha".to_string());
        present.insert("Gamma".to_string());
        let mut changed = db.reconcile_orphaned_mods(&present)?;
        changed.sort();
        assert_eq!(changed, vec!["Beta".to_string()]);

        let mods = db.get_installed_mods()?;
        for m in &mods {
            if m.name == "Beta" {
                assert!(m.orphaned, "Beta must be orphan");
            } else {
                assert!(!m.orphaned, "{} must not be orphan", m.name);
            }
        }

        // Re-running with same set should produce no changes.
        let changed = db.reconcile_orphaned_mods(&present)?;
        assert!(changed.is_empty());

        // Beta reappears in remote -> orphan cleared.
        present.insert("beta".to_string()); // case-insensitive
        let changed = db.reconcile_orphaned_mods(&present)?;
        assert_eq!(changed, vec!["Beta".to_string()]);
        let beta = db.get_mod_details("Beta")?;
        assert!(!beta.orphaned);

        Ok(())
    }

    #[test]
    fn test_add_installed_mod_clears_orphan_flag() -> Result<(), AppError> {
        use std::collections::HashSet;
        let mut db = create_memory_db()?;
        db.add_installed_mod("Foo", "/p/foo", &[], None)?;
        // Mark orphan via reconcile with empty remote set
        let empty: HashSet<String> = HashSet::new();
        db.reconcile_orphaned_mods(&empty)?;
        assert!(db.get_mod_details("Foo")?.orphaned);
        // Reinstalling should reset orphaned to default (0) because INSERT OR
        // REPLACE writes a fresh row.
        db.add_installed_mod("Foo", "/p/foo", &[], None)?;
        assert!(!db.get_mod_details("Foo")?.orphaned);
        Ok(())
    }
}
