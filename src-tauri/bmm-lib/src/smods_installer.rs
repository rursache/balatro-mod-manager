//! Steamodded and Talisman mod framework installer.
//!
//! This module provides functionality to install, uninstall, and manage
//! the Steamodded and Talisman mod frameworks for Balatro. These frameworks
//! are required dependencies for many community mods.
//!
//! # Features
//!
//! - Fetch available versions from GitHub releases
//! - Install specific versions or the latest from the default branch
//! - Detect existing installations
//! - Uninstall mod frameworks
//! - Rate limiting for GitHub API requests
//!
//! # Example
//!
//! ```ignore
//! let installer = ModInstaller::new(ModType::Steamodded);
//! let versions = installer.get_available_versions().await?;
//! installer.install_version(&versions[0]).await?;
//! ```

use anyhow::{Context, Result, anyhow};
use log::info;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tokio::fs as tokio_fs;
use tokio::time::sleep;
use zip::ZipArchive;

use crate::rate_limiter::{
    RateLimitResult, check_github_rate_limit, parse_rate_limit_headers, update_github_rate_limit,
};

#[derive(Debug, Serialize, Deserialize)]
struct Release {
    tag_name: String,
    name: String,
    assets: Vec<ReleaseAsset>,
    published_at: String,
    html_url: String,
    prerelease: bool,
    zipball_url: String, // Add this field
}

#[derive(Debug, Clone)]
pub enum ModType {
    Steamodded,
    Talisman,
}

impl std::fmt::Display for ModType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ModType::Steamodded => write!(f, "Steamodded"),
            ModType::Talisman => write!(f, "Talisman"),
        }
    }
}

impl ModType {
    fn get_repo_url(&self) -> &str {
        match self {
            ModType::Steamodded => "Steamodded/smods",
            ModType::Talisman => "MathIsFun0/Talisman",
        }
    }

    pub async fn check_installation(&self) -> bool {
        match self {
            ModType::Steamodded => {
                let installer = ModInstaller::new(ModType::Steamodded);
                installer.is_installed()
            }
            ModType::Talisman => {
                let installer = ModInstaller::new(ModType::Talisman);
                installer.is_installed()
            }
        }
    }
}

/// True when `path` is a directory that holds a Steamodded install, judged by
/// its own files (manifest name or the smods source layout), never by folder name
pub fn is_steamodded_install(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    if let Ok(raw) = fs::read_to_string(path.join("manifest.json"))
        && let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&raw)
        && manifest
            .get("name")
            .and_then(|n| n.as_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("steamodded"))
    {
        return true;
    }

    let has_core =
        path.join("src").join("core.lua").is_file() || path.join("core").join("core.lua").is_file();
    has_core && path.join("version.lua").is_file() && path.join("lovely").is_dir()
}

fn has_steamodded_dir_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("smods") || lower.starts_with("steamodded")
}

/// Lists folders directly inside `mods_dir` that are named like Steamodded
/// (`smods*`, `Steamodded*`) and contain a Steamodded install
pub fn find_steamodded_dirs(mods_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(mods_dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| e.file_name().to_str().is_some_and(has_steamodded_dir_name))
        .map(|e| e.path())
        .filter(|p| is_steamodded_install(p))
        .collect();
    found.sort();
    found
}

#[derive(Debug, Serialize, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}
pub struct ModInstaller {
    client: reqwest::Client,
    pub mod_type: ModType,
}

impl ModInstaller {
    pub fn new(mod_type: ModType) -> Self {
        Self {
            client: crate::http::shared_client(),
            mod_type,
        }
    }

    pub fn is_installed(&self) -> bool {
        match self.get_installation_path() {
            Ok(path) => fs::read_dir(path)
                .map(|mut entries| {
                    entries.any(|e| {
                        e.ok()
                            .map(|entry| {
                                let binding = entry.file_name();
                                let name = binding.to_str().unwrap_or("");
                                match self.mod_type {
                                    ModType::Steamodded => name.starts_with("Steamodded-smods-"),
                                    ModType::Talisman => name.contains("Talisman"),
                                }
                            })
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    fn get_installation_path(&self) -> Result<PathBuf> {
        let mod_path = crate::local_mod_detection::resolve_mods_dir_path()
            .map_err(|e| anyhow!(e))
            .context("Failed to resolve mods directory")?;
        fs::create_dir_all(&mod_path).with_context(|| {
            format!("Failed to create mods directory at {}", mod_path.display())
        })?;
        Ok(mod_path)
    }

    pub async fn get_latest_release(&self) -> Result<String> {
        let versions = self.get_available_versions().await?;
        Ok(versions[0].clone())
    }

    pub async fn get_available_versions(&self) -> Result<Vec<String>> {
        // Check rate limit before making request
        match check_github_rate_limit() {
            RateLimitResult::Proceed => {}
            RateLimitResult::Wait(delay) => {
                log::debug!(
                    "Rate limit backpressure: waiting {:?} before GitHub API call",
                    delay
                );
                sleep(delay).await;
            }
            RateLimitResult::Limited(until) => {
                log::warn!("GitHub API rate limited, reset in {:?}", until);
                return Err(anyhow!(
                    "GitHub API rate limit exceeded. Please wait {} seconds.",
                    until.as_secs()
                ));
            }
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Balatro-Mod-Manager/1.0"),
        );
        headers.insert(
            "Accept",
            HeaderValue::from_static("application/vnd.github.v3+json"),
        );

        let url = format!(
            "https://api.github.com/repos/{}/releases",
            self.mod_type.get_repo_url()
        );

        log::debug!("Fetching {} releases from: {}", self.mod_type, url);

        let response = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| {
                log::error!("Failed to fetch {} releases: {}", self.mod_type, e);
                e
            })
            .context("Failed to fetch Steamodded releases")?;

        // Update rate limit state from response headers
        let (remaining, reset) = parse_rate_limit_headers(response.headers());
        let is_429 = response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS;
        update_github_rate_limit(remaining, reset, is_429);

        if !response.status().is_success() {
            let status = response.status();
            log::error!(
                "GitHub API error for {}: status={}, rate_limit_remaining={:?}",
                self.mod_type,
                status,
                remaining
            );
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "GitHub API error: {} - {}",
                status,
                error_text
            ));
        }

        log::debug!("Successfully fetched {} releases", self.mod_type);

        let releases: Vec<Release> = response
            .json()
            .await
            .context("Failed to decode Steamodded releases")?;

        let mut versions: Vec<String> = releases
            .into_iter()
            .filter(|r| !r.prerelease) // Filter out pre-releases
            .map(|r| r.tag_name)
            .collect();

        versions.sort_by(|a, b| b.cmp(a));

        Ok(versions)
    }

    async fn get_default_branch_download_url(&self) -> Result<String> {
        use reqwest::header::{ACCEPT, USER_AGENT};

        let api_url = format!(
            "https://api.github.com/repos/{}",
            self.mod_type.get_repo_url()
        );
        log::debug!(
            "Fetching default branch for {}",
            self.mod_type.get_repo_url()
        );

        let response = self
            .client
            .get(&api_url)
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header(USER_AGENT, "Balatro-Mod-Manager/1.0")
            .send()
            .await
            .map_err(|e| {
                log::error!("Failed to fetch repo info for {}: {}", self.mod_type, e);
                e
            })?
            .error_for_status()
            .map_err(|e| {
                log::error!("GitHub API error for {}: {}", self.mod_type, e);
                e
            })?;

        let body = response.json::<serde_json::Value>().await?;
        body["default_branch"]
            .as_str()
            .map(|b| {
                format!(
                    "https://github.com/{}/archive/refs/heads/{b}.zip",
                    self.mod_type.get_repo_url(),
                )
            })
            .ok_or(anyhow::anyhow!(
                "repo {} has no default branch",
                self.mod_type.get_repo_url()
            ))
    }

    pub async fn install_version(&self, version: &str) -> Result<String> {
        let installation_path = self.get_installation_path()?;
        let url = match version {
            "newest" => self.get_default_branch_download_url().await?,
            _ => format!(
                "https://github.com/{}/archive/refs/tags/{}.zip",
                self.mod_type.get_repo_url(),
                version
            ),
        };

        info!(
            "Installing {:?} version {} to {:?}",
            self.mod_type, version, installation_path
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Balatro-Mod-Manager/1.0"),
        );

        // Download the zip file
        log::debug!("Downloading {} from: {}", self.mod_type, url);
        let response = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| {
                log::error!("Failed to download {}: {}", self.mod_type, e);
                e
            })?;

        if !response.status().is_success() {
            log::error!(
                "Download failed for {}: HTTP {}",
                self.mod_type,
                response.status()
            );
            return Err(anyhow!("Download failed: HTTP {}", response.status()));
        }

        let bytes = response.bytes().await?;
        log::debug!("Downloaded {} bytes for {}", bytes.len(), self.mod_type);

        // Create temp directory
        let temp_dir = installation_path.join(format!(
            "temp_{}",
            match self.mod_type {
                ModType::Steamodded => "smods",
                ModType::Talisman => "talisman",
            }
        ));
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)?;
        }
        fs::create_dir_all(&temp_dir)?;

        // Extract to temp directory
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)?;
        archive.extract(&temp_dir)?;

        // Find the root directory name (GitHub format: Steamodded-smods-commitHash)
        let root_dir = fs::read_dir(&temp_dir)?
            .next()
            .ok_or(anyhow!("Empty archive"))??
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("Invalid directory name"))?;

        // Move to final location
        let final_dir = match self.mod_type {
            ModType::Steamodded => installation_path.join(&root_dir),
            ModType::Talisman => installation_path.join("Talisman"),
        };
        if final_dir.exists() {
            fs::remove_dir_all(&final_dir)?;
        }
        fs::rename(temp_dir.join(&root_dir), &final_dir)?;

        // Cleanup
        fs::remove_dir_all(temp_dir)?;

        info!(
            "Successfully installed {:?} version {} to {:?}",
            self.mod_type, version, final_dir
        );

        Ok(final_dir.to_string_lossy().to_string())
    }

    pub async fn uninstall(&self) -> Result<()> {
        let installation_path = self.get_installation_path()?;
        let mods_dir = installation_path;

        if !mods_dir.exists() {
            info!("Mods directory not found");
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(&mods_dir).await?;
        let mut found = false;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir()
                && let Some(dir_name) = path.file_name().and_then(|n| n.to_str())
            {
                // Match both Steamodded and specific commit formats
                if dir_name.to_lowercase().contains("steamodded")
                    || dir_name.starts_with("smods-")
                    || dir_name.starts_with("Steamodded-smods-")
                {
                    found = true;
                    info!("Removing mod directory: {path:?}");
                    tokio_fs::remove_dir_all(&path).await?;
                }
            }
        }

        if !found {
            info!("No Steamodded installations found");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mod_type_display() {
        assert_eq!(format!("{}", ModType::Steamodded), "Steamodded");
        assert_eq!(format!("{}", ModType::Talisman), "Talisman");
    }

    #[test]
    fn test_mod_type_get_repo_url() {
        assert_eq!(ModType::Steamodded.get_repo_url(), "Steamodded/smods");
        assert_eq!(ModType::Talisman.get_repo_url(), "MathIsFun0/Talisman");
    }

    #[test]
    fn test_version_sorting_descending() {
        // Simulates the sorting logic used in get_available_versions
        let mut versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "1.5.0".to_string(),
            "0.9.0".to_string(),
        ];
        versions.sort_by(|a, b| b.cmp(a));
        assert_eq!(
            versions,
            vec![
                "2.0.0".to_string(),
                "1.5.0".to_string(),
                "1.0.0".to_string(),
                "0.9.0".to_string()
            ]
        );
    }

    #[test]
    fn test_is_installed_with_no_dir() {
        // ModInstaller::is_installed returns false when the mods directory doesn't exist
        // This tests the fallback behavior
        let installer = ModInstaller::new(ModType::Steamodded);
        // We can't fully test is_installed without mocking the filesystem,
        // but we can verify the struct initializes correctly
        assert!(matches!(installer.mod_type, ModType::Steamodded));
    }

    #[test]
    fn test_mod_installer_new() {
        let steamodded = ModInstaller::new(ModType::Steamodded);
        assert!(matches!(steamodded.mod_type, ModType::Steamodded));

        let talisman = ModInstaller::new(ModType::Talisman);
        assert!(matches!(talisman.mod_type, ModType::Talisman));
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn is_steamodded_install_checks_files_not_names() {
        let td = tempfile::tempdir().unwrap();

        let by_manifest = td.path().join("anything");
        write(
            &by_manifest.join("manifest.json"),
            r#"{"name": "Steamodded", "version_number": "1.0.0"}"#,
        );
        assert!(is_steamodded_install(&by_manifest));

        let by_layout = td.path().join("smods-main");
        write(&by_layout.join("src").join("core.lua"), "");
        write(&by_layout.join("version.lua"), "return \"1.0.0\"");
        fs::create_dir_all(by_layout.join("lovely")).unwrap();
        assert!(is_steamodded_install(&by_layout));

        let other_manifest = td.path().join("Steamodded-addon");
        write(
            &other_manifest.join("manifest.json"),
            r#"{"name": "Cryptid"}"#,
        );
        assert!(!is_steamodded_install(&other_manifest));

        let empty = td.path().join("Steamodded");
        fs::create_dir_all(&empty).unwrap();
        assert!(!is_steamodded_install(&empty));
    }

    #[test]
    fn find_steamodded_dirs_detects_leftover_smods_folders() {
        let td = tempfile::tempdir().unwrap();
        let mods = td.path();
        let manifest = r#"{"name": "Steamodded"}"#;
        for name in [
            "smods-1.0.0-beta-1814a",
            "Steamodded",
            "Steamodded-smods-abc",
        ] {
            write(&mods.join(name).join("manifest.json"), manifest);
        }
        // Steamodded files under an unrelated name are not a leftover copy
        write(&mods.join("MyFork").join("manifest.json"), manifest);
        // Steamodded like name without Steamodded files
        fs::create_dir_all(mods.join("smods-backup-notes")).unwrap();
        write(&mods.join("smods-file.zip"), "");

        let found: Vec<String> = find_steamodded_dirs(mods)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            found,
            vec![
                "Steamodded",
                "Steamodded-smods-abc",
                "smods-1.0.0-beta-1814a"
            ]
        );
    }
}
