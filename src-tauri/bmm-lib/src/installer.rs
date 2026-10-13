//! Mod installation and uninstallation logic.
//!
//! This module handles downloading mods from URLs and extracting them into the
//! Balatro Mods directory. It supports multiple archive formats:
//! - ZIP archives (most common)
//! - Gzipped tarballs (.tar.gz)
//! - RAR archives
//!
//! # Installation Process
//!
//! 1. Download the mod archive from the provided URL
//! 2. Detect the archive format (ZIP, tar.gz, or RAR)
//! 3. Extract contents to the Mods directory
//! 4. Handle nested folder structures (single-folder archives)
//!
//! # Security
//!
//! Path traversal attacks are prevented by validating extracted paths stay
//! within the target directory.

use crate::errors::AppError;
use crate::http::shared_client;
use crate::local_mod_detection::{
    ensure_proton_mod_dir_link, mod_dir_candidates, resolve_mods_dir_path,
};
use flate2::read::GzDecoder;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use std::fs::{self, File};
use std::io::Read;
use std::io::{self, BufReader, Write};
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;
use tar::Archive;
use tokio::io::AsyncWriteExt;
use tokio::time::sleep;
use zip::ZipArchive;

/// Atomic counter to generate unique temp file names for concurrent downloads.
static DOWNLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Maximum total decompressed size allowed (2 GB)
const MAX_DECOMPRESSED_SIZE: u64 = 2 * 1024 * 1024 * 1024;

/// Maximum number of files allowed in an archive
const MAX_ARCHIVE_FILES: usize = 10_000;

/// Delay after uninstalling a mod before reinstalling (Windows).
/// Windows file handles take longer to fully release.
#[cfg(target_os = "windows")]
const FILE_LOCK_RELEASE_DELAY_MS: u64 = 250;

/// Delay after uninstalling a mod before reinstalling (Unix).
#[cfg(not(target_os = "windows"))]
const FILE_LOCK_RELEASE_DELAY_MS: u64 = 100;

/// Base retry delay for directory removal (Windows).
#[cfg(target_os = "windows")]
const DIR_REMOVE_BASE_DELAY_MS: u64 = 100;

/// Base retry delay for directory removal (Unix).
#[cfg(not(target_os = "windows"))]
const DIR_REMOVE_BASE_DELAY_MS: u64 = 50;

/// Downloads and installs a mod from the given URL.
///
/// The archive is automatically detected and extracted to the Balatro Mods
/// directory. If `folder_name` is provided, it overrides the archive's internal
/// folder name.
///
/// # Arguments
///
/// * `url` - The download URL for the mod archive
/// * `folder_name` - Optional custom folder name for the installed mod
///
/// # Returns
///
/// The path to the installed mod directory on success.
///
/// # Errors
///
/// Returns an error if the download fails, the archive format is unsupported,
/// or extraction fails.
pub async fn install_mod(url: String, folder_name: Option<String>) -> Result<PathBuf, AppError> {
    log::info!("Starting mod download from: {}", url);
    let client = shared_client();
    let response = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            log::error!("Request timed out for URL: {}", url);
        } else if e.is_connect() {
            log::error!("Connection failed for URL: {} - {}", url, e);
        } else if e.is_request() {
            log::error!("Request error for URL: {} - {}", url, e);
        } else {
            log::error!("Network error for URL: {} - {}", url, e);
        }
        AppError::NetworkRequest {
            url: url.clone(),
            source: e.to_string(),
        }
    })?;

    // Capture headers for fallback detection
    let content_type_header = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let content_disposition_filename = response
        .headers()
        .get(CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_disposition_filename);

    // Check status before reading body
    if !response.status().is_success() {
        log::error!("HTTP error {} for URL: {}", response.status(), url);
        return Err(AppError::NetworkRequest {
            url: url.clone(),
            source: format!("Download URL not reachable (HTTP {})", response.status()),
        });
    }

    log::debug!(
        "Response received: status={}, content-type={:?}, content-length={:?}",
        response.status(),
        content_type_header,
        response.content_length()
    );

    // Stream download to a temporary file to avoid loading entire archive into memory
    // This reduces peak memory usage from "archive size" to ~64KB chunk size
    // Use atomic counter to ensure unique temp file per concurrent download
    let temp_dir = std::env::temp_dir();
    let unique_id = DOWNLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_file_path = temp_dir.join(format!(
        "bmm_download_{}_{}.tmp",
        std::process::id(),
        unique_id
    ));

    let mut temp_file = tokio::fs::File::create(&temp_file_path)
        .await
        .map_err(|e| {
            log::error!("Failed to create temp file: {}", e);
            AppError::FileWrite {
                path: temp_file_path.clone(),
                source: e.to_string(),
            }
        })?;

    let mut total_bytes: u64 = 0;
    let mut magic_bytes = Vec::with_capacity(512);

    // Use response.chunk() for streaming without needing futures-util
    let mut response = response;
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        log::error!("Failed to read response chunk from {}: {}", url, e);
        AppError::NetworkRequest {
            url: url.clone(),
            source: e.to_string(),
        }
    })? {
        // Capture first 512 bytes for magic detection
        if magic_bytes.len() < 512 {
            let needed = 512 - magic_bytes.len();
            let take = needed.min(chunk.len());
            magic_bytes.extend_from_slice(&chunk[..take]);
        }

        temp_file.write_all(&chunk).await.map_err(|e| {
            log::error!("Failed to write to temp file: {}", e);
            AppError::FileWrite {
                path: temp_file_path.clone(),
                source: e.to_string(),
            }
        })?;
        total_bytes += chunk.len() as u64;
    }

    temp_file.flush().await.map_err(|e| AppError::FileWrite {
        path: temp_file_path.clone(),
        source: e.to_string(),
    })?;
    drop(temp_file); // Close the file handle

    log::debug!(
        "Downloaded {} bytes from {} (streamed to temp file)",
        total_bytes,
        url
    );

    // Detect archive type from magic bytes
    let magic_bytes = bytes::Bytes::from(magic_bytes);
    let archive_kind = guess_archive_kind(
        &magic_bytes,
        &url,
        content_type_header.as_deref(),
        content_disposition_filename.as_deref(),
    )
    .ok_or_else(|| {
        // Clean up temp file on error
        let _ = std::fs::remove_file(&temp_file_path);
        AppError::InvalidState(
            "Unsupported or unknown archive type (supported: .zip, .tar, .tar.gz)".into(),
        )
    })?;

    let mod_dir = resolve_mods_dir()?;

    let fallback_name = || {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("mod_{timestamp}")
    };

    let mut mod_name = {
        if let Some(name) = folder_name.filter(|n| !n.is_empty()) {
            // Use provided folder name if it exists and isn't empty
            name
        } else {
            // Extract from URL as fallback
            let url_name = url
                .split('/')
                .next_back()
                .and_then(|s| s.split('.').next())
                .unwrap_or("unknown_mod");

            // If the extracted name is too generic (like "main" or "master")
            if url_name == "main" || url_name == "master" || url_name.len() <= 2 {
                // Generate a more unique name with a timestamp
                fallback_name()
            } else {
                url_name.to_string()
            }
        }
    };
    mod_name = sanitize_mod_name(&mod_name);
    if mod_name.is_empty() {
        mod_name = fallback_name();
    }

    // Uninstall old mod folder if it exists
    let target_dir = mod_dir.join(&mod_name);
    let was_disabled = tokio::fs::try_exists(target_dir.join(".lovelyignore"))
        .await
        .unwrap_or(false);
    if tokio::fs::try_exists(&target_dir).await.unwrap_or(false) {
        log::info!("Uninstalling existing mod at: {target_dir:?}");
        uninstall_mod(target_dir.clone()).await?;

        // Give the filesystem time to fully release file handles.
        // This is especially important on Windows where file locks can persist briefly.
        sleep(Duration::from_millis(FILE_LOCK_RELEASE_DELAY_MS)).await;
    }

    log::info!("Installing mod: {url}");

    // Run CPU-bound archive extraction in a blocking task
    // Clone these values for the blocking task - they're used after spawn_blocking returns
    // (for was_disabled logic and error reporting), so we can't move them into the closure.
    let mod_dir_clone = mod_dir.clone();
    let mod_name_clone = mod_name.clone();
    let temp_path_clone = temp_file_path.clone();
    let installed_path = tokio::task::spawn_blocking(move || {
        let result = match archive_kind {
            ArchiveKind::Zip => handle_zip(&temp_path_clone, &mod_dir_clone, &mod_name_clone),
            ArchiveKind::Tar => handle_tar(&temp_path_clone, &mod_dir_clone, &mod_name_clone),
            ArchiveKind::TarGz => handle_tar_gz(&temp_path_clone, &mod_dir_clone, &mod_name_clone),
        };
        // Clean up temp file after extraction (regardless of success/failure)
        let _ = std::fs::remove_file(&temp_path_clone);
        result
    })
    .await
    .map_err(|e| {
        // Clean up temp file if spawn_blocking fails
        let _ = std::fs::remove_file(&temp_file_path);
        AppError::InvalidState(format!("Archive extraction task failed: {e}"))
    })??;

    if was_disabled {
        log::info!("Restoring disabled state for {}", installed_path.display());
        let path = installed_path.clone();
        tokio::task::spawn_blocking(move || apply_disabled_marker(&path))
            .await
            .map_err(|e| AppError::InvalidState(format!("Apply disabled marker failed: {e}")))??;
    }

    log::info!("Mod installed successfully at: {installed_path:?}");
    Ok(installed_path)
}

#[derive(Debug, Clone, Copy)]
enum ArchiveKind {
    Zip,
    Tar,
    TarGz,
}

fn parse_disposition_filename(header_value: &str) -> Option<String> {
    // very simple parser for filename=... parameter
    // e.g. attachment; filename="foo.zip" or filename=foo.zip
    for part in header_value.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename=") {
            let mut val = rest.trim().to_string();
            if (val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\''))
            {
                val.remove(0);
                val.pop();
            }
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

fn sanitize_mod_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_space = true; // Start true to trim leading spaces

    for c in name.chars() {
        if c.is_ascii_control() {
            continue;
        }

        let is_illegal = matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
        let is_space = c == ' ' || is_illegal;

        if is_space {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(c);
            last_was_space = false;
        }
    }

    // Trim trailing space and dots
    out.trim_end_matches([' ', '.']).to_string()
}

/// Sanitize an archive entry path to prevent path traversal attacks.
/// Removes `.` and `..` components and ensures the path is relative.
fn sanitize_archive_path(entry_name: &str) -> Option<PathBuf> {
    let path = std::path::Path::new(entry_name);
    let mut result = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::Normal(name) => {
                result.push(name);
            }
            std::path::Component::CurDir => {
                // Skip "." components
            }
            std::path::Component::ParentDir => {
                // Reject paths with ".." - this is a traversal attempt
                return None;
            }
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                // Reject absolute paths
                return None;
            }
        }
    }

    if result.as_os_str().is_empty() {
        None
    } else {
        Some(result)
    }
}

fn has_zip_magic(bytes: &bytes::Bytes) -> bool {
    bytes.len() >= 4
        && ((bytes[0] == 0x50 && bytes[1] == 0x4B && bytes[2] == 0x03 && bytes[3] == 0x04)
            || (bytes[0] == 0x50 && bytes[1] == 0x4B && bytes[2] == 0x05 && bytes[3] == 0x06)
            || (bytes[0] == 0x50 && bytes[1] == 0x4B && bytes[2] == 0x07 && bytes[3] == 0x08))
}

fn has_gzip_magic(bytes: &bytes::Bytes) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1F && bytes[1] == 0x8B
}

fn has_tar_ustar(bytes: &bytes::Bytes) -> bool {
    bytes.len() > 262 && &bytes[257..262] == b"ustar"
}

fn guess_archive_kind(
    bytes: &bytes::Bytes,
    url: &str,
    content_type: Option<&str>,
    cd_filename: Option<&str>,
) -> Option<ArchiveKind> {
    // 1) Strongest: manual magic-byte checks
    if has_zip_magic(bytes) {
        return Some(ArchiveKind::Zip);
    }
    if has_gzip_magic(bytes) {
        return Some(ArchiveKind::TarGz);
    }
    if has_tar_ustar(bytes) {
        return Some(ArchiveKind::Tar);
    }

    // 2) infer crate as a hint
    if let Some(kind) = infer::get(bytes) {
        match kind.mime_type() {
            "application/zip" | "application/x-zip-compressed" => return Some(ArchiveKind::Zip),
            "application/x-tar" => return Some(ArchiveKind::Tar),
            "application/gzip" | "application/x-gzip" => return Some(ArchiveKind::TarGz),
            _ => {}
        }
    }

    // 3) Headers/filename hints, but only accept if minimally consistent with bytes
    if let Some(ct) = content_type {
        let ct = ct.to_ascii_lowercase();
        if ct.contains("zip") && has_zip_magic(bytes) {
            return Some(ArchiveKind::Zip);
        }
        if (ct.contains("x-tar") || ct == "application/tar")
            && (has_tar_ustar(bytes) || !has_zip_magic(bytes))
        {
            return Some(ArchiveKind::Tar);
        }
        if ct.contains("gzip") && has_gzip_magic(bytes) {
            return Some(ArchiveKind::TarGz);
        }
    }

    let name = cd_filename
        .map(|s| s.to_string())
        .or_else(|| url.split('?').next().map(|s| s.to_string()));
    if let Some(n) = name {
        let n = n.to_ascii_lowercase();
        if n.ends_with(".zip") && has_zip_magic(bytes) {
            return Some(ArchiveKind::Zip);
        }
        if n.ends_with(".tar") && (has_tar_ustar(bytes) || !has_zip_magic(bytes)) {
            return Some(ArchiveKind::Tar);
        }
        if (n.ends_with(".tar.gz") || n.ends_with(".tgz") || n.ends_with(".gz"))
            && has_gzip_magic(bytes)
        {
            return Some(ArchiveKind::TarGz);
        }
    }

    None
}

fn handle_zip(archive_path: &Path, mod_dir: &Path, mod_name: &str) -> Result<PathBuf, AppError> {
    let file = File::open(archive_path).map_err(|e| {
        log::error!("Failed to open archive file: {}", e);
        AppError::FileRead {
            path: archive_path.to_path_buf(),
            source: e.to_string(),
        }
    })?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    log::debug!("Parsing ZIP archive, size: {} bytes", file_size);

    let reader = BufReader::new(file);
    let mut zip = ZipArchive::new(reader).map_err(|e| {
        log::error!("Failed to parse ZIP archive: {}", e);
        AppError::FileWrite {
            path: mod_dir.to_path_buf(),
            source: format!("Invalid zip archive: {e}"),
        }
    })?;

    log::debug!("ZIP archive contains {} entries", zip.len());

    // Inspect the archive layout to decide whether it has a single wrapper
    // directory that should be stripped (like GitHub source zips, which wrap
    // everything in `<repo>-<ref>/`) or whether the mod files live at the
    // archive root and must be kept as-is.
    //
    // We only strip when there is exactly ONE top-level directory and no files
    // at the root. Mod release zips frequently ship sibling directories at the
    // root (e.g. `lib/`, `lovely/`, `src/`); blindly stripping to the first
    // entry there would discard every sibling but the first (see #363).
    let mut has_root_files = false;
    let mut top_level: std::collections::HashSet<String> = std::collections::HashSet::new();
    for i in 0..zip.len() {
        let file = zip.by_index(i).map_err(|e| AppError::FileRead {
            path: mod_dir.to_path_buf(),
            source: format!("Zip entry error: {e}"),
        })?;
        let name = file.name();
        if name.starts_with("__MACOSX/") || name.starts_with("__MACOSX\\") {
            continue;
        }
        let normalized = name.trim_start_matches("./").trim_start_matches(".\\");
        let first = normalized.split(['/', '\\']).next().unwrap_or("");
        if first.is_empty() {
            continue;
        }
        // A file sitting directly at the archive root has no path separator.
        if !file.is_dir()
            && !normalized
                .trim_end_matches(['/', '\\'])
                .contains(['/', '\\'])
        {
            has_root_files = true;
        }
        top_level.insert(first.to_string());
    }

    let single_wrapper_dir = !has_root_files && top_level.len() == 1;

    // The target directory where the mod will be installed
    let target_dir = mod_dir.join(mod_name);

    // Remove target directory if it exists (with retry logic)
    if target_dir.exists() {
        remove_dir_with_retry(&target_dir, 5)?;
    }

    // Use staging directory for atomic extraction
    // Extract to a temp location first, then rename to final destination
    // This ensures partial extractions don't leave corrupted state
    let staging_dir = mod_dir.join(format!(".staging_{}", mod_name));
    if staging_dir.exists() {
        remove_dir_with_retry(&staging_dir, 5)?;
    }

    // Helper to clean up staging on error
    let cleanup_staging = |staging: &Path| {
        if staging.exists() {
            let _ = fs::remove_dir_all(staging);
        }
    };

    if !single_wrapper_dir {
        // Mod files live at the archive root (root files and/or multiple
        // top-level directories) - extract them directly, preserving layout.
        fs::create_dir_all(&staging_dir).map_err(|e| AppError::DirCreate {
            path: staging_dir.clone(),
            source: e.to_string(),
        })?;

        if let Err(e) = extract_zip_root(&mut zip, &staging_dir) {
            cleanup_staging(&staging_dir);
            return Err(e);
        }

        // Atomic rename from staging to target
        if let Err(e) = fs::rename(&staging_dir, &target_dir) {
            cleanup_staging(&staging_dir);
            return Err(AppError::FileWrite {
                path: staging_dir.clone(),
                source: format!("Failed to rename staging directory: {e}"),
            });
        }
    } else {
        // Single wrapper directory - strip it and keep the inner contents.
        fs::create_dir_all(&staging_dir).map_err(|e| AppError::DirCreate {
            path: staging_dir.clone(),
            source: e.to_string(),
        })?;

        // Extract to staging directory
        if let Err(e) = extract_zip(&mut zip, &staging_dir) {
            cleanup_staging(&staging_dir);
            return Err(e);
        }

        // Get root directory name
        let root_dir = match get_zip_root_dir(&mut zip, &staging_dir) {
            Ok(dir) => dir,
            Err(e) => {
                cleanup_staging(&staging_dir);
                return Err(e);
            }
        };
        let source_dir = staging_dir.join(root_dir);

        // Move inner folder to target directory
        if let Err(e) = fs::rename(&source_dir, &target_dir) {
            cleanup_staging(&staging_dir);
            return Err(AppError::FileWrite {
                path: source_dir.clone(),
                source: format!("Failed to rename directory: {e}"),
            });
        }

        // Clean up staging directory
        let _ = remove_dir_with_retry(&staging_dir, 5);
    }

    Ok(target_dir)
}

fn extract_zip_root<R: Read + io::Seek>(
    zip: &mut ZipArchive<R>,
    path: &PathBuf,
) -> Result<(), AppError> {
    if zip.len() > MAX_ARCHIVE_FILES {
        return Err(AppError::ArchiveTooLarge {
            reason: format!(
                "Archive contains {} files, exceeds limit of {}",
                zip.len(),
                MAX_ARCHIVE_FILES
            ),
        });
    }

    fs::create_dir_all(path).map_err(|e| AppError::DirCreate {
        path: path.clone(),
        source: e.to_string(),
    })?;

    let mut total_decompressed: u64 = 0;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| AppError::FileRead {
            path: path.clone(),
            source: format!("Zip entry error: {e}"),
        })?;

        if file.name().starts_with("__MACOSX/") {
            continue;
        }

        let sanitized = match sanitize_archive_path(file.name()) {
            Some(p) => p,
            None => {
                log::warn!("Skipping suspicious archive entry: {}", file.name());
                continue;
            }
        };
        let entry_path = path.join(&sanitized);
        ensure_safe_path(path, &entry_path)?;

        if file.is_dir() {
            fs::create_dir_all(&entry_path).map_err(|e| AppError::DirCreate {
                path: entry_path.clone(),
                source: e.to_string(),
            })?;
        } else {
            create_parent_dir(&entry_path)?;
            let remaining = MAX_DECOMPRESSED_SIZE.saturating_sub(total_decompressed);
            let bytes_written = copy_file_contents_limited(&mut file, &entry_path, remaining)?;
            total_decompressed += bytes_written;
        }
    }
    Ok(())
}

fn get_zip_root_dir<R: Read + io::Seek>(
    zip: &mut ZipArchive<R>,
    mod_dir: &Path,
) -> Result<String, AppError> {
    let first_entry = zip.by_index(0).map_err(|e| AppError::FileRead {
        path: mod_dir.to_path_buf(),
        source: format!("Zip entry error: {e}"),
    })?;

    let name_parts: Vec<&str> = first_entry.name().split('/').collect();
    name_parts
        .first()
        .map(|s: &&str| s.to_string())
        .ok_or_else(|| AppError::InvalidState("Empty zip archive".into()))
}

fn extract_zip<R: Read + io::Seek>(
    zip: &mut ZipArchive<R>,
    mod_dir: &Path,
) -> Result<(), AppError> {
    if zip.len() > MAX_ARCHIVE_FILES {
        return Err(AppError::ArchiveTooLarge {
            reason: format!(
                "Archive contains {} files, exceeds limit of {}",
                zip.len(),
                MAX_ARCHIVE_FILES
            ),
        });
    }

    let mut total_decompressed: u64 = 0;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| AppError::FileRead {
            path: mod_dir.to_path_buf(),
            source: format!("Zip entry error: {e}"),
        })?;

        if file.name().starts_with("__MACOSX/") {
            continue;
        }

        let sanitized = match sanitize_archive_path(&file.mangled_name().to_string_lossy()) {
            Some(p) => p,
            None => {
                log::warn!(
                    "Skipping suspicious archive entry: {:?}",
                    file.mangled_name()
                );
                continue;
            }
        };
        let entry_path = mod_dir.join(&sanitized);
        ensure_safe_path(mod_dir, &entry_path)?;

        if file.is_dir() {
            fs::create_dir_all(&entry_path).map_err(|e| AppError::DirCreate {
                path: entry_path.clone(),
                source: e.to_string(),
            })?;
        } else {
            create_parent_dir(&entry_path)?;
            let remaining = MAX_DECOMPRESSED_SIZE.saturating_sub(total_decompressed);
            let bytes_written = copy_file_contents_limited(&mut file, &entry_path, remaining)?;
            total_decompressed += bytes_written;
        }
    }
    Ok(())
}

fn handle_tar(archive_path: &Path, mod_dir: &Path, mod_name: &str) -> Result<PathBuf, AppError> {
    let file = File::open(archive_path).map_err(|e| AppError::FileRead {
        path: archive_path.to_path_buf(),
        source: e.to_string(),
    })?;
    let reader = BufReader::new(file);
    let mut tar = Archive::new(reader);
    extract_tar_to_staging(&mut tar, mod_dir, mod_name)
}

fn handle_tar_gz(archive_path: &Path, mod_dir: &Path, mod_name: &str) -> Result<PathBuf, AppError> {
    let file = File::open(archive_path).map_err(|e| AppError::FileRead {
        path: archive_path.to_path_buf(),
        source: e.to_string(),
    })?;
    let reader = BufReader::new(file);
    let gz = GzDecoder::new(reader);
    let mut tar = Archive::new(gz);
    extract_tar_to_staging(&mut tar, mod_dir, mod_name)
}

/// Extract tar archive using staging directory for atomic installation.
/// Extracts to a temporary staging directory first, then renames to final location.
fn extract_tar_to_staging(
    tar: &mut Archive<impl Read>,
    mod_dir: &Path,
    mod_name: &str,
) -> Result<PathBuf, AppError> {
    let target_dir = mod_dir.join(mod_name);

    // Remove target directory if it exists
    if target_dir.exists() {
        remove_dir_with_retry(&target_dir, 5)?;
    }

    // Use staging directory for atomic extraction
    let staging_dir = mod_dir.join(format!(".staging_{}", mod_name));
    if staging_dir.exists() {
        remove_dir_with_retry(&staging_dir, 5)?;
    }

    fs::create_dir_all(&staging_dir).map_err(|e| AppError::DirCreate {
        path: staging_dir.clone(),
        source: e.to_string(),
    })?;

    // Helper to clean up staging on error
    let cleanup_staging = || {
        if staging_dir.exists() {
            let _ = fs::remove_dir_all(&staging_dir);
        }
    };

    // Extract to staging directory
    if let Err(e) = extract_tar_contents(tar, &staging_dir) {
        cleanup_staging();
        return Err(e);
    }

    // Atomic rename from staging to target
    if let Err(e) = fs::rename(&staging_dir, &target_dir) {
        cleanup_staging();
        return Err(AppError::FileWrite {
            path: staging_dir.clone(),
            source: format!("Failed to rename staging directory: {e}"),
        });
    }

    Ok(target_dir)
}

/// Extract tar contents to the given directory.
fn extract_tar_contents(tar: &mut Archive<impl Read>, target_dir: &Path) -> Result<(), AppError> {
    let entries = tar.entries().map_err(|e| AppError::FileRead {
        path: target_dir.to_path_buf(),
        source: format!("Tar entry error: {e}"),
    })?;

    let mut total_decompressed: u64 = 0;
    let mut file_count: usize = 0;

    for entry in entries {
        let mut entry = entry.map_err(|e| AppError::FileRead {
            path: target_dir.to_path_buf(),
            source: format!("Tar entry error: {e}"),
        })?;

        file_count += 1;
        if file_count > MAX_ARCHIVE_FILES {
            return Err(AppError::ArchiveTooLarge {
                reason: format!("Archive contains more than {} files", MAX_ARCHIVE_FILES),
            });
        }

        let entry_path = entry.path().map_err(|e| AppError::FileRead {
            path: target_dir.to_path_buf(),
            source: format!("Invalid path in tar: {e}"),
        })?;

        let sanitized = match sanitize_archive_path(&entry_path.to_string_lossy()) {
            Some(p) => p,
            None => {
                log::warn!("Skipping suspicious tar entry: {:?}", entry_path);
                continue;
            }
        };
        let path = target_dir.join(&sanitized);
        ensure_safe_path(target_dir, &path)?;

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&path).map_err(|e| AppError::DirCreate {
                path: path.clone(),
                source: e.to_string(),
            })?;
        } else {
            create_parent_dir(&path)?;
            let remaining = MAX_DECOMPRESSED_SIZE.saturating_sub(total_decompressed);
            let bytes_written = copy_file_contents_limited(&mut entry, &path, remaining)?;
            total_decompressed += bytes_written;
        }
    }

    Ok(())
}

fn create_parent_dir(path: &Path) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::DirCreate {
            path: parent.to_path_buf(),
            source: e.to_string(),
        })
    } else {
        Ok(())
    }
}

/// Copy file contents with size limit enforcement.
/// Returns the number of bytes written.
fn copy_file_contents_limited(
    reader: &mut impl io::Read,
    path: &PathBuf,
    max_bytes: u64,
) -> Result<u64, AppError> {
    let mut output = fs::File::create(path).map_err(|e| AppError::FileWrite {
        path: path.clone(),
        source: e.to_string(),
    })?;

    let mut total: u64 = 0;
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = reader.read(&mut buffer).map_err(|e| AppError::FileRead {
            path: path.clone(),
            source: e.to_string(),
        })?;

        if bytes_read == 0 {
            break;
        }

        total += bytes_read as u64;
        if total > max_bytes {
            drop(output);
            let _ = fs::remove_file(path);
            return Err(AppError::ArchiveTooLarge {
                reason: format!(
                    "Decompressed size exceeds {} MB limit",
                    max_bytes / 1024 / 1024
                ),
            });
        }

        output
            .write_all(&buffer[..bytes_read])
            .map_err(|e| AppError::FileWrite {
                path: path.clone(),
                source: e.to_string(),
            })?;
    }

    Ok(total)
}

fn ensure_safe_path(base: &Path, path: &Path) -> Result<(), AppError> {
    if !path.starts_with(base) {
        Err(AppError::PathValidation {
            path: path.to_path_buf(),
            reason: "Path traversal attempt detected".into(),
        })
    } else {
        Ok(())
    }
}

fn apply_disabled_marker(mod_path: &Path) -> Result<(), AppError> {
    let entries = fs::read_dir(mod_path).map_err(|e| AppError::FileRead {
        path: mod_path.to_path_buf(),
        source: e.to_string(),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| AppError::FileRead {
            path: mod_path.to_path_buf(),
            source: e.to_string(),
        })?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            let ignore_path = entry_path.join(".lovelyignore");
            fs::write(&ignore_path, "").map_err(|e| AppError::FileWrite {
                path: ignore_path,
                source: e.to_string(),
            })?;
        }
    }

    let top_level_ignore = mod_path.join(".lovelyignore");
    fs::write(&top_level_ignore, "").map_err(|e| AppError::FileWrite {
        path: top_level_ignore,
        source: e.to_string(),
    })
}

/// Retry removing a directory with exponential backoff to handle file locks on Windows.
/// Uses longer delays on Windows where file handle release is slower.
fn remove_dir_with_retry(path: &PathBuf, max_retries: u32) -> Result<(), AppError> {
    let mut last_error = None;

    let base_delay_ms = DIR_REMOVE_BASE_DELAY_MS;

    for attempt in 0..max_retries {
        match fs::remove_dir_all(path) {
            Ok(_) => {
                if attempt > 0 {
                    log::info!(
                        "Successfully removed directory after {} retries: {path:?}",
                        attempt
                    );
                }
                return Ok(());
            }
            Err(e) => {
                last_error = Some(e);

                if attempt < max_retries - 1 {
                    // Calculate exponential backoff: 100ms, 200ms, 400ms, 800ms, 1600ms on Windows
                    // or 50ms, 100ms, 200ms, 400ms, 800ms on other platforms
                    let delay_ms = base_delay_ms * (1 << attempt);
                    log::warn!(
                        "Failed to remove directory {path:?} (attempt {}/{}): {}. Retrying in {}ms...",
                        attempt + 1,
                        max_retries,
                        last_error.as_ref().unwrap(),
                        delay_ms
                    );
                    thread::sleep(Duration::from_millis(delay_ms));
                }
            }
        }
    }

    // All retries exhausted
    Err(AppError::FileWrite {
        path: path.clone(),
        source: format!(
            "Failed to remove directory after {} attempts: {}",
            max_retries,
            last_error.unwrap()
        ),
    })
}

pub async fn uninstall_mod(path: PathBuf) -> Result<(), AppError> {
    log::info!("Uninstalling mod: {path:?}");

    let mods_dir = resolve_mods_dir()?;
    let candidates = mod_dir_candidates().unwrap_or_else(|_| vec![mods_dir.clone()]);

    validate_uninstall_path(&path, &candidates)?;
    log::debug!("Uninstall path validation passed for: {:?}", path);

    if let Some(dir_name) = path.file_name().and_then(|n| n.to_str())
        && dir_name.starts_with("Steamodded-smods-")
    {
        log::info!("Uninstalling Steamodded variant: {dir_name}");
    }

    // Use retry logic to handle file locks, especially on Windows
    // Run in blocking task since fs operations are synchronous
    let result = tokio::task::spawn_blocking(move || remove_dir_with_retry(&path, 5))
        .await
        .map_err(|e| AppError::InvalidState(format!("Uninstall task failed: {e}")))?;

    if result.is_ok() {
        log::info!("Successfully uninstalled mod");
    }
    result
}

fn validate_uninstall_path(path: &PathBuf, mods_dirs: &[PathBuf]) -> Result<(), AppError> {
    if !path.exists() {
        return Err(AppError::PathValidation {
            path: path.clone(),
            reason: "Path doesn't exist".into(),
        });
    }

    // Path::starts_with is lexical, so `Mods/../..` would otherwise pass
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::PathValidation {
            path: path.clone(),
            reason: "Path contains parent directory components".into(),
        });
    }

    if mods_dirs.iter().any(|mods_dir| path == mods_dir) {
        return Err(AppError::InvalidState(
            "Blocked attempt to delete Mods directory".into(),
        ));
    }

    if !mods_dirs.iter().any(|mods_dir| path.starts_with(mods_dir)) {
        return Err(AppError::PathValidation {
            path: path.clone(),
            reason: "Path outside Mods directory".into(),
        });
    }

    Ok(())
}

fn canonical_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Checks that `candidate` is an app removable mod folder: a real directory
/// sitting directly in a Mods dir that is not the fresh install, does not
/// overlap it or another tracked mod, and is not a git checkout
fn check_replaced_dir(
    candidate: &Path,
    new_path: &Path,
    mods_dirs: &[PathBuf],
    protected_paths: &[PathBuf],
) -> Result<(), String> {
    let meta = fs::symlink_metadata(candidate).map_err(|e| format!("cannot stat: {e}"))?;
    if !meta.is_dir() {
        return Err("not a real directory".into());
    }

    validate_uninstall_path(&candidate.to_path_buf(), mods_dirs).map_err(|e| e.to_string())?;

    let canonical = canonical_or_self(candidate);
    let parent_is_mods_dir = canonical
        .parent()
        .is_some_and(|parent| mods_dirs.iter().any(|m| canonical_or_self(m) == parent));
    if !parent_is_mods_dir {
        return Err("not a direct child of the Mods directory".into());
    }

    let name = candidate
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if name.is_empty() || name.starts_with('.') || name == "lovely" || name == "bmm-compat" {
        return Err("reserved folder".into());
    }

    let overlaps = |other: &Path| {
        let other = canonical_or_self(other);
        canonical.starts_with(&other) || other.starts_with(&canonical)
    };
    if overlaps(new_path) {
        return Err("overlaps the new install".into());
    }
    if protected_paths.iter().any(|p| overlaps(p)) {
        return Err("belongs to another tracked mod".into());
    }

    if candidate.join(".git").exists() {
        return Err("looks like a git checkout".into());
    }

    Ok(())
}

/// Cleans up after a successful install or update of `mod_name` at `new_path`
///
/// - removes `previous_path` (the folder the DB recorded before this install)
///   when it differs from the new folder
/// - for Steamodded also removes other `smods*`/`Steamodded*` copies in the
///   same Mods dir, so updates never leave a second copy behind
/// - every removal goes through [`check_replaced_dir`] and Steamodded folders
///   must also contain a Steamodded install
/// - a disabled previous install keeps the new one disabled
///
/// Returns the removed folders, failures are only logged
pub fn cleanup_replaced_install(
    mod_name: &str,
    previous_path: Option<&Path>,
    new_path: &Path,
    mods_dirs: &[PathBuf],
    protected_paths: &[PathBuf],
) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    if !new_path.is_dir() {
        log::warn!(
            "Skipping cleanup for {mod_name}: new install {} is missing",
            new_path.display()
        );
        return removed;
    }

    let is_steamodded = mod_name.eq_ignore_ascii_case("steamodded");
    // (folder, is the DB recorded previous install)
    let mut candidates: Vec<(PathBuf, bool)> = Vec::new();
    if let Some(prev) = previous_path.filter(|p| !p.as_os_str().is_empty() && p.exists()) {
        candidates.push((prev.to_path_buf(), true));
    }
    if is_steamodded {
        // Only the Mods dir holding the new install matters for duplicate loading
        let new_parent = canonical_or_self(new_path).parent().map(Path::to_path_buf);
        for mods_dir in mods_dirs
            .iter()
            .filter(|m| new_parent.as_deref() == Some(canonical_or_self(m).as_path()))
        {
            candidates.extend(
                crate::smods_installer::find_steamodded_dirs(mods_dir)
                    .into_iter()
                    .map(|p| (p, false)),
            );
        }
    }

    let mut seen = std::collections::HashSet::new();
    for (candidate, is_previous) in candidates {
        if !seen.insert(canonical_or_self(&candidate)) {
            continue;
        }
        if let Err(reason) = check_replaced_dir(&candidate, new_path, mods_dirs, protected_paths) {
            log::debug!(
                "Keeping {} after installing {mod_name}: {reason}",
                candidate.display()
            );
            continue;
        }
        if is_steamodded && !crate::smods_installer::is_steamodded_install(&candidate) {
            log::warn!(
                "Keeping {}: recorded for Steamodded but it is not a Steamodded install",
                candidate.display()
            );
            continue;
        }

        let was_disabled = is_previous && candidate.join(".lovelyignore").exists();
        match remove_dir_with_retry(&candidate, 5) {
            Ok(()) => {
                log::info!(
                    "Removed previous {mod_name} install at {}",
                    candidate.display()
                );
                if was_disabled
                    && !new_path.join(".lovelyignore").exists()
                    && let Err(e) = apply_disabled_marker(new_path)
                {
                    log::warn!("Failed to keep {mod_name} disabled after update: {e}");
                }
                removed.push(candidate);
            }
            Err(e) => log::warn!(
                "Failed to remove previous {mod_name} install at {}: {e}",
                candidate.display()
            ),
        }
    }

    removed
}

fn resolve_mods_dir() -> Result<PathBuf, AppError> {
    // Ensure Proton symlinks are set up before resolving the mods directory.
    // This ensures that on Linux, the symlink from the Proton prefix to the
    // host mods directory exists before we try to install mods.
    if let Err(e) = ensure_proton_mod_dir_link(None) {
        log::warn!("Failed to ensure Proton mod dir link during install: {}", e);
    }

    let mods_dir = resolve_mods_dir_path().map_err(|e| AppError::DirNotFound(PathBuf::from(e)))?;
    fs::create_dir_all(&mods_dir).map_err(|e| AppError::DirCreate {
        path: mods_dir.clone(),
        source: e.to_string(),
    })?;
    Ok(mods_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use tempfile::tempdir;

    #[test]
    fn guess_archive_kind_magic_zip() {
        let data = Bytes::from_static(&[0x50, 0x4B, 0x03, 0x04]);
        let kind = guess_archive_kind(&data, "", None, None);
        assert!(matches!(kind, Some(ArchiveKind::Zip)));
    }

    #[test]
    fn guess_archive_kind_rejects_misleading_headers() {
        let data = Bytes::from_static(&[0x00, 0x01, 0x02, 0x03]);
        let kind = guess_archive_kind(&data, "file.zip", Some("application/zip"), Some("file.zip"));
        // Without matching magic bytes, should not trust headers/filename alone
        assert!(kind.is_none());
    }

    #[test]
    fn ensure_safe_path_blocks_traversal() {
        let td = tempdir().unwrap();
        let base = td.path().join("base");
        std::fs::create_dir_all(&base).unwrap();
        // clearly outside the base path
        let outside = td.path().join("outside.txt");
        let res = ensure_safe_path(&base, &outside);
        assert!(res.is_err());
    }

    #[test]
    fn sanitize_mod_name_strips_illegal_chars() {
        let name = "Agarmons: A Pokermon Addon";
        let sanitized = sanitize_mod_name(name);
        assert_eq!(sanitized, "Agarmons A Pokermon Addon");
    }

    #[test]
    fn handle_zip_extracts_root_files() {
        use std::io::Write;
        use zip::ZipWriter;
        use zip::write::FileOptions;

        let mut buf: Vec<u8> = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zw = ZipWriter::new(cursor);
            let opts: FileOptions<'_, ()> = FileOptions::default();
            zw.start_file("hello.txt", opts).unwrap();
            zw.write_all(b"hi").unwrap();
            zw.finish().unwrap();
        }

        let td = tempdir().unwrap();
        let mod_dir = td.path();

        // Write buffer to temp file for handle_zip
        let archive_path = td.path().join("test.zip");
        std::fs::write(&archive_path, &buf).unwrap();

        let out = handle_zip(&archive_path, mod_dir, "TestMod").unwrap();
        let file_path = out.join("hello.txt");
        assert!(file_path.exists());
        let content = std::fs::read_to_string(file_path).unwrap();
        assert_eq!(content, "hi");
    }

    #[test]
    fn handle_zip_extracts_folder_structure() {
        use std::io::Write;
        use zip::ZipWriter;
        use zip::write::FileOptions;

        let mut buf: Vec<u8> = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zw = ZipWriter::new(cursor);
            let opts: FileOptions<'_, ()> = FileOptions::default();
            // Write root directory entry first so get_zip_root_dir sees it
            zw.add_directory("Root/", opts).unwrap();
            zw.start_file("Root/readme.md", opts).unwrap();
            zw.write_all(b"docs").unwrap();
            zw.finish().unwrap();
        }

        let td = tempdir().unwrap();
        let mod_dir = td.path();

        // Write buffer to temp file for handle_zip
        let archive_path = td.path().join("test.zip");
        std::fs::write(&archive_path, &buf).unwrap();

        let out = handle_zip(&archive_path, mod_dir, "TestMod").unwrap();
        let file_path = out.join("readme.md");
        assert!(file_path.exists());
        let content = std::fs::read_to_string(file_path).unwrap();
        assert_eq!(content, "docs");
    }

    #[test]
    fn handle_zip_preserves_multiple_top_level_dirs() {
        use std::io::Write;
        use zip::ZipWriter;
        use zip::write::FileOptions;

        // Archive whose root holds several sibling directories with NO single
        // wrapper folder and no root files (e.g. a mod release zip packaged as
        // lib/, lovely/, src/). Regression test for #363: the installer used to
        // assume a single wrapper dir and keep only the first one, discarding
        // the siblings.
        let mut buf: Vec<u8> = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zw = ZipWriter::new(cursor);
            let opts: FileOptions<'_, ()> = FileOptions::default();
            zw.start_file("lib/helper.lua", opts).unwrap();
            zw.write_all(b"lib").unwrap();
            zw.start_file("lovely/patch.toml", opts).unwrap();
            zw.write_all(b"lovely").unwrap();
            zw.start_file("src/main.lua", opts).unwrap();
            zw.write_all(b"src").unwrap();
            zw.finish().unwrap();
        }

        let td = tempdir().unwrap();
        let mod_dir = td.path();
        let archive_path = td.path().join("test.zip");
        std::fs::write(&archive_path, &buf).unwrap();

        let out = handle_zip(&archive_path, mod_dir, "DVPreview").unwrap();

        assert!(out.join("lib/helper.lua").exists(), "lib/ was discarded");
        assert!(
            out.join("lovely/patch.toml").exists(),
            "lovely/ was discarded"
        );
        assert!(out.join("src/main.lua").exists(), "src/ was discarded");
    }

    #[test]
    fn validate_uninstall_path_guards_mods_root() {
        let td = tempdir().unwrap();
        let mods_dir = td.path().join("Mods");
        std::fs::create_dir_all(&mods_dir).unwrap();

        // Should not allow deleting Mods root
        let res = validate_uninstall_path(&mods_dir, std::slice::from_ref(&mods_dir));
        assert!(res.is_err());
    }

    #[test]
    fn validate_uninstall_path_accepts_any_candidate() {
        let td = tempdir().unwrap();
        let mods_dir = td.path().join("Mods");
        let alt_mods_dir = td.path().join("AltMods");
        let alt_mod = alt_mods_dir.join("Talisman");
        std::fs::create_dir_all(&mods_dir).unwrap();
        std::fs::create_dir_all(&alt_mod).unwrap();

        let res = validate_uninstall_path(&alt_mod, &[mods_dir, alt_mods_dir]);
        assert!(res.is_ok());
    }

    #[test]
    fn apply_disabled_marker_creates_ignore_files() {
        let td = tempdir().unwrap();
        let mod_dir = td.path().join("ExampleMod");
        std::fs::create_dir_all(mod_dir.join("sub1")).unwrap();
        std::fs::create_dir_all(mod_dir.join("sub2/nested")).unwrap();

        apply_disabled_marker(&mod_dir).unwrap();

        assert!(mod_dir.join(".lovelyignore").exists());
        assert!(mod_dir.join("sub1/.lovelyignore").exists());
        assert!(mod_dir.join("sub2/.lovelyignore").exists());
        // Nested directories aren't touched directly; parent markers should suffice
        assert!(!mod_dir.join("sub2/nested/.lovelyignore").exists());
    }

    #[test]
    fn sanitize_archive_path_blocks_traversal() {
        assert!(sanitize_archive_path("../etc/passwd").is_none());
        assert!(sanitize_archive_path("foo/../../../etc/passwd").is_none());
        assert!(sanitize_archive_path("/etc/passwd").is_none());
        assert!(sanitize_archive_path("..").is_none());

        // Valid paths should work
        assert_eq!(
            sanitize_archive_path("mod/init.lua"),
            Some(PathBuf::from("mod/init.lua"))
        );
        assert_eq!(
            sanitize_archive_path("./mod/init.lua"),
            Some(PathBuf::from("mod/init.lua"))
        );
    }

    #[test]
    fn test_validate_uninstall_path_rejects_outside_mods_dir() {
        let td = tempfile::tempdir().unwrap();
        let mods_dir = td.path().join("Mods");
        let outside = td.path().join("outside");
        std::fs::create_dir_all(&mods_dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let result = validate_uninstall_path(&outside, &[mods_dir]);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_uninstall_path_rejects_parent_dir_escape() {
        let td = tempfile::tempdir().unwrap();
        let mods_dir = td.path().join("Mods");
        std::fs::create_dir_all(mods_dir.join("SomeMod")).unwrap();
        std::fs::create_dir_all(td.path().join("outside")).unwrap();

        for escape in [
            mods_dir.join(".."),
            mods_dir.join("..").join("outside"),
            mods_dir.join("SomeMod").join("..").join(".."),
            mods_dir.join("SomeMod").join(".."),
        ] {
            let result = validate_uninstall_path(&escape, std::slice::from_ref(&mods_dir));
            assert!(result.is_err(), "{}", escape.display());
        }
    }

    #[test]
    fn test_validate_uninstall_path_rejects_nonexistent() {
        let td = tempfile::tempdir().unwrap();
        let mods_dir = td.path().join("Mods");
        let nonexistent = mods_dir.join("DoesNotExist");
        std::fs::create_dir_all(&mods_dir).unwrap();

        let result = validate_uninstall_path(&nonexistent, &[mods_dir]);
        assert!(result.is_err());
    }

    fn make_mod(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("main.lua"), "-- mod").unwrap();
    }

    fn make_smods(dir: &Path, version: &str) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("lovely")).unwrap();
        std::fs::write(dir.join("src").join("core.lua"), "").unwrap();
        std::fs::write(dir.join("version.lua"), format!("return \"{version}\"")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            format!("{{\"name\": \"Steamodded\", \"version_number\": \"{version}\"}}"),
        )
        .unwrap();
    }

    fn mods_root() -> (tempfile::TempDir, PathBuf) {
        let td = tempdir().unwrap();
        let mods = td.path().join("Mods");
        std::fs::create_dir_all(&mods).unwrap();
        (td, mods)
    }

    #[test]
    fn cleanup_update_removes_old_folder_after_success() {
        let (_td, mods) = mods_root();
        let old = mods.join("CoolMod-1.0.0");
        let new = mods.join("CoolMod-1.1.0");
        make_mod(&old);
        make_mod(&new);

        let removed = cleanup_replaced_install(
            "CoolMod",
            Some(&old),
            &new,
            std::slice::from_ref(&mods),
            &[],
        );

        assert_eq!(removed, vec![old.clone()]);
        assert!(!old.exists());
        assert!(new.join("main.lua").exists());
    }

    #[test]
    fn cleanup_failed_install_keeps_old_folder() {
        let (_td, mods) = mods_root();
        let old = mods.join("CoolMod-1.0.0");
        make_mod(&old);
        // The new folder never got created because the download failed
        let new = mods.join("CoolMod-1.1.0");

        let removed = cleanup_replaced_install(
            "CoolMod",
            Some(&old),
            &new,
            std::slice::from_ref(&mods),
            &[],
        );

        assert!(removed.is_empty());
        assert!(old.join("main.lua").exists());
    }

    #[test]
    fn cleanup_same_folder_is_a_noop() {
        let (_td, mods) = mods_root();
        let path = mods.join("CoolMod");
        make_mod(&path);

        let removed = cleanup_replaced_install(
            "CoolMod",
            Some(&path),
            &path,
            std::slice::from_ref(&mods),
            &[],
        );

        assert!(removed.is_empty());
        assert!(path.join("main.lua").exists());
    }

    #[test]
    fn cleanup_refuses_to_delete_outside_mods() {
        let (td, mods) = mods_root();
        let new = mods.join("CoolMod");
        make_mod(&new);

        let outside = td.path().join("Documents").join("CoolMod");
        make_mod(&outside);
        let nested = mods.join("OtherMod").join("CoolMod");
        make_mod(&nested);
        let escape = mods.join("..").join("Documents");

        for previous in [&outside, &nested, &escape, &mods] {
            let removed = cleanup_replaced_install(
                "CoolMod",
                Some(previous),
                &new,
                std::slice::from_ref(&mods),
                &[],
            );
            assert!(removed.is_empty(), "{}", previous.display());
            assert!(previous.exists(), "{}", previous.display());
        }
        assert!(new.join("main.lua").exists());
        assert!(outside.join("main.lua").exists());
    }

    #[test]
    fn cleanup_keeps_folder_of_another_tracked_mod() {
        let (_td, mods) = mods_root();
        let shared = mods.join("SharedFolder");
        let new = mods.join("CoolMod");
        make_mod(&shared);
        make_mod(&new);

        let removed = cleanup_replaced_install(
            "CoolMod",
            Some(&shared),
            &new,
            std::slice::from_ref(&mods),
            std::slice::from_ref(&shared),
        );

        assert!(removed.is_empty());
        assert!(shared.exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_does_not_follow_symlinked_previous_install() {
        let (td, mods) = mods_root();
        let target = td.path().join("elsewhere");
        make_mod(&target);
        let link = mods.join("CoolMod-1.0.0");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let new = mods.join("CoolMod");
        make_mod(&new);

        let removed = cleanup_replaced_install(
            "CoolMod",
            Some(&link),
            &new,
            std::slice::from_ref(&mods),
            &[],
        );

        assert!(removed.is_empty());
        assert!(target.join("main.lua").exists());
    }

    #[test]
    fn cleanup_keeps_disabled_state_of_previous_install() {
        let (_td, mods) = mods_root();
        let old = mods.join("CoolMod-1.0.0");
        let new = mods.join("CoolMod-1.1.0");
        make_mod(&old);
        make_mod(&new);
        std::fs::write(old.join(".lovelyignore"), "").unwrap();

        cleanup_replaced_install(
            "CoolMod",
            Some(&old),
            &new,
            std::slice::from_ref(&mods),
            &[],
        );

        assert!(!old.exists());
        assert!(new.join(".lovelyignore").exists());
    }

    #[test]
    fn cleanup_steamodded_update_removes_versioned_folder_and_leftovers() {
        let (_td, mods) = mods_root();
        let recorded = mods.join("smods-1.0.0-beta-1814a");
        let stale = mods.join("Steamodded-smods-abc1234");
        let new = mods.join("Steamodded");
        make_smods(&recorded, "1.0.0~BETA-1814a");
        make_smods(&stale, "0.9.8");
        make_smods(&new, "26.1002.0");
        let unrelated = mods.join("JokerDisplay");
        make_mod(&unrelated);
        // Named like Steamodded but without its files, so it is left alone
        let lookalike = mods.join("smods-notes");
        make_mod(&lookalike);

        let mut removed = cleanup_replaced_install(
            "Steamodded",
            Some(&recorded),
            &new,
            std::slice::from_ref(&mods),
            &[],
        );
        removed.sort();

        assert_eq!(removed, vec![stale.clone(), recorded.clone()]);
        assert!(new.join("manifest.json").exists());
        assert!(unrelated.exists());
        assert!(lookalike.exists());
    }

    #[test]
    fn cleanup_steamodded_keeps_recorded_path_that_is_not_steamodded() {
        let (_td, mods) = mods_root();
        let recorded = mods.join("SomethingElse");
        make_mod(&recorded);
        let new = mods.join("Steamodded");
        make_smods(&new, "26.1002.0");

        let removed = cleanup_replaced_install(
            "Steamodded",
            Some(&recorded),
            &new,
            std::slice::from_ref(&mods),
            &[],
        );

        assert!(removed.is_empty());
        assert!(recorded.exists());
    }

    #[test]
    fn cleanup_steamodded_keeps_git_checkouts() {
        let (_td, mods) = mods_root();
        let checkout = mods.join("smods");
        make_smods(&checkout, "dev");
        std::fs::create_dir_all(checkout.join(".git")).unwrap();
        let new = mods.join("Steamodded");
        make_smods(&new, "26.1002.0");

        let removed =
            cleanup_replaced_install("Steamodded", None, &new, std::slice::from_ref(&mods), &[]);

        assert!(removed.is_empty());
        assert!(checkout.exists());
    }

    #[test]
    fn cleanup_steamodded_leftover_scan_stays_in_active_mods_dir() {
        let td = tempdir().unwrap();
        let active = td.path().join("active").join("Mods");
        let legacy = td.path().join("legacy").join("Mods");
        let new = active.join("Steamodded");
        let other = legacy.join("smods-old");
        make_smods(&new, "26.1002.0");
        make_smods(&other, "1.0.0");

        let removed = cleanup_replaced_install("Steamodded", None, &new, &[active, legacy], &[]);

        assert!(removed.is_empty());
        assert!(other.exists());
    }
}
