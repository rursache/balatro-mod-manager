//! Lovely (mod loader/injector) installer and version management.
//!
//! Lovely is the mod injection framework that enables Balatro mods to work.
//! This module handles:
//! - Downloading and installing the Lovely injector
//! - Version checking and updates
//! - Platform-specific installation (Windows DLL, macOS dylib, Linux .so)
//!
//! # Platform-Specific Details
//!
//! - **Windows/Linux (Proton)**: Uses `winmm.dll` for DLL injection (Lovely 0.10.0+).
//!   Older releases shipped `version.dll`, which Lovely 0.10 refuses to start next to,
//!   so the legacy file is removed whenever `winmm.dll` is installed
//! - **macOS**: Uses `liblovely.dylib` with `DYLD_INSERT_LIBRARIES`
//! - **Linux (native LOVE)**: Uses `liblovely.so` with `LD_PRELOAD`

use crate::errors::AppError;
use crate::http::{shared_client, shared_no_redirect_client};
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Seek};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Injector DLL shipped by Lovely 0.10.0+ for Windows and Proton/Wine
pub const WIN_INJECTOR_DLL: &str = "winmm.dll";
/// Injector DLL names used by older Lovely releases. Lovely 0.10 panics when one is present
pub const LEGACY_WIN_INJECTOR_DLLS: &[&str] = &["version.dll"];
/// Lovely 0.3.x proxy name. Lovely also refuses to start next to it, but it is only reported
const ANCIENT_WIN_INJECTOR_DLL: &str = "dwmapi.dll";
pub const MAC_DYLIB: &str = "liblovely.dylib";
pub const LINUX_SO: &str = "liblovely.so";

const LOVELY_RELEASES_URL: &str = "https://github.com/ethangreen-dev/lovely-injector/releases";
#[cfg_attr(target_os = "macos", allow(dead_code))]
const WIN_ASSET: &str = "lovely-x86_64-pc-windows-msvc.zip";
#[cfg(target_os = "linux")]
const LINUX_ASSET: &str = "lovely-x86_64-unknown-linux-gnu.tar.gz";

/// Result of making sure a Lovely injector is in place
#[derive(Debug, Clone)]
pub struct EnsuredLovely {
    pub path: PathBuf,
    /// Set when this call downloaded Lovely and the release tag is known
    pub installed_version: Option<String>,
}

/// Cached Lovely version with timestamp
struct CachedVersion {
    version: String,
    fetched_at: Instant,
}

/// Disk-persisted Lovely version cache
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedVersionCache {
    version: String,
    /// Unix timestamp in seconds when fetched
    fetched_at_unix: u64,
}

/// Global cache for Lovely version (24 hour TTL)
static VERSION_CACHE: OnceLock<Mutex<Option<CachedVersion>>> = OnceLock::new();
const VERSION_CACHE_TTL_SECS: u64 = 60 * 60 * 24; // 24 hours

/// Get the path to the Lovely version cache file
fn get_version_cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Balatro").join("lovely_version_cache.json"))
}

/// Load Lovely version from disk cache if still fresh
fn load_persisted_version_cache() -> Option<String> {
    let path = get_version_cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let cached: PersistedVersionCache = serde_json::from_str(&content).ok()?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();

    if parse_lovely_version(&cached.version).is_none() {
        log::warn!(
            "Ignoring invalid cached Lovely version {:?}",
            cached.version
        );
        return None;
    }

    if now.saturating_sub(cached.fetched_at_unix) < VERSION_CACHE_TTL_SECS {
        log::debug!("Loaded Lovely version from disk cache: {}", cached.version);
        Some(cached.version)
    } else {
        log::debug!("Lovely version disk cache expired");
        None
    }
}

/// Save Lovely version to disk cache
fn save_persisted_version_cache(version: &str) {
    if let Some(path) = get_version_cache_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let cached = PersistedVersionCache {
            version: version.to_string(),
            fetched_at_unix: now,
        };
        if let Ok(json) = serde_json::to_string(&cached) {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!("Failed to save Lovely version cache: {}", e);
            } else {
                log::debug!("Saved Lovely version to disk cache: {}", version);
            }
        }
    }
}

/// Parses a Lovely release tag such as `0.10.0`, `v0.10.0` or `0.5.0-beta7` into
/// `(major, minor, patch)`. Missing minor or patch parts default to 0
pub fn parse_lovely_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim();
    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether the given Lovely version understands the `--vanilla` launch flag (0.9.0+)
pub fn supports_vanilla_flag(version: Option<&str>) -> bool {
    version
        .and_then(parse_lovely_version)
        .is_some_and(|v| v >= (0, 9, 0))
}

/// Extracts the version from the `/releases/latest` redirect, e.g. ".../tag/v0.10.0" -> "0.10.0"
fn version_from_latest_redirect(is_redirect: bool, location: &str) -> Option<String> {
    if !is_redirect {
        return None;
    }
    let (_, tag) = location.rsplit_once("/tag/")?;
    let tag = tag.split(['?', '#', '/']).next()?.trim();
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    parse_lovely_version(version)?;
    Some(version.to_string())
}

/// Download URL for a Lovely release asset, pinned to `version` when it is known
pub fn lovely_asset_url(version: Option<&str>, asset: &str) -> String {
    let tag = version
        .map(str::trim)
        .map(|v| v.strip_prefix(['v', 'V']).unwrap_or(v))
        .filter(|v| !v.is_empty());
    match tag {
        Some(tag) => format!("{LOVELY_RELEASES_URL}/download/v{tag}/{asset}"),
        None => format!("{LOVELY_RELEASES_URL}/latest/download/{asset}"),
    }
}

fn entry_basename(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn win_injector_names() -> impl Iterator<Item = &'static str> {
    std::iter::once(WIN_INJECTOR_DLL).chain(LEGACY_WIN_INJECTOR_DLLS.iter().copied())
}

/// Picks the injector DLL to extract from a Lovely Windows zip, preferring
/// `winmm.dll` over the legacy names. Matches the whole file name only
pub fn select_injector_entry(entry_names: &[&str]) -> Option<&'static str> {
    win_injector_names().find(|wanted| {
        entry_names
            .iter()
            .any(|name| entry_basename(name).eq_ignore_ascii_case(wanted))
    })
}

/// Adds `<dll_stem>=n,b` to a WINEDLLOVERRIDES value unless the DLL is already
/// listed. Wine uses `;` separated `name[,name...]=mode` entries
pub fn merge_wine_dll_overrides(existing: Option<&str>, dll_stem: &str) -> String {
    let existing = existing.unwrap_or("").trim();
    let already_listed = existing.split(';').any(|entry| {
        let names = entry.split_once('=').map_or(entry, |(names, _)| names);
        names.split(',').any(|name| {
            let name = name.trim().to_ascii_lowercase();
            name.strip_suffix(".dll").unwrap_or(&name) == dll_stem.to_ascii_lowercase()
        })
    });
    if already_listed {
        return existing.to_string();
    }

    let addition = format!("{dll_stem}=n,b");
    if existing.is_empty() {
        addition
    } else if existing.ends_with(';') {
        format!("{existing}{addition}")
    } else {
        format!("{existing};{addition}")
    }
}

/// Which Lovely injector DLLs exist in a directory (active or `.disabled`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinInjectorState {
    None,
    Current,
    Legacy,
    Both,
}

pub fn detect_win_injector(dir: &Path) -> WinInjectorState {
    let current = injector_artifact_exists(&dir.join(WIN_INJECTOR_DLL));
    let legacy = LEGACY_WIN_INJECTOR_DLLS
        .iter()
        .any(|name| injector_artifact_exists(&dir.join(name)));
    match (current, legacy) {
        (true, true) => WinInjectorState::Both,
        (true, false) => WinInjectorState::Current,
        (false, true) => WinInjectorState::Legacy,
        (false, false) => WinInjectorState::None,
    }
}

pub fn win_injector_installed(dir: &Path) -> bool {
    detect_win_injector(dir) != WinInjectorState::None
}

/// The first active legacy injector DLL in `dir`, if any
pub fn active_legacy_win_injector(dir: &Path) -> Option<PathBuf> {
    LEGACY_WIN_INJECTOR_DLLS
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// How the Linux (Proton) launcher gets an injector DLL into its cache
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachedInjectorPlan {
    /// The cache already holds this DLL
    Cached(&'static str),
    /// Only the game dir has this DLL, copy it into the cache first
    SeedFromGame(&'static str),
    Download,
}

/// Prefers `winmm.dll` from either location before falling back to a legacy DLL
pub fn plan_cached_win_injector(cache_dir: &Path, game_dir: &Path) -> CachedInjectorPlan {
    for name in win_injector_names() {
        if cache_dir.join(name).is_file() {
            return CachedInjectorPlan::Cached(name);
        }
        if game_dir.join(name).is_file() {
            return CachedInjectorPlan::SeedFromGame(name);
        }
    }
    CachedInjectorPlan::Download
}

fn remove_file_if_exists(path: &Path) -> Result<bool, AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(AppError::FileWrite {
            path: path.to_path_buf(),
            source: format!("Failed to remove file, delete it manually: {e}"),
        }),
    }
}

/// Removes an injector file and its `.disabled` twin, returning what was deleted
fn remove_injector_artifacts(active_path: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut removed = Vec::new();
    for path in
        std::iter::once(active_path.to_path_buf()).chain(disabled_injector_path(active_path))
    {
        if remove_file_if_exists(&path)? {
            removed.push(path);
        }
    }
    Ok(removed)
}

/// Deletes pre 0.10 injector DLLs (and `.disabled` copies) from `dir`.
/// Lovely 0.10 panics at startup when it finds one next to `winmm.dll`
pub fn remove_legacy_win_injectors(dir: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut removed = Vec::new();
    for name in LEGACY_WIN_INJECTOR_DLLS {
        removed.extend(remove_injector_artifacts(&dir.join(name))?);
    }
    for path in &removed {
        log::info!("Removed legacy Lovely injector {}", path.display());
    }

    let ancient = dir.join(ANCIENT_WIN_INJECTOR_DLL);
    if ancient.is_file() {
        log::warn!(
            "Found {} from a very old Lovely install. Lovely will refuse to start until it is deleted",
            ancient.display()
        );
    }
    Ok(removed)
}

fn temp_sibling(dest: &Path) -> Result<PathBuf, AppError> {
    let name = dest
        .file_name()
        .ok_or_else(|| AppError::InvalidState(format!("Invalid target path {}", dest.display())))?;
    let mut tmp_name = OsString::from(name);
    tmp_name.push(".bmm-tmp");
    Ok(dest.with_file_name(tmp_name))
}

/// Writes `bytes` next to `dest` and renames it into place, so a failed
/// write never leaves the old file deleted or half written
fn install_bytes_atomically(bytes: &[u8], dest: &Path, mode: Option<u32>) -> Result<(), AppError> {
    let tmp = temp_sibling(dest)?;
    let result = (|| -> std::io::Result<()> {
        fs::write(&tmp, bytes)?;
        #[cfg(unix)]
        {
            if let Some(mode) = mode {
                fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
            }
        }
        #[cfg(not(unix))]
        let _ = mode;
        fs::rename(&tmp, dest)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(AppError::FileWrite {
            path: dest.to_path_buf(),
            source: e.to_string(),
        });
    }
    Ok(())
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn copy_file_atomically(src: &Path, dest: &Path) -> Result<(), AppError> {
    let tmp = temp_sibling(dest)?;
    let result = fs::copy(src, &tmp).and_then(|_| fs::rename(&tmp, dest));
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(AppError::FileCopy {
            source: src.display().to_string(),
            dest: dest.display().to_string(),
            source_error: e.to_string(),
        });
    }
    Ok(())
}

/// Extracts the injector DLL from a Lovely Windows zip into `dest_dir` and
/// returns the file name that was written (`winmm.dll` or a legacy name)
pub fn extract_injector_from_zip<R: Read + Seek>(
    reader: R,
    dest_dir: &Path,
) -> Result<&'static str, AppError> {
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| AppError::InvalidState(format!("Failed to open Lovely zip: {e}")))?;
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let Some(selected) = select_injector_entry(&name_refs) else {
        log::error!("Lovely zip has no known injector DLL, entries: {names:?}");
        return Err(AppError::InvalidState(
            "No Lovely injector DLL (winmm.dll or version.dll) found in downloaded zip".to_string(),
        ));
    };
    let entry_name = names
        .iter()
        .find(|name| entry_basename(name).eq_ignore_ascii_case(selected))
        .ok_or_else(|| AppError::InvalidState(format!("{selected} vanished from zip")))?;

    let mut entry = archive
        .by_name(entry_name)
        .map_err(|e| AppError::InvalidState(format!("Failed to read {entry_name}: {e}")))?;
    let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or_default());
    entry
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::InvalidState(format!("Failed to extract {entry_name}: {e}")))?;

    install_bytes_atomically(&bytes, &dest_dir.join(selected), None)?;
    log::info!(
        "Installed Lovely injector {selected} into {}",
        dest_dir.display()
    );
    Ok(selected)
}

/// Extracts the file named `wanted` from a `.tar.gz` into `dest_dir` atomically
pub fn extract_file_from_tar_gz<R: Read>(
    reader: R,
    wanted: &str,
    dest_dir: &Path,
    mode: Option<u32>,
) -> Result<PathBuf, AppError> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(reader));
    let entries = archive
        .entries()
        .map_err(|e| AppError::InvalidState(format!("Failed to read Lovely archive: {e}")))?;
    for entry in entries {
        let mut entry = entry
            .map_err(|e| AppError::InvalidState(format!("Failed to read Lovely archive: {e}")))?;
        let is_wanted = entry.header().entry_type().is_file()
            && entry
                .path()
                .ok()
                .is_some_and(|p| p.file_name().is_some_and(|n| n == wanted));
        if !is_wanted {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::InvalidState(format!("Failed to extract {wanted}: {e}")))?;
        let dest = dest_dir.join(wanted);
        install_bytes_atomically(&bytes, &dest, mode)?;
        return Ok(dest);
    }
    Err(AppError::InvalidState(format!(
        "{wanted} not found in downloaded Lovely archive"
    )))
}

/// Game folder to manage, preferring the path stored in the database
pub fn resolve_game_dir(db_path: Option<&str>) -> Option<PathBuf> {
    db_path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::finder::get_balatro_paths_cached().into_iter().next())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn bins_path() -> Result<PathBuf, AppError> {
    dirs::config_dir()
        .map(|d| d.join("Balatro").join("bins"))
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("config directory")))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn ensure_bins_dir() -> Result<PathBuf, AppError> {
    let bins_dir = bins_path()?;
    fs::create_dir_all(&bins_dir).map_err(|e| AppError::DirCreate {
        path: bins_dir.clone(),
        source: e.to_string(),
    })?;
    Ok(bins_dir)
}

#[cfg(target_os = "macos")]
fn detect_architecture() -> Result<&'static str, AppError> {
    use libc::{c_void, size_t, sysctl};

    let mut size: size_t = 0;
    let mut mib = [libc::CTL_HW, libc::HW_MACHINE];

    // First call to get buffer size
    unsafe {
        if sysctl(
            mib.as_mut_ptr(),
            2,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return Err(AppError::SystemDetection(
                "Failed to get architecture buffer size".into(),
            ));
        }
    }

    let mut buf = vec![0u8; size];

    // Second call to get actual value
    unsafe {
        if sysctl(
            mib.as_mut_ptr(),
            2,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return Err(AppError::SystemDetection(
                "Failed to retrieve architecture".into(),
            ));
        }
    }

    // Convert buffer to string and match architecture
    match String::from_utf8_lossy(&buf).trim_end_matches('\0') {
        "arm64" => Ok("aarch64"),
        "x86_64" => Ok("x86_64"),
        other => Err(AppError::UnsupportedArchitecture(other.into())),
    }
}

async fn resolve_install_version() -> Option<String> {
    match get_latest_lovely_version().await {
        Ok(version) => Some(version),
        Err(e) => {
            log::warn!("Could not resolve the latest Lovely tag, downloading /latest: {e}");
            None
        }
    }
}

/// Ensures a Lovely injector DLL exists in the game directory (Windows/Linux Proton).
///
/// Prefers `winmm.dll` (Lovely 0.10.0+) and removes any legacy `version.dll` next to it.
/// A lone legacy DLL is kept so an old Lovely keeps working until it is updated.
/// On Linux the DLL is cached in ~/.config/Balatro/bins/ and copied to the game
/// directory each launch, since Steam may verify/restore game files.
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub async fn ensure_win_injector(game_path: &Path) -> Result<EnsuredLovely, AppError> {
    #[cfg(target_os = "windows")]
    {
        let current = game_path.join(WIN_INJECTOR_DLL);
        restore_disabled_injector_file(&current)?;
        if current.is_file() {
            remove_legacy_win_injectors(game_path)?;
            return Ok(EnsuredLovely {
                path: current,
                installed_version: None,
            });
        }
        if let Some(legacy) = active_legacy_win_injector(game_path) {
            log::info!(
                "Using legacy Lovely injector {} until Lovely is updated",
                legacy.display()
            );
            return Ok(EnsuredLovely {
                path: legacy,
                installed_version: None,
            });
        }

        let version = resolve_install_version().await;
        let name = download_win_injector(game_path, version.as_deref()).await?;
        if name == WIN_INJECTOR_DLL {
            remove_legacy_win_injectors(game_path)?;
        }
        Ok(EnsuredLovely {
            path: game_path.join(name),
            installed_version: version,
        })
    }

    #[cfg(target_os = "linux")]
    {
        let cache_dir = ensure_bins_dir()?;
        restore_disabled_injector_file(&cache_dir.join(WIN_INJECTOR_DLL))?;
        restore_disabled_injector_file(&game_path.join(WIN_INJECTOR_DLL))?;

        let mut installed_version = None;
        let name = match plan_cached_win_injector(&cache_dir, game_path) {
            CachedInjectorPlan::Cached(name) => name,
            CachedInjectorPlan::SeedFromGame(name) => {
                copy_file_atomically(&game_path.join(name), &cache_dir.join(name))?;
                name
            }
            CachedInjectorPlan::Download => {
                installed_version = resolve_install_version().await;
                download_win_injector(&cache_dir, installed_version.as_deref()).await?
            }
        };

        let cached = cache_dir.join(name);
        let target = game_path.join(name);
        copy_file_atomically(&cached, &target)?;
        log::debug!(
            "Copied {name} from cache {} to {}",
            cached.display(),
            target.display()
        );
        if name == WIN_INJECTOR_DLL {
            remove_legacy_win_injectors(&cache_dir)?;
            remove_legacy_win_injectors(game_path)?;
        }

        Ok(EnsuredLovely {
            path: target,
            installed_version,
        })
    }
}

/// Makes sure Lovely is present, downloading it when missing.
/// macOS returns the dylib path, Windows/Linux return the Balatro.exe path
pub async fn ensure_lovely_exists() -> Result<EnsuredLovely, AppError> {
    #[cfg(target_os = "macos")]
    {
        let lovely_path = ensure_bins_dir()?.join(MAC_DYLIB);
        restore_disabled_injector_file(&lovely_path)?;
        if lovely_path.exists() {
            return Ok(EnsuredLovely {
                path: lovely_path,
                installed_version: None,
            });
        }

        let version = resolve_install_version().await;
        let path = install_lovely(version.as_deref(), None).await?;
        Ok(EnsuredLovely {
            path,
            installed_version: version,
        })
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        let game_path = resolve_game_dir(None)
            .ok_or_else(|| AppError::DirNotFound(PathBuf::from("Balatro installation")))?;
        let ensured = ensure_win_injector(&game_path).await?;
        Ok(EnsuredLovely {
            path: game_path.join("Balatro.exe"),
            installed_version: ensured.installed_version,
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(AppError::InvalidState(
            "Lovely injection is not supported on this platform.".into(),
        ))
    }
}

/// Downloads Lovely `version` (or the latest release when `None`) and swaps it in
/// atomically. The previous install stays untouched if the download fails.
/// `game_path` is required on Windows/Linux, where the DLL lives in the game folder.
/// Returns the installed injector path
pub async fn install_lovely(
    version: Option<&str>,
    game_path: Option<&Path>,
) -> Result<PathBuf, AppError> {
    #[cfg(target_os = "macos")]
    {
        let _ = game_path;
        let bins_dir = ensure_bins_dir()?;
        let arch = detect_architecture()?;
        let asset = format!("lovely-{arch}-apple-darwin.tar.gz");
        let bytes = download_asset(version, &asset).await?;
        // run_lovely_macos.sh is skipped, BMM sets DYLD_INSERT_LIBRARIES itself
        let path = extract_file_from_tar_gz(bytes.as_ref(), MAC_DYLIB, &bins_dir, Some(0o755))?;
        remove_stale_disabled_twin(&path)?;
        Ok(path)
    }

    #[cfg(target_os = "windows")]
    {
        let game_path = game_path
            .ok_or_else(|| AppError::DirNotFound(PathBuf::from("Balatro installation")))?;
        let name = download_win_injector(game_path, version).await?;
        let path = game_path.join(name);
        remove_stale_disabled_twin(&path)?;
        if name == WIN_INJECTOR_DLL {
            remove_legacy_win_injectors(game_path)?;
        }
        Ok(path)
    }

    #[cfg(target_os = "linux")]
    {
        let game_path = game_path
            .ok_or_else(|| AppError::DirNotFound(PathBuf::from("Balatro installation")))?;
        let cache_dir = ensure_bins_dir()?;
        let name = download_win_injector(&cache_dir, version).await?;
        let cached = cache_dir.join(name);
        let target = game_path.join(name);
        remove_stale_disabled_twin(&cached)?;
        remove_stale_disabled_twin(&target)?;
        copy_file_atomically(&cached, &target)?;
        if name == WIN_INJECTOR_DLL {
            remove_legacy_win_injectors(&cache_dir)?;
            remove_legacy_win_injectors(game_path)?;
        }

        // Refresh the native LOVE injector too when it is in use
        let cached_so = cache_dir.join(LINUX_SO);
        if injector_artifact_exists(&cached_so) {
            download_lovely_so(&cache_dir, version).await?;
            remove_stale_disabled_twin(&cached_so)?;
        }
        Ok(target)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (version, game_path);
        Err(AppError::InvalidState(
            "Lovely injection is not supported on this platform.".into(),
        ))
    }
}

fn remove_stale_disabled_twin(active_path: &Path) -> Result<(), AppError> {
    if let Some(disabled) = disabled_injector_path(active_path) {
        remove_file_if_exists(&disabled)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub async fn ensure_love_binary() -> Result<(PathBuf, Option<PathBuf>), AppError> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("config directory")))?;
    let bins_dir = config_dir.join("Balatro/bins");
    fs::create_dir_all(&bins_dir).map_err(|e| AppError::DirCreate {
        path: bins_dir.clone(),
        source: e.to_string(),
    })?;
    let target_dir = bins_dir.join("love");
    let target_bin = target_dir.join("love");
    if target_bin.exists() {
        // Refresh permissions in case they were lost.
        let perms = std::fs::Permissions::from_mode(0o755);
        let _ = std::fs::set_permissions(&target_bin, perms);
        let lib_dir = target_bin
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("lib"))
            .filter(|p| p.is_dir());
        return Ok((target_bin, lib_dir));
    }

    download_love_appimage_and_extract(&target_dir).await?;
    // Prefer extracted AppImage binary: love/bin/love
    let bin = target_dir.join("bin/love");
    if !bin.exists() {
        return Err(AppError::InvalidState(
            "LOVE AppImage extraction did not produce bin/love".to_string(),
        ));
    }

    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(&bin, perms).map_err(|e| AppError::FileWrite {
        path: bin.clone(),
        source: e.to_string(),
    })?;
    let lib_dir = target_dir.join("lib");
    Ok((
        bin,
        if lib_dir.is_dir() {
            Some(lib_dir)
        } else {
            None
        },
    ))
}

#[cfg(target_os = "linux")]
pub async fn ensure_lovely_so_exists(game_path: &Path) -> Result<PathBuf, AppError> {
    let bins_dir = ensure_bins_dir()?;

    let cached_so = bins_dir.join(LINUX_SO);
    if !cached_so.exists() {
        let version = resolve_install_version().await;
        download_lovely_so(&bins_dir, version.as_deref()).await?;
    }

    let target_so = game_path.join(LINUX_SO);
    copy_file_atomically(&cached_so, &target_so)?;

    // Ensure it is executable for preload
    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(&target_so, perms).map_err(|e| AppError::FileWrite {
        path: target_so.clone(),
        source: e.to_string(),
    })?;

    Ok(target_so)
}

#[cfg(target_os = "linux")]
async fn download_love_appimage_and_extract(target_dir: &Path) -> Result<(), AppError> {
    // Pin to LOVE 11.5 linux x86_64 tarball (latest as of Balatro/Lovely expectations).
    let appimage_url =
        "https://github.com/love2d/love/releases/download/11.5/love-11.5-x86_64.AppImage";

    log::info!("Downloading LOVE AppImage from: {}", appimage_url);

    let client = shared_client();

    let response = client.get(appimage_url).send().await.map_err(|e| {
        log::error!("Failed to connect to LOVE AppImage download: {}", e);
        AppError::Network(format!("Failed to download LOVE AppImage: {e}"))
    })?;

    if !response.status().is_success() {
        log::error!(
            "LOVE AppImage download failed with HTTP {}",
            response.status()
        );
        return Err(AppError::Network(format!(
            "LOVE AppImage download failed: HTTP {}",
            response.status()
        )));
    }

    let bytes = response.bytes().await.map_err(|e| {
        log::error!("Failed to read LOVE AppImage response body: {}", e);
        AppError::Network(format!("Failed to read LOVE AppImage bytes: {e}"))
    })?;

    log::debug!("Downloaded LOVE AppImage: {} bytes", bytes.len());

    let temp_dir = tempfile::tempdir().map_err(|e| AppError::FileWrite {
        path: PathBuf::from("temp directory"),
        source: e.to_string(),
    })?;
    let temp_appimage = temp_dir.path().join("love.AppImage");
    fs::write(&temp_appimage, &bytes).map_err(|e| AppError::FileWrite {
        path: temp_appimage.clone(),
        source: e.to_string(),
    })?;
    // Ensure the AppImage is executable before extraction.
    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(&temp_appimage, perms).map_err(|e| AppError::FileWrite {
        path: temp_appimage.clone(),
        source: e.to_string(),
    })?;

    if target_dir.exists() {
        fs::remove_dir_all(target_dir).map_err(|e| AppError::DirCreate {
            path: target_dir.to_path_buf(),
            source: e.to_string(),
        })?;
    }
    fs::create_dir_all(target_dir).map_err(|e| AppError::DirCreate {
        path: target_dir.to_path_buf(),
        source: e.to_string(),
    })?;

    // Extract the AppImage contents into target_dir/squashfs-root then move its contents up.
    let mut extract = Command::new(&temp_appimage);
    extract
        .arg("--appimage-extract")
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        .current_dir(target_dir);
    let status = extract
        .status()
        .map_err(|e| AppError::InvalidState(format!("Failed to extract LOVE AppImage: {e}")))?;
    if !status.success() {
        return Err(AppError::InvalidState(format!(
            "LOVE AppImage extraction failed with status {status}"
        )));
    }

    let squash_root = target_dir.join("squashfs-root");
    if !squash_root.exists() {
        return Err(AppError::InvalidState(
            "LOVE AppImage extraction did not produce squashfs-root".to_string(),
        ));
    }

    for ent in (fs::read_dir(&squash_root).map_err(|e| AppError::DirCreate {
        path: squash_root.clone(),
        source: e.to_string(),
    })?)
    .flatten()
    {
        let src = ent.path();
        let dst = target_dir.join(ent.file_name());
        let _ = fs::rename(&src, &dst);
    }

    let _ = fs::remove_dir_all(&squash_root);

    Ok(())
}

/// Query GitHub for the latest Lovely release tag (e.g., "0.8.0").
/// Results are cached for 24 hours to avoid blocking game launches.
/// Cache persists to disk to survive app restarts.
pub async fn get_latest_lovely_version() -> Result<String, AppError> {
    // Check in-memory cache first
    let cache = VERSION_CACHE.get_or_init(|| Mutex::new(None));
    {
        let guard = cache.lock().await;
        if let Some(cached) = guard.as_ref()
            && cached.fetched_at.elapsed() < Duration::from_secs(VERSION_CACHE_TTL_SECS)
        {
            log::debug!("Using cached Lovely version: {}", cached.version);
            return Ok(cached.version.clone());
        }
    }

    // Check disk cache (survives app restarts)
    if let Some(version) = load_persisted_version_cache() {
        // Populate in-memory cache from disk
        let mut guard = cache.lock().await;
        *guard = Some(CachedVersion {
            version: version.clone(),
            fetched_at: Instant::now(),
        });
        return Ok(version);
    }

    // Cache miss or expired - fetch from GitHub
    log::debug!("Querying GitHub for latest Lovely release version");

    let client = shared_no_redirect_client();

    let resp = client
        .get("https://github.com/ethangreen-dev/lovely-injector/releases/latest")
        .send()
        .await
        .map_err(|e| {
            log::error!("Failed to query Lovely releases: {}", e);
            AppError::Network(e.to_string())
        })?;

    // GitHub answers with a 3xx whose Location points at /tag/vX.Y.Z. Anything else
    // (rate limit, 5xx, proxy page) must not be cached as a version
    let status = resp.status();
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let Some(version) = version_from_latest_redirect(status.is_redirection(), &location) else {
        log::error!(
            "Unexpected Lovely latest release response: HTTP {} location {:?}",
            status,
            location
        );
        return Err(AppError::InvalidState(
            "Failed to resolve latest Lovely version tag".to_string(),
        ));
    };

    // Update in-memory cache
    {
        let mut guard = cache.lock().await;
        *guard = Some(CachedVersion {
            version: version.clone(),
            fetched_at: Instant::now(),
        });
    }

    // Persist to disk for next app launch
    save_persisted_version_cache(&version);

    log::debug!("Latest Lovely version: {}", version);
    Ok(version)
}

/// Remove every installed Lovely artifact (explicit uninstall).
/// Updates go through [`install_lovely`] instead, which never deletes first
pub fn remove_installed_lovely(game_path: Option<&Path>) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        let _ = game_path;
        remove_injector_artifacts(&bins_path()?.join(MAC_DYLIB))?;
        Ok(())
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        let game_path = game_path
            .map(Path::to_path_buf)
            .or_else(|| resolve_game_dir(None));
        if let Some(game_path) = &game_path {
            for name in win_injector_names() {
                remove_injector_artifacts(&game_path.join(name))?;
            }
            #[cfg(target_os = "linux")]
            {
                if let Err(e) = remove_injector_artifacts(&game_path.join(LINUX_SO)) {
                    log::warn!("Failed to remove liblovely.so from the game folder: {e}");
                }
            }
        }

        // Also remove cached copies on Linux
        #[cfg(target_os = "linux")]
        {
            if let Ok(bins_dir) = bins_path() {
                for name in win_injector_names().chain(std::iter::once(LINUX_SO)) {
                    match remove_injector_artifacts(&bins_dir.join(name)) {
                        Ok(removed) => {
                            for path in removed {
                                log::info!("Removed cached {}", path.display());
                            }
                        }
                        Err(e) => log::warn!("Failed to remove cached {name}: {e}"),
                    }
                }
            }
        }

        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = game_path;
        Err(AppError::InvalidState(
            "Lovely injection is not supported on this platform.".into(),
        ))
    }
}

/// The injector DLL name that is installed in `dir` (active or disabled),
/// defaulting to `winmm.dll`
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn installed_win_injector_name(dir: &Path) -> &'static str {
    win_injector_names()
        .find(|name| injector_artifact_exists(&dir.join(name)))
        .unwrap_or(WIN_INJECTOR_DLL)
}

/// Set whether the Lovely injector is enabled or disabled by renaming files.
/// When disabled, the injector file is renamed to .disabled so it won't load.
/// This is a legacy escape hatch; launch mode should use runtime flags instead.
/// Returns Ok(()) if successful, or an error if the operation fails.
pub fn set_injector_enabled(enabled: bool) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        let bins_dir = bins_path()?;
        let active_path = bins_dir.join(MAC_DYLIB);
        let disabled_path = bins_dir.join("liblovely.dylib.disabled");

        toggle_injector_file(&active_path, &disabled_path, enabled)
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        // For Windows and Linux (Proton), toggle the injector DLL in the game directory
        let Some(game_path) = resolve_game_dir(None) else {
            // No game path found - if we're enabling, that's an error; if disabling, nothing to do
            if enabled {
                return Err(AppError::DirNotFound(PathBuf::from("Balatro installation")));
            }
            return Ok(());
        };
        let active_path = game_path.join(installed_win_injector_name(&game_path));
        let disabled_path = disabled_injector_path(&active_path)
            .ok_or_else(|| AppError::InvalidState("Invalid injector path".into()))?;

        toggle_injector_file(&active_path, &disabled_path, enabled)?;

        // On Linux, also handle liblovely.so for native LOVE builds
        #[cfg(target_os = "linux")]
        {
            if let Ok(bins_dir) = bins_path() {
                let so_active = bins_dir.join(LINUX_SO);
                let so_disabled = bins_dir.join("liblovely.so.disabled");
                // Also check in game directory
                let game_so_active = game_path.join(LINUX_SO);
                let game_so_disabled = game_path.join("liblovely.so.disabled");

                // Toggle both locations (ignore errors if files don't exist)
                let _ = toggle_injector_file(&so_active, &so_disabled, enabled);
                let _ = toggle_injector_file(&game_so_active, &game_so_disabled, enabled);
            }
        }

        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = enabled;
        Err(AppError::InvalidState(
            "Lovely injection is not supported on this platform.".into(),
        ))
    }
}

/// Check if the injector is currently enabled (active file exists).
pub fn is_injector_enabled() -> Result<bool, AppError> {
    #[cfg(target_os = "macos")]
    {
        Ok(bins_path()?.join(MAC_DYLIB).exists())
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        let Some(game_path) = resolve_game_dir(None) else {
            // No game path - consider it "enabled" (no injector to disable)
            return Ok(true);
        };
        Ok(game_path
            .join(installed_win_injector_name(&game_path))
            .exists())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(AppError::InvalidState(
            "Lovely injection is not supported on this platform.".into(),
        ))
    }
}

fn disabled_injector_path(active_path: &Path) -> Option<PathBuf> {
    let file_name = active_path.file_name()?;
    let mut disabled_name = OsString::from(file_name);
    disabled_name.push(".disabled");
    Some(active_path.with_file_name(disabled_name))
}

/// Returns true if an active injector file or its legacy `.disabled` variant exists.
pub fn injector_artifact_exists(active_path: &Path) -> bool {
    active_path.exists()
        || disabled_injector_path(active_path)
            .as_deref()
            .is_some_and(Path::exists)
}

fn restore_disabled_injector_file(active_path: &Path) -> Result<bool, AppError> {
    if active_path.exists() {
        return Ok(false);
    }

    let Some(disabled_path) = disabled_injector_path(active_path) else {
        return Ok(false);
    };

    if !disabled_path.exists() {
        return Ok(false);
    }

    std::fs::rename(&disabled_path, active_path).map_err(|e| AppError::FileWrite {
        path: active_path.to_path_buf(),
        source: format!("Failed to restore injector: {}", e),
    })?;

    Ok(true)
}

/// Helper function to toggle an injector file between active and disabled states.
fn toggle_injector_file(
    active_path: &Path,
    disabled_path: &Path,
    enable: bool,
) -> Result<(), AppError> {
    if enable {
        // Enable: rename .disabled to active
        if disabled_path.exists() {
            // If both exist (corrupt state), remove the disabled one
            if active_path.exists() {
                std::fs::remove_file(disabled_path).map_err(|e| AppError::FileWrite {
                    path: disabled_path.to_path_buf(),
                    source: e.to_string(),
                })?;
            } else {
                std::fs::rename(disabled_path, active_path).map_err(|e| AppError::FileWrite {
                    path: active_path.to_path_buf(),
                    source: format!("Failed to enable injector: {}", e),
                })?;
            }
        }
        // If neither exists, Lovely isn't installed - that's fine, we just save the preference
        Ok(())
    } else {
        // Disable: rename active to .disabled
        if active_path.exists() {
            // If both exist (corrupt state), remove the active one since we're disabling
            if disabled_path.exists() {
                std::fs::remove_file(active_path).map_err(|e| AppError::FileWrite {
                    path: active_path.to_path_buf(),
                    source: e.to_string(),
                })?;
            } else {
                std::fs::rename(active_path, disabled_path).map_err(|e| AppError::FileWrite {
                    path: disabled_path.to_path_buf(),
                    source: format!("Failed to disable injector: {}", e),
                })?;
            }
        }
        // If active doesn't exist, already disabled - that's fine
        Ok(())
    }
}

async fn download_asset(version: Option<&str>, asset: &str) -> Result<bytes::Bytes, AppError> {
    let url = lovely_asset_url(version, asset);
    log::info!("Downloading Lovely from: {url}");

    let response = shared_client().get(&url).send().await.map_err(|e| {
        log::error!("Failed to download Lovely: {e}");
        AppError::Network(format!("Failed to download Lovely injector: {e}"))
    })?;

    if !response.status().is_success() {
        log::error!("Lovely download failed with HTTP {}", response.status());
        return Err(AppError::Network(format!(
            "Lovely download failed: HTTP {}",
            response.status()
        )));
    }

    let bytes = response.bytes().await.map_err(|e| {
        log::error!("Failed to read Lovely download response: {e}");
        AppError::Network(format!("Failed to read download response: {e}"))
    })?;
    log::debug!("Downloaded {asset}: {} bytes", bytes.len());
    Ok(bytes)
}

/// Downloads the Lovely Windows zip and installs its injector DLL into `dest_dir`.
/// Returns the installed file name
#[cfg(any(target_os = "windows", target_os = "linux"))]
async fn download_win_injector(
    dest_dir: &Path,
    version: Option<&str>,
) -> Result<&'static str, AppError> {
    let bytes = download_asset(version, WIN_ASSET).await?;
    extract_injector_from_zip(std::io::Cursor::new(bytes), dest_dir)
}

#[cfg(target_os = "linux")]
async fn download_lovely_so(dest_dir: &Path, version: Option<&str>) -> Result<PathBuf, AppError> {
    let bytes = download_asset(version, LINUX_ASSET).await?;
    extract_file_from_tar_gz(bytes.as_ref(), LINUX_SO, dest_dir, Some(0o755))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_toggle_injector_enable_from_disabled() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("version.dll");
        let disabled = dir.path().join("version.dll.disabled");

        // Create the disabled file
        std::fs::write(&disabled, b"test").unwrap();
        assert!(!active.exists());
        assert!(disabled.exists());

        // Enable should rename disabled to active
        toggle_injector_file(&active, &disabled, true).unwrap();
        assert!(active.exists());
        assert!(!disabled.exists());
    }

    #[test]
    fn test_toggle_injector_disable_from_enabled() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("version.dll");
        let disabled = dir.path().join("version.dll.disabled");

        // Create the active file
        std::fs::write(&active, b"test").unwrap();
        assert!(active.exists());
        assert!(!disabled.exists());

        // Disable should rename active to disabled
        toggle_injector_file(&active, &disabled, false).unwrap();
        assert!(!active.exists());
        assert!(disabled.exists());
    }

    #[test]
    fn test_toggle_injector_already_enabled() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("version.dll");
        let disabled = dir.path().join("version.dll.disabled");

        // Create both files (corrupt state)
        std::fs::write(&active, b"active").unwrap();
        std::fs::write(&disabled, b"disabled").unwrap();

        // Enable should remove the disabled file
        toggle_injector_file(&active, &disabled, true).unwrap();
        assert!(active.exists());
        assert!(!disabled.exists());
        assert_eq!(std::fs::read(&active).unwrap(), b"active");
    }

    #[test]
    fn test_toggle_injector_already_disabled() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("version.dll");
        let disabled = dir.path().join("version.dll.disabled");

        // Create both files (corrupt state)
        std::fs::write(&active, b"active").unwrap();
        std::fs::write(&disabled, b"disabled").unwrap();

        // Disable should remove the active file
        toggle_injector_file(&active, &disabled, false).unwrap();
        assert!(!active.exists());
        assert!(disabled.exists());
        assert_eq!(std::fs::read(&disabled).unwrap(), b"disabled");
    }

    #[test]
    fn test_toggle_injector_no_files_exist() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("version.dll");
        let disabled = dir.path().join("version.dll.disabled");

        // Neither file exists
        assert!(!active.exists());
        assert!(!disabled.exists());

        // Enable should succeed without error
        toggle_injector_file(&active, &disabled, true).unwrap();
        assert!(!active.exists());
        assert!(!disabled.exists());

        // Disable should also succeed without error
        toggle_injector_file(&active, &disabled, false).unwrap();
        assert!(!active.exists());
        assert!(!disabled.exists());
    }

    #[test]
    fn test_injector_artifact_exists_detects_disabled_file() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("version.dll");
        let disabled = dir.path().join("version.dll.disabled");

        std::fs::write(&disabled, b"disabled").unwrap();

        assert!(injector_artifact_exists(&active));
    }

    #[test]
    fn test_restore_disabled_injector_file_reenables_existing_artifact() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("version.dll");
        let disabled = dir.path().join("version.dll.disabled");

        std::fs::write(&disabled, b"disabled").unwrap();

        let restored = restore_disabled_injector_file(&active).unwrap();

        assert!(restored);
        assert!(active.exists());
        assert!(!disabled.exists());
        assert_eq!(std::fs::read(&active).unwrap(), b"disabled");
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"x").unwrap();
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> std::io::Cursor<Vec<u8>> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        let mut cursor = writer.finish().unwrap();
        cursor.set_position(0);
        cursor
    }

    fn tar_gz_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn parse_lovely_version_handles_tags() {
        assert_eq!(parse_lovely_version("0.10.0"), Some((0, 10, 0)));
        assert_eq!(parse_lovely_version("v0.10.0"), Some((0, 10, 0)));
        assert_eq!(parse_lovely_version(" 0.9.1 "), Some((0, 9, 1)));
        assert_eq!(parse_lovely_version("0.5.0-beta7"), Some((0, 5, 0)));
        assert_eq!(parse_lovely_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_lovely_version(""), None);
        assert_eq!(parse_lovely_version("abc"), None);
        assert_eq!(parse_lovely_version("1.2.3.4"), None);
    }

    #[test]
    fn supports_vanilla_flag_requires_0_9() {
        assert!(!supports_vanilla_flag(Some("0.8.0")));
        assert!(!supports_vanilla_flag(Some("0.5.0-beta7")));
        assert!(supports_vanilla_flag(Some("0.9.0")));
        assert!(supports_vanilla_flag(Some("v0.10.0")));
        assert!(supports_vanilla_flag(Some("1.0.0")));
        assert!(!supports_vanilla_flag(Some("garbage")));
        assert!(!supports_vanilla_flag(None));
    }

    #[test]
    fn version_from_latest_redirect_requires_tag_redirect() {
        let tag = "https://github.com/ethangreen-dev/lovely-injector/releases/tag/v0.10.0";
        assert_eq!(
            version_from_latest_redirect(true, tag),
            Some("0.10.0".to_string())
        );
        assert_eq!(
            version_from_latest_redirect(
                true,
                "/ethangreen-dev/lovely-injector/releases/tag/0.9.1?x=1"
            ),
            Some("0.9.1".to_string())
        );
        assert_eq!(version_from_latest_redirect(false, tag), None);
        assert_eq!(version_from_latest_redirect(true, ""), None);
        assert_eq!(
            version_from_latest_redirect(
                true,
                "https://github.com/ethangreen-dev/lovely-injector/releases/latest"
            ),
            None
        );
        assert_eq!(
            version_from_latest_redirect(true, "http://portal.example/login.html"),
            None
        );
        assert_eq!(
            version_from_latest_redirect(true, "https://github.com/x/releases/tag/latest"),
            None
        );
    }

    #[test]
    fn lovely_asset_url_pins_tag() {
        let pinned = "https://github.com/ethangreen-dev/lovely-injector/releases/download/v0.10.0/lovely-x86_64-pc-windows-msvc.zip";
        assert_eq!(lovely_asset_url(Some("0.10.0"), WIN_ASSET), pinned);
        assert_eq!(lovely_asset_url(Some("v0.10.0"), WIN_ASSET), pinned);
        let latest = "https://github.com/ethangreen-dev/lovely-injector/releases/latest/download/lovely-x86_64-pc-windows-msvc.zip";
        assert_eq!(lovely_asset_url(None, WIN_ASSET), latest);
        assert_eq!(lovely_asset_url(Some("  "), WIN_ASSET), latest);
    }

    #[test]
    fn select_injector_entry_prefers_winmm() {
        assert_eq!(select_injector_entry(&["winmm.dll"]), Some("winmm.dll"));
        assert_eq!(select_injector_entry(&["version.dll"]), Some("version.dll"));
        assert_eq!(
            select_injector_entry(&["version.dll", "winmm.dll"]),
            Some("winmm.dll")
        );
        assert_eq!(select_injector_entry(&["sub/winmm.dll"]), Some("winmm.dll"));
        assert_eq!(
            select_injector_entry(&["sub\\WINMM.DLL"]),
            Some("winmm.dll")
        );
        assert_eq!(
            select_injector_entry(&["notversion.dll.txt", "readme", "foo_version.dll"]),
            None
        );
        assert_eq!(select_injector_entry(&[]), None);
    }

    #[test]
    fn merge_wine_dll_overrides_appends_only_when_missing() {
        assert_eq!(merge_wine_dll_overrides(None, "winmm"), "winmm=n,b");
        assert_eq!(merge_wine_dll_overrides(Some(""), "winmm"), "winmm=n,b");
        assert_eq!(
            merge_wine_dll_overrides(Some("dxgi=n"), "winmm"),
            "dxgi=n;winmm=n,b"
        );
        assert_eq!(
            merge_wine_dll_overrides(Some("dxgi=n;"), "winmm"),
            "dxgi=n;winmm=n,b"
        );
        assert_eq!(
            merge_wine_dll_overrides(Some("winmm=n,b"), "winmm"),
            "winmm=n,b"
        );
        assert_eq!(
            merge_wine_dll_overrides(Some("version,winmm=n,b"), "winmm"),
            "version,winmm=n,b"
        );
        assert_eq!(
            merge_wine_dll_overrides(Some("winmm.dll=n,b"), "winmm"),
            "winmm.dll=n,b"
        );
        assert_eq!(
            merge_wine_dll_overrides(Some("version=n,b"), "winmm"),
            "version=n,b;winmm=n,b"
        );
        assert_eq!(
            merge_wine_dll_overrides(Some("version=n,b"), "version"),
            "version=n,b"
        );
        // A substring of another DLL name is not a match
        assert_eq!(
            merge_wine_dll_overrides(Some("winmmbase=n"), "winmm"),
            "winmmbase=n;winmm=n,b"
        );
    }

    #[test]
    fn detect_win_injector_states() {
        let dir = tempdir().unwrap();
        let p = dir.path();
        assert_eq!(detect_win_injector(p), WinInjectorState::None);
        assert!(!win_injector_installed(p));

        touch(&p.join("version.dll.disabled"));
        assert_eq!(detect_win_injector(p), WinInjectorState::Legacy);
        std::fs::remove_file(p.join("version.dll.disabled")).unwrap();

        touch(&p.join("version.dll"));
        assert_eq!(detect_win_injector(p), WinInjectorState::Legacy);
        assert!(win_injector_installed(p));

        touch(&p.join("winmm.dll"));
        assert_eq!(detect_win_injector(p), WinInjectorState::Both);

        std::fs::remove_file(p.join("version.dll")).unwrap();
        assert_eq!(detect_win_injector(p), WinInjectorState::Current);

        std::fs::rename(p.join("winmm.dll"), p.join("winmm.dll.disabled")).unwrap();
        assert_eq!(detect_win_injector(p), WinInjectorState::Current);
    }

    #[test]
    fn active_legacy_win_injector_ignores_disabled() {
        let dir = tempdir().unwrap();
        touch(&dir.path().join("version.dll.disabled"));
        assert_eq!(active_legacy_win_injector(dir.path()), None);
        touch(&dir.path().join("version.dll"));
        assert_eq!(
            active_legacy_win_injector(dir.path()),
            Some(dir.path().join("version.dll"))
        );
    }

    #[test]
    fn remove_legacy_win_injectors_keeps_winmm_and_is_idempotent() {
        let dir = tempdir().unwrap();
        let p = dir.path();
        for name in [
            "winmm.dll",
            "version.dll",
            "version.dll.disabled",
            "Balatro.exe",
            "dwmapi.dll",
        ] {
            touch(&p.join(name));
        }

        let mut removed = remove_legacy_win_injectors(p).unwrap();
        removed.sort();
        assert_eq!(
            removed,
            vec![p.join("version.dll"), p.join("version.dll.disabled")]
        );
        assert!(p.join("winmm.dll").exists());
        assert!(p.join("Balatro.exe").exists());
        assert!(p.join("dwmapi.dll").exists());
        assert_eq!(detect_win_injector(p), WinInjectorState::Current);

        assert!(remove_legacy_win_injectors(p).unwrap().is_empty());
    }

    #[test]
    fn plan_cached_win_injector_prefers_winmm() {
        let cache = tempdir().unwrap();
        let game = tempdir().unwrap();
        let plan = || plan_cached_win_injector(cache.path(), game.path());

        assert_eq!(plan(), CachedInjectorPlan::Download);

        touch(&game.path().join("version.dll"));
        assert_eq!(plan(), CachedInjectorPlan::SeedFromGame("version.dll"));

        touch(&cache.path().join("version.dll"));
        assert_eq!(plan(), CachedInjectorPlan::Cached("version.dll"));

        // A newer winmm.dll anywhere wins over a cached legacy DLL
        touch(&game.path().join("winmm.dll"));
        assert_eq!(plan(), CachedInjectorPlan::SeedFromGame("winmm.dll"));

        touch(&cache.path().join("winmm.dll"));
        assert_eq!(plan(), CachedInjectorPlan::Cached("winmm.dll"));
    }

    #[test]
    fn extract_injector_from_zip_installs_winmm_then_cleanup_removes_legacy() {
        let dir = tempdir().unwrap();
        let p = dir.path();
        touch(&p.join("version.dll"));
        std::fs::write(p.join("winmm.dll"), b"old").unwrap();

        let zip = zip_with(&[("winmm.dll", b"new lovely")]);
        let name = extract_injector_from_zip(zip, p).unwrap();
        assert_eq!(name, WIN_INJECTOR_DLL);
        assert_eq!(std::fs::read(p.join("winmm.dll")).unwrap(), b"new lovely");
        assert!(!p.join("winmm.dll.bmm-tmp").exists());

        remove_legacy_win_injectors(p).unwrap();
        assert!(!p.join("version.dll").exists());
        assert!(p.join("winmm.dll").exists());
    }

    #[test]
    fn extract_injector_from_zip_supports_legacy_layout() {
        let dir = tempdir().unwrap();
        let zip = zip_with(&[("lovely/version.dll", b"legacy")]);
        let name = extract_injector_from_zip(zip, dir.path()).unwrap();
        assert_eq!(name, "version.dll");
        assert_eq!(
            std::fs::read(dir.path().join("version.dll")).unwrap(),
            b"legacy"
        );
    }

    #[test]
    fn extract_injector_from_zip_without_dll_keeps_existing_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("winmm.dll"), b"keep").unwrap();
        let zip = zip_with(&[("README.md", b"hi")]);
        let err = extract_injector_from_zip(zip, dir.path()).unwrap_err();
        assert!(err.to_string().contains("winmm.dll or version.dll"));
        assert_eq!(
            std::fs::read(dir.path().join("winmm.dll")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn extract_file_from_tar_gz_replaces_target() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(MAC_DYLIB), b"old").unwrap();
        let archive = tar_gz_with(&[
            ("run_lovely_macos.sh", b"#!/bin/sh"),
            ("liblovely.dylib", b"new dylib"),
        ]);

        let path = extract_file_from_tar_gz(archive.as_slice(), MAC_DYLIB, dir.path(), Some(0o755))
            .unwrap();
        assert_eq!(path, dir.path().join(MAC_DYLIB));
        assert_eq!(std::fs::read(&path).unwrap(), b"new dylib");
        assert!(!dir.path().join("run_lovely_macos.sh").exists());
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn extract_file_from_tar_gz_missing_entry_keeps_existing_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(MAC_DYLIB), b"old").unwrap();
        let archive = tar_gz_with(&[("liblovely.so", b"linux")]);
        assert!(extract_file_from_tar_gz(archive.as_slice(), MAC_DYLIB, dir.path(), None).is_err());
        assert_eq!(std::fs::read(dir.path().join(MAC_DYLIB)).unwrap(), b"old");
    }

    #[test]
    fn copy_file_atomically_overwrites_destination() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.dll");
        let dest = dir.path().join("dest.dll");
        std::fs::write(&src, b"new").unwrap();
        std::fs::write(&dest, b"old").unwrap();
        copy_file_atomically(&src, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert!(!dir.path().join("dest.dll.bmm-tmp").exists());

        let missing = dir.path().join("missing.dll");
        assert!(copy_file_atomically(&missing, &dest).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
    }

    #[test]
    fn installed_win_injector_name_prefers_winmm() {
        let dir = tempdir().unwrap();
        assert_eq!(installed_win_injector_name(dir.path()), "winmm.dll");
        touch(&dir.path().join("version.dll.disabled"));
        assert_eq!(installed_win_injector_name(dir.path()), "version.dll");
        touch(&dir.path().join("winmm.dll"));
        assert_eq!(installed_win_injector_name(dir.path()), "winmm.dll");
    }

    #[test]
    fn toggle_winmm_injector_roundtrip() {
        let dir = tempdir().unwrap();
        let active = dir.path().join(WIN_INJECTOR_DLL);
        let disabled = disabled_injector_path(&active).unwrap();
        touch(&active);
        toggle_injector_file(&active, &disabled, false).unwrap();
        assert!(!active.exists() && disabled.exists());
        assert!(injector_artifact_exists(&active));
        assert!(restore_disabled_injector_file(&active).unwrap());
        assert!(active.exists() && !disabled.exists());
    }

    #[tokio::test]
    #[ignore = "downloads real Lovely release assets"]
    async fn real_v0_10_assets_extract() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("version.dll"), b"old").unwrap();

        let zip = download_asset(Some("0.10.0"), WIN_ASSET).await.unwrap();
        let name = extract_injector_from_zip(std::io::Cursor::new(zip), dir.path()).unwrap();
        assert_eq!(name, WIN_INJECTOR_DLL);
        remove_legacy_win_injectors(dir.path()).unwrap();
        assert_eq!(detect_win_injector(dir.path()), WinInjectorState::Current);

        let tarball = download_asset(Some("v0.10.0"), "lovely-aarch64-apple-darwin.tar.gz")
            .await
            .unwrap();
        let dylib =
            extract_file_from_tar_gz(tarball.as_ref(), MAC_DYLIB, dir.path(), Some(0o755)).unwrap();
        assert!(std::fs::metadata(dylib).unwrap().len() > 1_000_000);
    }

    #[test]
    fn remove_injector_artifacts_removes_both_variants() {
        let dir = tempdir().unwrap();
        let active = dir.path().join(WIN_INJECTOR_DLL);
        touch(&active);
        touch(&dir.path().join("winmm.dll.disabled"));
        assert_eq!(remove_injector_artifacts(&active).unwrap().len(), 2);
        assert!(!injector_artifact_exists(&active));
        assert!(remove_injector_artifacts(&active).unwrap().is_empty());
    }
}
