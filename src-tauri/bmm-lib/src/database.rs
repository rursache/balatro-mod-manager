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

/// Source of mods installed from the GitHub mod index (and of every row
/// created before sources were tracked)
pub const SOURCE_INDEX: &str = "index";
/// Source of mods installed from Thunderstore, `source_id` is `Namespace-Name`
pub const SOURCE_THUNDERSTORE: &str = "thunderstore";

/// Represents an installed mod stored in the database.
#[derive(Serialize, Debug, Clone)]
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
    /// Where the mod was installed from, see [`SOURCE_INDEX`] and [`SOURCE_THUNDERSTORE`]
    pub source: String,
    /// Stable package identity within `source`, `Namespace-Name` for Thunderstore
    pub source_id: Option<String>,
    /// Unix seconds of the last install or update, None for rows that predate it
    pub installed_at: Option<i64>,
}

const INSTALLED_MOD_COLUMNS: &str =
    "name, path, dependencies, current_version, orphaned, source, source_id, installed_at";

impl InstalledMod {
    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, AppError> {
        Ok(InstalledMod {
            name: row.get(0)?,
            path: row.get(1)?,
            dependencies: serde_json::from_str(&row.get::<_, String>(2)?)?,
            current_version: row.get(3)?,
            orphaned: row.get::<_, i64>(4)? != 0,
            source: row.get(5)?,
            source_id: row.get(6)?,
            installed_at: row.get(7)?,
        })
    }

    /// The Thunderstore namespace when this row was installed from Thunderstore
    pub fn thunderstore_namespace(&self) -> Option<&str> {
        if self.source != SOURCE_THUNDERSTORE {
            return None;
        }
        self.source_id
            .as_deref()
            .and_then(|id| id.split_once('-'))
            .map(|(ns, _)| ns)
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Name for a row whose wanted `name` is already used by another package:
/// `Name (Namespace)`, then `Name (Namespace) 2` and so on
fn disambiguated_name(name: &str, source_id: &str, taken: impl Fn(&str) -> bool) -> String {
    let qualifier = source_id
        .split_once('-')
        .map(|(ns, _)| ns)
        .filter(|ns| !ns.is_empty())
        .unwrap_or(source_id);
    let base = format!("{name} ({qualifier})");
    if !taken(&base) {
        return base;
    }
    (2u32..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or(base)
}

impl Database {
    const CURRENT_DB_VERSION: (u32, u32) = (1, 4); // Update this when schema changes

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
        Self::open_at(&storage_path)
    }

    /// Opens or creates the database file at `storage_path`, migrating it to
    /// the current schema when needed
    pub fn open_at(storage_path: &Path) -> Result<Self, AppError> {
        let storage_path = storage_path.to_path_buf();

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

        // Columns added by later schema versions are copied when the old table has them
        let old_columns = Self::table_columns(old_conn, "installed_mods");
        let optional = |column: &str, fallback: &str| {
            if old_columns.iter().any(|c| c == column) {
                column.to_string()
            } else {
                fallback.to_string()
            }
        };
        let query = format!(
            "SELECT name, path, dependencies, current_version, {}, {}, {}, {} FROM installed_mods",
            optional("orphaned", "0"),
            optional("source", "NULL"),
            optional("source_id", "NULL"),
            optional("installed_at", "NULL"),
        );

        let mut stmt = match old_conn.prepare(&query) {
            Ok(stmt) => stmt,
            Err(_) => return Ok(()), // If query fails, just continue
        };

        for (name, path, dependencies, current_version, orphaned, source, source_id, installed_at) in
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                ))
            })?
            .flatten()
        {
            new_conn.execute(
                "INSERT INTO installed_mods (name, path, dependencies, current_version, orphaned, source, source_id, installed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    name,
                    path,
                    dependencies,
                    current_version.unwrap_or_default(),
                    orphaned.unwrap_or(0),
                    source.unwrap_or_else(|| SOURCE_INDEX.to_string()),
                    source_id,
                    installed_at,
                ],
            )?;
        }

        Ok(())
    }

    fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
        let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
            return Vec::new();
        };
        stmt.query_map([], |row| row.get::<_, String>(1))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
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
                orphaned INTEGER NOT NULL DEFAULT 0,
                source TEXT NOT NULL DEFAULT 'index',
                source_id TEXT,
                installed_at INTEGER
            )",
            [],
        )
        .map_err(|e| AppError::DatabaseInit(e.to_string()))?;

        // One row per package, rows without a source_id are not constrained
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_installed_mods_source_id
             ON installed_mods(source, source_id) WHERE source_id IS NOT NULL",
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
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {INSTALLED_MOD_COLUMNS} FROM installed_mods WHERE name = ?1"
        ))?;

        let mut rows = stmt.query([mod_name])?;

        if let Some(row) = rows.next()? {
            InstalledMod::from_row(row)
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
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {INSTALLED_MOD_COLUMNS} FROM installed_mods"
        ))?;
        let mut mods = Vec::new();
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            mods.push(InstalledMod::from_row(row)?);
        }

        Ok(mods)
    }

    /// Records an index install of `name`, replacing any row with that name,
    /// so the row becomes `source = 'index'` without a source_id
    pub fn add_installed_mod(
        &self,
        name: &str,
        path: &str,
        dependencies: &[String],
        current_version: Option<String>,
    ) -> Result<(), AppError> {
        let deps_json = serde_json::to_string(dependencies)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO installed_mods (name, path, dependencies, current_version, source, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                name,
                path,
                deps_json,
                current_version.unwrap_or_default(),
                SOURCE_INDEX,
                unix_now(),
            ],
        )?;
        Ok(())
    }

    /// Records an install from `source`, returning the row name actually used
    ///
    /// - with a `source_id` that is already tracked, that row is updated in
    ///   place and keeps its name, so dependents and the UI stay stable
    /// - when `name` is taken by a row of a different package, the new row is
    ///   named `Name (Namespace)`
    /// - without a `source_id` this behaves like [`Self::add_installed_mod`]
    ///   for the given source
    ///
    /// Switching an existing row to another source goes through
    /// [`Self::switch_installed_mod_source`] instead
    pub fn add_installed_mod_with_source(
        &self,
        name: &str,
        path: &str,
        dependencies: &[String],
        current_version: Option<String>,
        source: &str,
        source_id: Option<&str>,
    ) -> Result<String, AppError> {
        let deps_json = serde_json::to_string(dependencies)?;
        let version = current_version.unwrap_or_default();
        let now = unix_now();

        let Some(source_id) = source_id.filter(|id| !id.trim().is_empty()) else {
            self.conn.execute(
                "INSERT OR REPLACE INTO installed_mods (name, path, dependencies, current_version, source, installed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![name, path, deps_json, version, source, now],
            )?;
            return Ok(name.to_string());
        };

        if let Some(existing) = self.find_by_source(source, source_id)? {
            self.conn.execute(
                "UPDATE installed_mods
                 SET path = ?1, dependencies = ?2, current_version = ?3, orphaned = 0, installed_at = ?4
                 WHERE name = ?5",
                rusqlite::params![path, deps_json, version, now, existing.name],
            )?;
            return Ok(existing.name);
        }

        let taken = |candidate: &str| -> bool {
            self.conn
                .query_row(
                    "SELECT 1 FROM installed_mods WHERE lower(name) = lower(?1)",
                    [candidate],
                    |_| Ok(()),
                )
                .is_ok()
        };
        let final_name = if taken(name) {
            disambiguated_name(name, source_id, taken)
        } else {
            name.to_string()
        };

        self.conn.execute(
            "INSERT INTO installed_mods (name, path, dependencies, current_version, orphaned, source, source_id, installed_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7)",
            rusqlite::params![final_name, path, deps_json, version, source, source_id, now],
        )?;
        Ok(final_name)
    }

    /// Moves the row `name` to another source, for source switches and the
    /// legacy Multiplayer migration. The row keeps its name and loses its
    /// orphaned flag. Fails when another row already tracks the target package
    pub fn switch_installed_mod_source(
        &self,
        name: &str,
        path: &str,
        dependencies: &[String],
        current_version: Option<String>,
        source: &str,
        source_id: Option<&str>,
    ) -> Result<(), AppError> {
        let source_id = source_id.filter(|id| !id.trim().is_empty());
        if let Some(id) = source_id
            && let Some(other) = self.find_by_source(source, id)?
            && other.name != name
        {
            return Err(AppError::InvalidState(format!(
                "{id} is already tracked as {}",
                other.name
            )));
        }
        let deps_json = serde_json::to_string(dependencies)?;
        let updated = self.conn.execute(
            "UPDATE installed_mods
             SET path = ?1, dependencies = ?2, current_version = ?3, orphaned = 0,
                 source = ?4, source_id = ?5, installed_at = ?6
             WHERE name = ?7",
            rusqlite::params![
                path,
                deps_json,
                current_version.unwrap_or_default(),
                source,
                source_id,
                unix_now(),
                name,
            ],
        )?;
        if updated == 0 {
            return Err(AppError::InvalidState(format!("Mod {name} not found")));
        }
        Ok(())
    }

    /// The row tracking `source_id` from any source, if installed
    pub fn get_installed_by_source_id(
        &self,
        source_id: &str,
    ) -> Result<Option<InstalledMod>, AppError> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {INSTALLED_MOD_COLUMNS} FROM installed_mods
             WHERE lower(source_id) = lower(?1) ORDER BY installed_at DESC LIMIT 1"
        ))?;
        let mut rows = stmt.query([source_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(InstalledMod::from_row(row)?)),
            None => Ok(None),
        }
    }

    fn find_by_source(
        &self,
        source: &str,
        source_id: &str,
    ) -> Result<Option<InstalledMod>, AppError> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {INSTALLED_MOD_COLUMNS} FROM installed_mods
             WHERE source = ?1 AND lower(source_id) = lower(?2) LIMIT 1"
        ))?;
        let mut rows = stmt.query([source, source_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(InstalledMod::from_row(row)?)),
            None => Ok(None),
        }
    }

    /// Writes `record` as is, replacing any row with the same name or package,
    /// used to restore backups
    pub fn put_installed_mod(&self, record: &InstalledMod) -> Result<(), AppError> {
        let deps_json = serde_json::to_string(&record.dependencies)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO installed_mods (name, path, dependencies, current_version, orphaned, source, source_id, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                record.name,
                record.path,
                deps_json,
                record.current_version.clone().unwrap_or_default(),
                record.orphaned as i64,
                record.source,
                record.source_id,
                record.installed_at,
            ],
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

    /// Reconcile the orphaned flag for every index installed mod given the set
    /// of titles currently present in the remote index. Names not found in
    /// `present` get `orphaned = 1`; names found get `orphaned = 0`.
    /// Comparison is case-insensitive. Runs inside a single transaction.
    /// Rows from other sources are left untouched, see
    /// [`Self::reconcile_orphaned_mods_by_source`].
    /// Returns the list of mod names whose orphaned state changed.
    pub fn reconcile_orphaned_mods(
        &mut self,
        present: &std::collections::HashSet<String>,
    ) -> Result<Vec<String>, AppError> {
        self.reconcile_orphans(present, None)
    }

    /// Like [`Self::reconcile_orphaned_mods`], and Thunderstore rows are
    /// orphaned only when their source_id (`Namespace-Name`) is missing from
    /// `present_thunderstore`. Thunderstore rows without a source_id keep their flag
    pub fn reconcile_orphaned_mods_by_source(
        &mut self,
        present_index_titles: &std::collections::HashSet<String>,
        present_thunderstore: &std::collections::HashSet<String>,
    ) -> Result<Vec<String>, AppError> {
        self.reconcile_orphans(present_index_titles, Some(present_thunderstore))
    }

    fn reconcile_orphans(
        &mut self,
        present: &std::collections::HashSet<String>,
        present_thunderstore: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<String>, AppError> {
        let lowered: std::collections::HashSet<String> =
            present.iter().map(|s| s.to_lowercase()).collect();
        let lowered_ts: Option<std::collections::HashSet<String>> =
            present_thunderstore.map(|ids| ids.iter().map(|s| s.to_lowercase()).collect());

        let tx = self.conn.transaction()?;
        let mut changed: Vec<String> = Vec::new();
        {
            let mut stmt =
                tx.prepare("SELECT name, orphaned, source, source_id FROM installed_mods")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? != 0,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (name, prev, source, source_id) in rows {
                let should_be_orphan = match source.as_str() {
                    SOURCE_INDEX => !lowered.contains(&name.to_lowercase()),
                    SOURCE_THUNDERSTORE => match (&lowered_ts, source_id) {
                        (Some(ids), Some(id)) => !ids.contains(&id.to_lowercase()),
                        _ => continue,
                    },
                    _ => continue,
                };
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

    // Schema and data as written by app versions using db_version 1.3
    fn write_v1_3_database(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (setting TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE installed_mods (
                 name TEXT PRIMARY KEY,
                 path TEXT NOT NULL,
                 dependencies TEXT NOT NULL DEFAULT '[]',
                 current_version TEXT,
                 orphaned INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX idx_installed_mods_path ON installed_mods(path);
             CREATE INDEX idx_installed_mods_name ON installed_mods(name);
             INSERT INTO settings VALUES ('db_version', '1.3');
             INSERT INTO settings VALUES ('installation_path', '/games/balatro');
             INSERT INTO settings VALUES ('lovely_version', '0.9.0');
             INSERT INTO settings VALUES ('security_warning_acknowledged', 'yes');
             INSERT INTO installed_mods VALUES ('Steamodded', '/mods/Steamodded', '[]', '26.1002.0', 0);
             INSERT INTO installed_mods VALUES ('Multiplayer', '/mods/Multiplayer', '[\"Steamodded\"]', 'v0.5.4', 1);",
        )
        .unwrap();
    }

    #[test]
    fn migrates_v1_3_database_to_sources() -> Result<(), AppError> {
        let td = tempfile::tempdir().unwrap();
        let db_path = td.path().join("bmm_storage.db");
        write_v1_3_database(&db_path);

        let db = Database::open_at(&db_path)?;

        let version: String = db.conn.query_row(
            "SELECT value FROM settings WHERE setting = 'db_version'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(version, "1.4");
        assert_eq!(db.get_installation_path()?, Some("/games/balatro".into()));
        assert_eq!(db.get_lovely_version()?, Some("0.9.0".into()));
        assert!(db.is_security_warning_acknowledged()?);
        assert!(td.path().join("bmm_storage.db.bak").exists());

        let mut mods = db.get_installed_mods()?;
        mods.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(mods.len(), 2);
        for m in &mods {
            assert_eq!(m.source, SOURCE_INDEX);
            assert!(m.source_id.is_none());
            assert!(m.installed_at.is_none());
        }
        assert_eq!(mods[0].name, "Multiplayer");
        assert!(mods[0].orphaned, "orphaned flag must survive the migration");
        assert_eq!(mods[0].current_version.as_deref(), Some("v0.5.4"));
        assert_eq!(mods[0].dependencies, vec!["Steamodded".to_string()]);
        assert!(!mods[1].orphaned);

        // Reopening a current database does not migrate again
        drop(db);
        std::fs::remove_file(td.path().join("bmm_storage.db.bak")).unwrap();
        let db = Database::open_at(&db_path)?;
        assert_eq!(db.get_installed_mods()?.len(), 2);
        assert!(!td.path().join("bmm_storage.db.bak").exists());
        Ok(())
    }

    #[test]
    fn new_rows_record_source_and_install_time() -> Result<(), AppError> {
        let db = create_memory_db()?;
        db.add_installed_mod("IndexMod", "/p/index", &[], Some("1.0".into()))?;
        let name = db.add_installed_mod_with_source(
            "Multiplayer",
            "/p/BalatroMultiplayer-Multiplayer",
            &["Steamodded".into()],
            Some("0.5.7".into()),
            SOURCE_THUNDERSTORE,
            Some("BalatroMultiplayer-Multiplayer"),
        )?;
        assert_eq!(name, "Multiplayer");

        let index = db.get_mod_details("IndexMod")?;
        assert_eq!(index.source, SOURCE_INDEX);
        assert!(index.installed_at.is_some());

        let ts = db
            .get_installed_by_source_id("BalatroMultiplayer-Multiplayer")?
            .expect("tracked by source id");
        assert_eq!(ts.name, "Multiplayer");
        assert_eq!(ts.source, SOURCE_THUNDERSTORE);
        assert_eq!(ts.thunderstore_namespace(), Some("BalatroMultiplayer"));
        assert_eq!(ts.current_version.as_deref(), Some("0.5.7"));
        assert!(db.get_installed_by_source_id("Nobody-Nothing")?.is_none());
        Ok(())
    }

    #[test]
    fn same_source_id_updates_row_in_place() -> Result<(), AppError> {
        let db = create_memory_db()?;
        db.add_installed_mod_with_source(
            "Pokermon",
            "/p/v1",
            &[],
            Some("1.0.0".into()),
            SOURCE_THUNDERSTORE,
            Some("InertSteak-Pokermon"),
        )?;
        let name = db.add_installed_mod_with_source(
            "Pokermon Renamed",
            "/p/v2",
            &[],
            Some("1.1.0".into()),
            SOURCE_THUNDERSTORE,
            Some("InertSteak-Pokermon"),
        )?;
        assert_eq!(name, "Pokermon");
        let mods = db.get_installed_mods()?;
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].path, "/p/v2");
        assert_eq!(mods[0].current_version.as_deref(), Some("1.1.0"));
        Ok(())
    }

    #[test]
    fn name_taken_by_another_package_is_disambiguated() -> Result<(), AppError> {
        let db = create_memory_db()?;
        db.add_installed_mod("Pokermon", "/p/index-pokermon", &[], None)?;

        let first = db.add_installed_mod_with_source(
            "Pokermon",
            "/p/PokermonPlus-Pokermon",
            &[],
            None,
            SOURCE_THUNDERSTORE,
            Some("PokermonPlus-Pokermon"),
        )?;
        assert_eq!(first, "Pokermon (PokermonPlus)");

        let second = db.add_installed_mod_with_source(
            "pokermon",
            "/p/Other-Pokermon",
            &[],
            None,
            SOURCE_THUNDERSTORE,
            Some("PokermonPlus-pokermon_fork"),
        )?;
        assert_eq!(second, "pokermon (PokermonPlus) 2");

        // The original index row is untouched
        let index = db.get_mod_details("Pokermon")?;
        assert_eq!(index.path, "/p/index-pokermon");
        assert_eq!(index.source, SOURCE_INDEX);
        assert_eq!(db.get_installed_mods()?.len(), 3);
        Ok(())
    }

    #[test]
    fn switch_source_keeps_name_and_clears_orphan() -> Result<(), AppError> {
        let mut db = create_memory_db()?;
        db.add_installed_mod("Multiplayer", "/p/Multiplayer", &[], Some("v0.5.4".into()))?;
        db.reconcile_orphaned_mods(&std::collections::HashSet::new())?;
        assert!(db.get_mod_details("Multiplayer")?.orphaned);

        db.switch_installed_mod_source(
            "Multiplayer",
            "/p/BalatroMultiplayer-Multiplayer",
            &["Steamodded".into()],
            Some("0.5.7".into()),
            SOURCE_THUNDERSTORE,
            Some("BalatroMultiplayer-Multiplayer"),
        )?;
        let row = db.get_mod_details("Multiplayer")?;
        assert!(!row.orphaned);
        assert_eq!(row.source, SOURCE_THUNDERSTORE);
        assert_eq!(
            row.source_id.as_deref(),
            Some("BalatroMultiplayer-Multiplayer")
        );
        assert_eq!(row.path, "/p/BalatroMultiplayer-Multiplayer");

        // A second row cannot claim the same package
        db.add_installed_mod("Other", "/p/other", &[], None)?;
        assert!(
            db.switch_installed_mod_source(
                "Other",
                "/p/other",
                &[],
                None,
                SOURCE_THUNDERSTORE,
                Some("BalatroMultiplayer-Multiplayer"),
            )
            .is_err()
        );
        assert!(
            db.switch_installed_mod_source("Missing", "/p", &[], None, SOURCE_INDEX, None)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn reconcile_is_source_aware() -> Result<(), AppError> {
        use std::collections::HashSet;
        let mut db = create_memory_db()?;
        db.add_installed_mod("IndexMod", "/p/index", &[], None)?;
        db.add_installed_mod_with_source(
            "TsMod",
            "/p/ts",
            &[],
            None,
            SOURCE_THUNDERSTORE,
            Some("Ns-TsMod"),
        )?;

        // The index only reconcile never orphans Thunderstore rows
        db.reconcile_orphaned_mods(&HashSet::new())?;
        assert!(db.get_mod_details("IndexMod")?.orphaned);
        assert!(!db.get_mod_details("TsMod")?.orphaned);

        let titles: HashSet<String> = ["IndexMod".to_string()].into();
        let mut ts_ids: HashSet<String> = HashSet::new();
        let changed = db.reconcile_orphaned_mods_by_source(&titles, &ts_ids)?;
        assert_eq!(changed.len(), 2);
        assert!(!db.get_mod_details("IndexMod")?.orphaned);
        assert!(db.get_mod_details("TsMod")?.orphaned);

        ts_ids.insert("ns-tsmod".to_string());
        let changed = db.reconcile_orphaned_mods_by_source(&titles, &ts_ids)?;
        assert_eq!(changed, vec!["TsMod".to_string()]);
        assert!(!db.get_mod_details("TsMod")?.orphaned);
        Ok(())
    }

    #[test]
    fn put_installed_mod_roundtrips_all_fields() -> Result<(), AppError> {
        let db = create_memory_db()?;
        let record = InstalledMod {
            name: "TsMod".into(),
            path: "/p/ts".into(),
            dependencies: vec!["Steamodded".into()],
            current_version: Some("1.2.3".into()),
            orphaned: true,
            source: SOURCE_THUNDERSTORE.into(),
            source_id: Some("Ns-TsMod".into()),
            installed_at: Some(42),
        };
        db.put_installed_mod(&record)?;
        let row = db.get_mod_details("TsMod")?;
        assert!(row.orphaned);
        assert_eq!(row.source_id.as_deref(), Some("Ns-TsMod"));
        assert_eq!(row.installed_at, Some(42));
        Ok(())
    }
}
