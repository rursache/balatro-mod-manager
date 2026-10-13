//! Binary cache for remote mod index data.
//!
//! This module provides functions to save and load the mod catalog from a
//! compressed binary cache file. The cache is stored using gzip-compressed
//! bincode serialization for efficient storage and fast loading.
//!
//! # Cache Location
//!
//! The cache file is stored in the platform's cache directory under
//! `balatro-mod-manager/mods.cache.bin.gz`. On Flatpak, it uses the
//! sandboxed cache path.
//!
//! # Cache Expiration
//!
//! The cache has a TTL of 15 minutes, after which it's considered stale.

use crate::errors::AppError;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use serde_repr::*;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_DURATION: u64 = 15 * 60; // 15 minutes in seconds
/// Bumped to 2 when the catalog moved off the BMI server, so caches holding `bmi://` URLs are dropped
const MODS_CACHE_FORMAT: u32 = 2;

fn flatpak_cache_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join(".var/app/io.balatro.ModManager/cache/balatro-mod-manager/mods.cache.bin.gz")
    })
}

fn cache_mtime(path: &PathBuf) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[derive(Serialize, Deserialize, Debug)]
struct CacheHeader {
    version: u32,
    timestamp: u64,
}

#[derive(Serialize, Deserialize)]
struct ModCache {
    header: CacheHeader,
    mods: Vec<Mod>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Clone)]
pub struct Mod {
    pub title: String,
    pub description: String,
    pub image: String,
    #[serde(rename = "categories")]
    pub categories: Vec<Category>,
    #[serde(rename = "colors")]
    pub colors: ColorPair,
    pub installed: bool,
    #[serde(rename = "requires_steamodded")]
    pub requires_steamodded: bool,
    #[serde(rename = "requires_talisman")]
    pub requires_talisman: bool,
    pub publisher: String,
    pub repo: String,
    #[serde(rename = "downloadURL")]
    pub download_url: String,
    pub folderName: Option<String>,
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ColorPair {
    pub color1: String,
    pub color2: String,
}

#[derive(Serialize_repr, Deserialize_repr, Debug, Clone, Copy, PartialEq)]
#[repr(u32)]
pub enum Category {
    Content = 0,
    Joker = 1,
    QualityOfLife = 2,
    Technical = 3,
    Miscellaneous = 4,
    ResourcePacks = 5,
    API = 6,
}

impl From<std::string::String> for Category {
    fn from(value: std::string::String) -> Self {
        match value.as_str() {
            "Content" => Category::Content,
            "Joker" => Category::Joker,
            "Quality of Life" => Category::QualityOfLife,
            "Technical" => Category::Technical,
            "Miscellaneous" => Category::Miscellaneous,
            "Resource Packs" => Category::ResourcePacks,
            "API" => Category::API,
            _ => panic!("Invalid category: {value}"),
        }
    }
}

impl From<i32> for Category {
    fn from(value: i32) -> Self {
        match value {
            0 => Category::Content,
            1 => Category::Joker,
            2 => Category::QualityOfLife,
            3 => Category::Technical,
            4 => Category::Miscellaneous,
            5 => Category::ResourcePacks,
            6 => Category::API,
            _ => panic!("Invalid category index: {value}"),
        }
    }
}

pub fn clear_cache() -> Result<(), AppError> {
    let cache_dir = dirs::cache_dir()
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("cache directory")))?
        .join("balatro-mod-manager");
    let mut targets = vec![cache_dir.join("mods.cache.bin.gz")];
    if let Some(flatpak_path) = flatpak_cache_path() {
        targets.push(flatpak_path);
    }

    // Delete mods cache(s)
    for mods_cache in targets {
        if mods_cache.exists() {
            std::fs::remove_file(&mods_cache).map_err(|e| AppError::FileWrite {
                path: mods_cache,
                source: e.to_string(),
            })?;
        }
    }

    // Delete version caches
    [
        "versions-steamodded.cache.bin.gz",
        "versions-talisman.cache.bin.gz",
    ]
    .into_iter()
    .try_for_each(|file| {
        let path = cache_dir.join(file);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| AppError::FileWrite {
                path: path.clone(),
                source: e.to_string(),
            })
        } else {
            Ok(())
        }
    })
}

pub fn save_versions_cache(mod_type: &str, versions: &[String]) -> Result<(), AppError> {
    let mut path = dirs::cache_dir()
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("cache directory")))?
        .join("balatro-mod-manager");

    std::fs::create_dir_all(&path).map_err(|e| AppError::DirCreate {
        path: path.clone(),
        source: e.to_string(),
    })?;

    path.push(format!("versions-{mod_type}.cache.bin.gz"));

    let file = File::create(&path).map_err(|e| AppError::FileWrite {
        path: path.clone(),
        source: e.to_string(),
    })?;

    let mut encoder = GzEncoder::new(file, Compression::default());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AppError::SystemTime(e.to_string()))?
        .as_secs();

    let cache = VersionCache {
        header: CacheHeader {
            version: 1,
            timestamp,
        },
        versions: versions.to_vec(),
    };

    // Use bincode 2.0 for serialization
    let config = bincode::config::standard();
    let encoded =
        bincode::serde::encode_to_vec(&cache, config).map_err(|e| AppError::Serialization {
            format: "bincode".into(),
            source: e.to_string(),
        })?;

    encoder
        .write_all(&encoded)
        .map_err(|e| AppError::FileWrite {
            path: path.clone(),
            source: e.to_string(),
        })?;

    // Ensure gzip footer is written and file is flushed to disk
    let file = encoder.finish().map_err(|e| AppError::FileWrite {
        path: path.clone(),
        source: e.to_string(),
    })?;
    file.sync_all().map_err(|e| AppError::FileWrite {
        path,
        source: e.to_string(),
    })?;

    Ok(())
}

pub fn load_versions_cache(mod_type: &str) -> Result<Option<Vec<String>>, AppError> {
    let path = dirs::cache_dir()
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("cache directory")))?
        .join("balatro-mod-manager")
        .join(format!("versions-{mod_type}.cache.bin.gz"));

    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };

    // Stream decompress directly from file instead of loading entire file to memory
    let mut decoder = GzDecoder::new(BufReader::new(file));
    let mut decompressed = Vec::new();

    // Decompress the data
    if let Err(e) = decoder.read_to_end(&mut decompressed) {
        return Err(AppError::FileRead {
            path: path.clone(),
            source: e.to_string(),
        });
    }

    // Deserialize using bincode 2.0
    let config = bincode::config::standard();
    let (cache, _): (VersionCache, _) =
        match bincode::serde::decode_from_slice(&decompressed, config) {
            Ok(result) => result,
            Err(_) => return Ok(None),
        };

    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AppError::SystemTime(e.to_string()))?
        .as_secs();

    if current_time - cache.header.timestamp > CACHE_DURATION {
        return Ok(None);
    }

    Ok(Some(cache.versions))
}

#[derive(Serialize, Deserialize)]
struct VersionCache {
    header: CacheHeader,
    versions: Vec<String>,
}

pub fn get_cache_path() -> Result<PathBuf, AppError> {
    // Primary cache location (used when writing)
    let mut path = dirs::cache_dir()
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("cache directory")))?
        .join("balatro-mod-manager");

    std::fs::create_dir_all(&path).map_err(|e| AppError::DirCreate {
        path: path.clone(),
        source: e.to_string(),
    })?;

    path.push("mods.cache.bin.gz");
    Ok(path)
}

fn select_cache_path_for_read() -> Result<PathBuf, AppError> {
    let primary = dirs::cache_dir()
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("cache directory")))?
        .join("balatro-mod-manager")
        .join("mods.cache.bin.gz");

    let candidates = flatpak_cache_path()
        .into_iter()
        .chain(std::iter::once(primary.clone()))
        .filter(|p| p.exists());

    let selected = candidates.max_by_key(cache_mtime).unwrap_or(primary);

    Ok(selected)
}

pub fn save_cache(mods: &[Mod]) -> Result<(), AppError> {
    let path = get_cache_path()?;
    let file = File::create(&path).map_err(|e| AppError::FileWrite {
        path: path.clone(),
        source: e.to_string(),
    })?;

    let mut encoder = GzEncoder::new(file, Compression::default());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AppError::SystemTime(e.to_string()))?
        .as_secs();

    let encoded = encode_mod_cache(mods, MODS_CACHE_FORMAT, timestamp)?;

    encoder
        .write_all(&encoded)
        .map_err(|e| AppError::FileWrite {
            path: path.clone(),
            source: e.to_string(),
        })?;

    // Ensure gzip footer is written and file is flushed to disk
    let file = encoder.finish().map_err(|e| AppError::FileWrite {
        path: path.clone(),
        source: e.to_string(),
    })?;
    file.sync_all().map_err(|e| AppError::FileWrite {
        path,
        source: e.to_string(),
    })?;

    Ok(())
}

pub fn load_cache() -> Result<Option<(Vec<Mod>, u64)>, AppError> {
    let path = select_cache_path_for_read()?;
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(AppError::FileRead {
                path: path.clone(),
                source: e.to_string(),
            });
        }
    };

    // Stream decompress directly from file instead of loading entire file to memory
    let mut decoder = GzDecoder::new(BufReader::new(file));
    let mut decompressed = Vec::new();

    // Decompress the data
    if let Err(e) = decoder.read_to_end(&mut decompressed) {
        return Err(AppError::FileRead {
            path: path.clone(),
            source: e.to_string(),
        });
    }

    // Keep using stale cache data; callers can decide if they want to refresh based on timestamp.
    // This prevents losing catalog lookups when offline or when a different install (e.g., Flatpak)
    // owns the freshest cache.
    Ok(decode_mod_cache(&decompressed))
}

fn encode_mod_cache(mods: &[Mod], version: u32, timestamp: u64) -> Result<Vec<u8>, AppError> {
    let cache = ModCache {
        header: CacheHeader { version, timestamp },
        mods: mods.to_vec(),
    };
    bincode::serde::encode_to_vec(&cache, bincode::config::standard()).map_err(|e| {
        AppError::Serialization {
            format: "bincode".into(),
            source: e.to_string(),
        }
    })
}

fn decode_mod_cache(bytes: &[u8]) -> Option<(Vec<Mod>, u64)> {
    let (cache, _): (ModCache, _) =
        bincode::serde::decode_from_slice(bytes, bincode::config::standard()).ok()?;
    if cache.header.version != MODS_CACHE_FORMAT {
        return None;
    }
    Some((cache.mods, cache.header.timestamp))
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

    fn with_temp_cache<T>(test: impl FnOnce(PathBuf) -> T) -> T {
        let _lock = crate::ENV_LOCK.lock().unwrap();
        let temp_dir = tempdir().unwrap();
        let original_cache = std::env::var_os("XDG_CACHE_HOME");
        let original_home = std::env::var_os("HOME");

        set_var("XDG_CACHE_HOME", temp_dir.path());
        // Ensure HOME points at temp so flatpak cache fallbacks don't hit real user data.
        set_var("HOME", temp_dir.path());
        let result = test(temp_dir.path().to_path_buf());

        // restore env
        match original_cache {
            Some(val) => set_var("XDG_CACHE_HOME", val),
            None => remove_var("XDG_CACHE_HOME"),
        }
        match original_home {
            Some(val) => set_var("HOME", val),
            None => remove_var("HOME"),
        }

        result
    }

    fn sample_mod(url: &str) -> Mod {
        Mod {
            title: "Sample".into(),
            description: String::new(),
            image: String::new(),
            categories: vec![Category::API],
            colors: ColorPair {
                color1: "#fff".into(),
                color2: "#000".into(),
            },
            installed: false,
            requires_steamodded: false,
            requires_talisman: false,
            publisher: "x".into(),
            repo: String::new(),
            download_url: url.into(),
            folderName: None,
            version: None,
        }
    }

    #[test]
    fn load_cache_rejects_v1_header() {
        let bytes = encode_mod_cache(&[sample_mod("bmi://sample")], 1, 10).unwrap();
        assert!(decode_mod_cache(&bytes).is_none());
    }

    #[test]
    fn mod_cache_roundtrip_at_current_format() {
        let bytes =
            encode_mod_cache(&[sample_mod("https://e.com/a.zip")], MODS_CACHE_FORMAT, 10).unwrap();
        let (mods, ts) = decode_mod_cache(&bytes).expect("decodes");
        assert_eq!(ts, 10);
        assert_eq!(mods[0].download_url, "https://e.com/a.zip");
    }

    #[test]
    fn decode_mod_cache_rejects_garbage() {
        assert!(decode_mod_cache(b"nope").is_none());
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "macOS sandbox sometimes disrupts cache readback in CI"
    )]
    fn test_mod_cache_lifecycle() -> Result<(), AppError> {
        with_temp_cache(|_| {
            let test_mod = Mod {
                title: "Test Mod".into(),
                description: "Test Description".into(),
                image: "test.png".into(),
                categories: vec![Category::Content],
                colors: ColorPair {
                    color1: "#fff".into(),
                    color2: "#000".into(),
                },
                installed: false,
                requires_steamodded: false,
                requires_talisman: false,
                publisher: "Test".into(),
                repo: "test/test".into(),
                download_url: "https://test.com/mod.zip".into(),
                folderName: None,
                version: None,
            };

            // Use a single-element slice referencing the value to avoid cloning
            save_cache(std::slice::from_ref(&test_mod))?;
            let loaded = load_cache()?.expect("Should load cache");

            assert_eq!(loaded.0.len(), 1);
            assert_eq!(loaded.0[0].title, "Test Mod");
            Ok(())
        })
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "macOS sandbox sometimes disrupts cache readback in CI"
    )]
    fn test_versions_cache_roundtrip() -> Result<(), AppError> {
        with_temp_cache(|_| {
            let versions = vec!["1.0.0".into(), "1.1.0".into()];
            save_versions_cache("steamodded", &versions)?;
            let loaded = load_versions_cache("steamodded")?.expect("versions cache present");
            assert_eq!(loaded, versions);
            Ok(())
        })
    }
}
