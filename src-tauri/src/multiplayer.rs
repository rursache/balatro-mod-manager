//! Multiplayer section: the official BalatroMultiplayer Thunderstore packages, which of the
//! two mutually exclusive tracks is installed, readiness checks and the launch environment
//!
//! Casual play only: nothing here sets or emulates BET_ variables or the ranked supervisor,
//! and mod files are never modified

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bmm_lib::database::{InstalledMod, SOURCE_THUNDERSTORE};
use bmm_lib::local_mod_detection::{parse_thunderstore_manifest_str, thunderstore_package_ref};
use serde::{Deserialize, Serialize};

use crate::thunderstore::{self, TsPackage};

/// Legacy standalone mod, internal id `Multiplayer`, plain TCP without native libraries
pub const LEGACY_PACKAGE: &str = "BalatroMultiplayer-Multiplayer";
/// Library the game modes build on, internal id `MultiplayerAPI`, MQTT over bundled OpenSSL
pub const API_PACKAGE: &str = "BalatroMultiplayer-MultiplayerAPI";
pub const LEGACY_MOD_ID: &str = "Multiplayer";
pub const API_MOD_ID: &str = "MultiplayerAPI";
/// Ranked play needs the team's own launcher
pub const BET_DOWNLOAD_URL: &str = "https://new.balatromp.com/download";
const BET_ENV_PREFIX: &str = "BET_";

const NETWORKING_DIR: &str = "networking";
pub const MACOS_LIBS: [&str; 2] = ["libssl.3.dylib", "libcrypto.3.dylib"];
pub const WINDOWS_LIBS: [&str; 2] = ["libssl-3-x64.dll", "libcrypto-3-x64.dll"];
pub const LINUX_LIBSSL: &str = "libssl.so.3";
/// Where the API looks for the system OpenSSL on Linux, besides the loader path
pub const LINUX_LIB_DIRS: [&str; 7] = [
    "/usr/lib",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/usr/lib64",
    "/usr/local/lib",
    "/lib",
    "/lib/x86_64-linux-gnu",
];
const REGISTRY_FILE: &str = "api/mod_registry/registry.lua";
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const BALATRO_APP_ID: &str = "2379780";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Track {
    Legacy,
    Api,
}

impl Track {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "legacy" => Ok(Self::Legacy),
            "api" => Ok(Self::Api),
            other => Err(format!("Unknown Multiplayer track '{other}'")),
        }
    }

    pub fn package(self) -> &'static str {
        match self {
            Self::Legacy => LEGACY_PACKAGE,
            Self::Api => API_PACKAGE,
        }
    }

    fn role(self) -> Role {
        match self {
            Self::Legacy => Role::Legacy,
            Self::Api => Role::Api,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Legacy,
    Api,
    Gamemode,
}

// ---------------------------------------------------------------------------
// Mods folder scan
// ---------------------------------------------------------------------------

/// A mod folder with Steamodded metadata or a Thunderstore manifest
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundMod {
    pub path: PathBuf,
    pub folder: String,
    /// Steamodded id from the mod's json metadata, empty when it has none
    pub id: String,
    pub version: Option<String>,
    pub dependencies: Vec<String>,
    /// `Namespace-Name` from manifest.json and a matching folder name
    pub package: Option<String>,
    pub package_version: Option<String>,
    pub disabled: bool,
}

fn json_strings(value: Option<&serde_json::Value>) -> Vec<String> {
    let mut out = Vec::new();
    let Some(serde_json::Value::Array(items)) = value else {
        return out;
    };
    for item in items {
        match item {
            serde_json::Value::String(s) => out.push(s.clone()),
            // Steamodded alternatives, any of them satisfies the dependency
            serde_json::Value::Array(alts) => {
                out.extend(alts.iter().filter_map(|a| a.as_str().map(str::to_string)))
            }
            _ => {}
        }
    }
    out
}

fn read_mod_folder(path: &Path) -> Option<FoundMod> {
    let folder = path.file_name()?.to_str()?.to_string();
    let mut json_files: Vec<PathBuf> = std::fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                && !p
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("manifest.json"))
        })
        .collect();
    json_files.sort();

    let mut found = FoundMod {
        path: path.to_path_buf(),
        folder: folder.clone(),
        id: String::new(),
        version: None,
        dependencies: Vec::new(),
        package: None,
        package_version: None,
        disabled: path.join(".lovelyignore").exists(),
    };
    for file in json_files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(value) =
            serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}'))
        else {
            continue;
        };
        let Some(id) = value.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        found.id = id.trim().to_string();
        found.version = value
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        found.dependencies = json_strings(value.get("dependencies"));
        break;
    }
    if let Some(manifest) = std::fs::read_to_string(path.join("manifest.json"))
        .ok()
        .and_then(|t| parse_thunderstore_manifest_str(&t))
    {
        let package = thunderstore_package_ref(&manifest, Some(&folder));
        found.package = package.full_name;
        found.package_version = package.version;
    }
    (!found.id.is_empty() || found.package.is_some()).then_some(found)
}

/// Mods with metadata directly inside `dir`
pub fn scan_mods_dir(dir: &Path) -> Vec<FoundMod> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<FoundMod> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
        })
        .filter_map(|p| read_mod_folder(&p))
        .collect();
    found.sort_by(|a, b| a.folder.cmp(&b.folder));
    found
}

/// Scans every Mods folder once, folders reachable through two paths count once
pub fn scan_mods_dirs(dirs: &[PathBuf]) -> Vec<FoundMod> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for dir in dirs {
        for m in scan_mods_dir(dir) {
            if seen.insert(canonical(&m.path)) {
                found.push(m);
            }
        }
    }
    found
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b || canonical(a) == canonical(b)
}

/// Mod id of a Steamodded dependency entry such as `MultiplayerAPI (>=1.0)`
fn dependency_id(dep: &str) -> &str {
    dep.trim()
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | '<' | '>' | '=' | '~'))
        .next()
        .unwrap_or("")
}

pub fn depends_on_api(found: &FoundMod) -> bool {
    found
        .dependencies
        .iter()
        .any(|d| dependency_id(d).eq_ignore_ascii_case(API_MOD_ID))
}

// ---------------------------------------------------------------------------
// Packages and game modes
// ---------------------------------------------------------------------------

fn package_depends_on_api(pkg: &TsPackage) -> bool {
    thunderstore::latest_active_version(pkg).is_some_and(|v| {
        v.dependencies.iter().any(|d| {
            thunderstore::parse_dependency(d)
                .is_some_and(|d| d.full_name().eq_ignore_ascii_case(API_PACKAGE))
        })
    })
}

/// Game modes are the official packages built on MultiplayerAPI, discovered from the listing
pub fn gamemode_packages(packages: &[TsPackage]) -> Vec<&TsPackage> {
    let mut modes: Vec<&TsPackage> = packages
        .iter()
        .filter(|p| thunderstore::is_multiplayer(p) && package_depends_on_api(p))
        .collect();
    modes.sort_by_key(|a| a.name.to_lowercase());
    modes
}

/// Resolves a game mode given as `Namespace-Name`, `ts:` reference or package name
pub fn resolve_gamemode<'a>(packages: &'a [TsPackage], key: &str) -> Option<&'a TsPackage> {
    let key = key.trim();
    let key = key
        .strip_prefix(crate::catalog::TS_URL_SCHEME)
        .or_else(|| key.strip_prefix(crate::catalog::TS_ID_PREFIX))
        .unwrap_or(key);
    gamemode_packages(packages)
        .into_iter()
        .find(|p| p.full_name.eq_ignore_ascii_case(key) || p.name.eq_ignore_ascii_case(key))
}

/// An official mode the installed API lists in its menu registry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryMode {
    pub id: String,
    pub name: String,
    pub coming_soon: bool,
}

fn lua_field(entry: &str, key: &str) -> Option<String> {
    let bytes = entry.as_bytes();
    let mut from = 0;
    while let Some(pos) = entry[from..].find(key) {
        let start = from + pos;
        let end = start + key.len();
        from = end;
        let boundary_before =
            start == 0 || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
        let rest = entry[end..].trim_start();
        if !boundary_before || !rest.starts_with('=') || rest.starts_with("==") {
            continue;
        }
        let value = rest[1..].trim_start();
        if let Some(quote) = value.chars().next().filter(|c| *c == '\'' || *c == '"') {
            let inner = &value[1..];
            return inner.find(quote).map(|e| inner[..e].to_string());
        }
        let token: String = value
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        return (!token.is_empty()).then_some(token);
    }
    None
}

/// Reads the `official_mods` table of the API's `registry.lua`, read only
pub fn parse_registry_modes(lua: &str) -> Vec<RegistryMode> {
    let Some(start) = lua.find("official_mods") else {
        return Vec::new();
    };
    let Some(open) = lua[start..].find('{').map(|i| start + i) else {
        return Vec::new();
    };
    let mut modes = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut entry_start = None;
    for (i, c) in lua[open..].char_indices() {
        let i = open + i;
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '{' => {
                depth += 1;
                if depth == 2 {
                    entry_start = Some(i + 1);
                }
            }
            '}' => {
                if depth == 2
                    && let Some(s) = entry_start.take()
                {
                    let entry = &lua[s..i];
                    if let Some(id) = lua_field(entry, "id") {
                        modes.push(RegistryMode {
                            name: lua_field(entry, "name").unwrap_or_else(|| id.clone()),
                            coming_soon: lua_field(entry, "coming_soon").as_deref() == Some("true"),
                            id,
                        });
                    }
                }
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    modes
}

pub fn read_registry_modes(api_dir: &Path) -> Vec<RegistryMode> {
    std::fs::read_to_string(api_dir.join(REGISTRY_FILE))
        .map(|lua| parse_registry_modes(&lua))
        .unwrap_or_default()
}

fn compact(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn registry_mode_matches(mode: &RegistryMode, pkg: &TsPackage) -> bool {
    let name = compact(&pkg.name);
    name == compact(&mode.id) || name == format!("multiplayer{}", compact(&mode.name))
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// A Multiplayer mod present in a Mods folder, with the database row tracking it
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstalledCopy {
    pub role: Role,
    pub path: String,
    pub folder: String,
    pub mod_id: String,
    /// Version in the mod's own metadata, the API reports 1.0.0 for package 0.1.2
    pub mod_version: Option<String>,
    pub package: Option<String>,
    pub package_version: Option<String>,
    pub tracked_name: Option<String>,
    pub tracked_source: Option<String>,
    pub tracked_version: Option<String>,
    /// Files of the official Thunderstore package rather than a manual or GitHub copy
    pub official: bool,
    pub disabled: bool,
}

impl InstalledCopy {
    /// Package version when known, the mod's own version otherwise
    pub fn version(&self) -> Option<&str> {
        if self.official {
            self.tracked_version
                .as_deref()
                .filter(|_| self.tracked_source.as_deref() == Some(SOURCE_THUNDERSTORE))
                .or(self.package_version.as_deref())
                .or(self.mod_version.as_deref())
        } else {
            self.mod_version
                .as_deref()
                .or(self.tracked_version.as_deref())
        }
    }
}

/// A row tracking a Multiplayer package whose folder is gone
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StaleRow {
    pub role: Role,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Detection {
    pub copies: Vec<InstalledCopy>,
    pub stale_rows: Vec<StaleRow>,
}

impl Detection {
    pub fn of(&self, role: Role) -> impl Iterator<Item = &InstalledCopy> {
        self.copies.iter().filter(move |c| c.role == role)
    }

    pub fn has(&self, track: Track) -> bool {
        self.of(track.role()).next().is_some()
    }

    /// The installed track, None when neither or both are installed
    pub fn active_track(&self) -> Option<Track> {
        match (self.has(Track::Legacy), self.has(Track::Api)) {
            (true, false) => Some(Track::Legacy),
            (false, true) => Some(Track::Api),
            _ => None,
        }
    }

    /// Both tracks ship lovely/TheOrder.toml and hud.toml, they cannot load together
    pub fn conflict(&self) -> bool {
        self.has(Track::Legacy) && self.has(Track::Api)
    }

    /// A legacy copy not installed from Thunderstore, such as the old GitHub release
    pub fn legacy_manual(&self) -> Option<&InstalledCopy> {
        self.of(Role::Legacy).find(|c| !c.official)
    }

    pub fn api_dir(&self) -> Option<PathBuf> {
        self.of(Role::Api)
            .find(|c| !c.disabled)
            .map(|c| PathBuf::from(&c.path))
    }
}

fn role_of(found: &FoundMod, gamemodes: &HashSet<String>) -> Option<Role> {
    let package = found.package.as_deref().map(str::to_ascii_lowercase);
    if found.id.eq_ignore_ascii_case(LEGACY_MOD_ID)
        || package.as_deref() == Some(&LEGACY_PACKAGE.to_ascii_lowercase())
    {
        return Some(Role::Legacy);
    }
    if found.id.eq_ignore_ascii_case(API_MOD_ID)
        || package.as_deref() == Some(&API_PACKAGE.to_ascii_lowercase())
    {
        return Some(Role::Api);
    }
    if package.as_ref().is_some_and(|p| gamemodes.contains(p)) || depends_on_api(found) {
        return Some(Role::Gamemode);
    }
    None
}

fn role_package(role: Role, found: &FoundMod) -> Option<String> {
    match role {
        Role::Legacy => Some(LEGACY_PACKAGE.to_string()),
        Role::Api => Some(API_PACKAGE.to_string()),
        Role::Gamemode => found
            .package
            .clone()
            .filter(|p| p.to_ascii_lowercase().starts_with("balatromultiplayer-")),
    }
}

pub fn detect(found: &[FoundMod], installed: &[InstalledMod], packages: &[TsPackage]) -> Detection {
    let gamemodes: HashSet<String> = gamemode_packages(packages)
        .iter()
        .map(|p| p.full_name.to_ascii_lowercase())
        .collect();
    let mut copies = Vec::new();
    let mut used_rows: HashSet<String> = HashSet::new();
    for m in found {
        let Some(role) = role_of(m, &gamemodes) else {
            continue;
        };
        let expected = role_package(role, m);
        let row = installed
            .iter()
            .find(|r| !r.path.trim().is_empty() && same_path(Path::new(&r.path), &m.path));
        let official = m
            .package
            .as_deref()
            .zip(expected.as_deref())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
            || row.is_some_and(|r| {
                r.source == SOURCE_THUNDERSTORE
                    && r.source_id
                        .as_deref()
                        .zip(expected.as_deref())
                        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
            });
        if let Some(r) = row {
            used_rows.insert(r.name.clone());
        }
        copies.push(InstalledCopy {
            role,
            path: m.path.to_string_lossy().to_string(),
            folder: m.folder.clone(),
            mod_id: m.id.clone(),
            mod_version: m.version.clone(),
            package: m.package.clone(),
            package_version: m.package_version.clone(),
            tracked_name: row.map(|r| r.name.clone()),
            tracked_source: row.map(|r| r.source.clone()),
            tracked_version: row.and_then(|r| r.current_version.clone()),
            official,
            disabled: m.disabled,
        });
    }
    let stale_rows = installed
        .iter()
        .filter(|r| !used_rows.contains(&r.name) && !Path::new(&r.path).exists())
        .filter_map(|r| {
            let id = r.source_id.as_deref()?.to_ascii_lowercase();
            let role = if id == LEGACY_PACKAGE.to_ascii_lowercase() {
                Role::Legacy
            } else if id == API_PACKAGE.to_ascii_lowercase() {
                Role::Api
            } else if gamemodes.contains(&id) {
                Role::Gamemode
            } else {
                return None;
            };
            Some(StaleRow {
                role,
                name: r.name.clone(),
            })
        })
        .collect();
    Detection { copies, stale_rows }
}

// ---------------------------------------------------------------------------
// Install plans and mutual exclusion
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Removal {
    pub role: Role,
    pub path: String,
    pub folder: String,
    pub tracked_name: Option<String>,
    /// Package that replaces this copy, set on `TrackPlan::replace` entries
    pub replaced_by: Option<String>,
    pub reason: String,
}

impl Removal {
    fn of(copy: &InstalledCopy, reason: String) -> Self {
        Self {
            role: copy.role,
            path: copy.path.clone(),
            folder: copy.folder.clone(),
            tracked_name: copy.tracked_name.clone(),
            replaced_by: None,
            reason,
        }
    }

    fn replacing(copy: &InstalledCopy, package: &str) -> Self {
        Self {
            replaced_by: Some(package.to_string()),
            ..Self::of(copy, format!("replaced by the official {package}"))
        }
    }
}

/// What installing a track does, `remove` needs the user's confirmation
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackPlan {
    pub track: Track,
    /// Official packages in install order, `Namespace-Name`
    pub packages: Vec<String>,
    /// The other track and its game modes, removed once the install succeeded
    pub remove: Vec<Removal>,
    /// Manual copies of the same mods replaced by the official packages, the first one
    /// with a database row keeps that row
    pub replace: Vec<Removal>,
    pub needs_confirmation: bool,
    pub warnings: Vec<String>,
}

pub fn plan_track(
    detection: &Detection,
    packages: &[TsPackage],
    track: Track,
    gamemodes: &[String],
) -> Result<TrackPlan, String> {
    let root = thunderstore::find_package(packages, track.package()).ok_or_else(|| {
        format!(
            "{} is not on Thunderstore right now, refresh and try again",
            track.package()
        )
    })?;
    if thunderstore::latest_active_version(root).is_none() {
        return Err(format!("{} has no active version", root.full_name));
    }
    if track == Track::Legacy && !gamemodes.is_empty() {
        return Err("Game modes need the MultiplayerAPI track".into());
    }

    let mut warnings = Vec::new();
    if root.is_deprecated {
        warnings.push(format!("{} is deprecated on Thunderstore", root.full_name));
    }
    let mut install = vec![root.full_name.clone()];
    let mut modes: Vec<&TsPackage> = Vec::new();
    for key in gamemodes {
        let pkg = resolve_gamemode(packages, key)
            .ok_or_else(|| format!("'{key}' is not a Multiplayer game mode"))?;
        if !install
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&pkg.full_name))
        {
            install.push(pkg.full_name.clone());
            modes.push(pkg);
        }
    }

    let remove: Vec<Removal> = match track {
        Track::Legacy => detection
            .copies
            .iter()
            .filter(|c| c.role != Role::Legacy)
            .map(|c| {
                Removal::of(
                    c,
                    match c.role {
                        Role::Gamemode => "game modes need MultiplayerAPI".into(),
                        _ => "MultiplayerAPI cannot be installed next to legacy Multiplayer".into(),
                    },
                )
            })
            .collect(),
        Track::Api => detection
            .of(Role::Legacy)
            .map(|c| {
                Removal::of(
                    c,
                    "legacy Multiplayer cannot be installed next to MultiplayerAPI".into(),
                )
            })
            .collect(),
    };

    let mut replace: Vec<Removal> = detection
        .of(track.role())
        .filter(|c| !c.official)
        .map(|c| Removal::replacing(c, &root.full_name))
        .collect();
    for pkg in &modes {
        replace.extend(
            detection
                .of(Role::Gamemode)
                .filter(|c| !c.official && !c.mod_id.is_empty())
                .filter(|c| {
                    compact(&c.folder) == compact(&pkg.name)
                        || compact(&c.folder) == compact(&pkg.full_name)
                })
                .map(|c| Removal::replacing(c, &pkg.full_name)),
        );
    }

    Ok(TrackPlan {
        track,
        packages: install,
        needs_confirmation: !remove.is_empty(),
        remove,
        replace,
        warnings,
    })
}

/// Upgrade of a manual legacy install (the old GitHub release) to the official package,
/// leaves the other track alone and the old folder goes only after the install succeeded
pub fn plan_legacy_migration(
    detection: &Detection,
    packages: &[TsPackage],
) -> Result<TrackPlan, String> {
    if detection.legacy_manual().is_none() {
        return Err("There is no manual Multiplayer install to upgrade".into());
    }
    let mut plan = plan_track(detection, packages, Track::Legacy, &[])?;
    plan.remove.clear();
    plan.needs_confirmation = false;
    Ok(plan)
}

fn removed_with(track: Track, role: Role) -> bool {
    match track {
        Track::Legacy => role == Role::Legacy,
        Track::Api => role != Role::Legacy,
    }
}

/// What uninstalling a track removes, the API takes its game modes with it
pub fn uninstall_targets(detection: &Detection, track: Track) -> Vec<Removal> {
    detection
        .copies
        .iter()
        .filter(|c| removed_with(track, c.role))
        .map(|c| Removal::of(c, "uninstall".into()))
        .collect()
}

/// Rows of the track whose folders are already gone
pub fn stale_rows_of(detection: &Detection, track: Track) -> Vec<String> {
    detection
        .stale_rows
        .iter()
        .filter(|r| removed_with(track, r.role))
        .map(|r| r.name.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackInfo {
    pub track: Track,
    pub full_name: String,
    pub title: String,
    pub description: String,
    pub latest_version: Option<String>,
    pub deprecated: bool,
    pub icon_url: String,
    pub package_url: String,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub tracked_name: Option<String>,
    /// Installed from the official package rather than a manual copy
    pub official: bool,
    pub disabled: bool,
    pub path: Option<String>,
    pub update_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameModeInfo {
    /// None for a mode the API lists as coming soon that is not on Thunderstore yet
    pub full_name: Option<String>,
    pub mod_id: Option<String>,
    pub title: String,
    pub description: String,
    pub latest_version: Option<String>,
    pub deprecated: bool,
    pub icon_url: String,
    pub package_url: String,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub tracked_name: Option<String>,
    pub update_available: bool,
    pub coming_soon: bool,
}

fn newer(latest: Option<&str>, installed: Option<&str>) -> bool {
    match (latest, installed) {
        (Some(l), Some(i)) => {
            thunderstore::parse_version(i).is_some()
                && thunderstore::compare_versions(l, i) == std::cmp::Ordering::Greater
        }
        _ => false,
    }
}

fn latest_of(pkg: Option<&TsPackage>) -> Option<String> {
    pkg.and_then(thunderstore::latest_active_version)
        .map(|v| v.version_number.clone())
}

fn description_of(pkg: Option<&TsPackage>) -> String {
    pkg.and_then(thunderstore::latest_active_version)
        .map(|v| v.description.clone())
        .unwrap_or_default()
}

pub fn track_info(detection: &Detection, packages: &[TsPackage], track: Track) -> TrackInfo {
    let pkg = thunderstore::find_package(packages, track.package());
    // An official copy wins over a manual one when both exist
    let copy = detection
        .of(track.role())
        .find(|c| c.official)
        .or_else(|| detection.of(track.role()).next());
    let latest_version = latest_of(pkg);
    let installed_version = copy.and_then(|c| c.version().map(str::to_string));
    TrackInfo {
        track,
        full_name: track.package().to_string(),
        title: match track {
            Track::Legacy => "Multiplayer".into(),
            Track::Api => "Multiplayer API".into(),
        },
        description: description_of(pkg),
        deprecated: pkg.is_some_and(|p| p.is_deprecated),
        icon_url: pkg.map(thunderstore::icon_url).unwrap_or_default(),
        package_url: pkg.map(|p| p.package_url.clone()).unwrap_or_default(),
        installed: copy.is_some(),
        tracked_name: copy.and_then(|c| c.tracked_name.clone()),
        official: copy.is_some_and(|c| c.official),
        disabled: copy.is_some_and(|c| c.disabled),
        path: copy.map(|c| c.path.clone()),
        // A manual copy reports the mod's own version, it is always worth replacing
        update_available: copy.is_some_and(|c| {
            !c.official || newer(latest_version.as_deref(), installed_version.as_deref())
        }),
        latest_version,
        installed_version,
    }
}

/// Packages built on the API, then modes the installed API marks as coming soon that no
/// package provides yet
pub fn gamemode_infos(
    detection: &Detection,
    packages: &[TsPackage],
    registry: &[RegistryMode],
) -> Vec<GameModeInfo> {
    let gamemode_pkgs = gamemode_packages(packages);
    let mut infos = Vec::new();
    for pkg in &gamemode_pkgs {
        let copy = detection.of(Role::Gamemode).find(|c| {
            c.package
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(&pkg.full_name))
                || compact(&c.folder) == compact(&pkg.name)
        });
        let installed = copy.is_some();
        if pkg.is_deprecated && !installed {
            continue;
        }
        let registry_mode = registry.iter().find(|m| registry_mode_matches(m, pkg));
        let latest_version = latest_of(Some(pkg));
        let installed_version = copy.and_then(|c| c.version().map(str::to_string));
        infos.push(GameModeInfo {
            full_name: Some(pkg.full_name.clone()),
            mod_id: copy
                .map(|c| c.mod_id.clone())
                .filter(|id| !id.is_empty())
                .or_else(|| registry_mode.map(|m| m.id.clone())),
            title: registry_mode.map(|m| m.name.clone()).unwrap_or_else(|| {
                let title = thunderstore::display_title(&pkg.name);
                title
                    .strip_prefix("Multiplayer")
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .unwrap_or(title)
            }),
            description: description_of(Some(pkg)),
            deprecated: pkg.is_deprecated,
            icon_url: thunderstore::icon_url(pkg),
            package_url: pkg.package_url.clone(),
            installed,
            tracked_name: copy.and_then(|c| c.tracked_name.clone()),
            update_available: copy.is_some_and(|c| {
                c.official && newer(latest_version.as_deref(), installed_version.as_deref())
            }),
            latest_version,
            installed_version,
            coming_soon: false,
        });
    }
    for mode in registry.iter().filter(|m| m.coming_soon) {
        let provided = gamemode_pkgs.iter().any(|p| registry_mode_matches(mode, p))
            || detection
                .of(Role::Gamemode)
                .any(|c| c.mod_id.eq_ignore_ascii_case(&mode.id));
        if provided {
            continue;
        }
        infos.push(GameModeInfo {
            full_name: None,
            mod_id: Some(mode.id.clone()),
            title: mode.name.clone(),
            description: String::new(),
            latest_version: None,
            deprecated: false,
            icon_url: String::new(),
            package_url: String::new(),
            installed: false,
            installed_version: None,
            tracked_name: None,
            update_available: false,
            coming_soon: true,
        });
    }
    infos
}

// ---------------------------------------------------------------------------
// Readiness
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchPlatform {
    Macos,
    Windows,
    /// Native LOVE on Linux
    Linux,
    /// The Windows build through Proton or Wine
    LinuxWine,
}

impl LaunchPlatform {
    pub fn current(linux_prefix: Option<&str>) -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if linux_prefix.is_some_and(|p| !p.trim().is_empty()) {
            Self::LinuxWine
        } else {
            Self::Linux
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadinessCheck {
    pub id: String,
    pub label: String,
    pub status: CheckStatus,
    pub detail: Option<String>,
}

fn check(id: &str, label: &str, status: CheckStatus, detail: Option<String>) -> ReadinessCheck {
    ReadinessCheck {
        id: id.into(),
        label: label.into(),
        status,
        detail,
    }
}

pub struct ReadinessInputs<'a> {
    pub platform: LaunchPlatform,
    pub detection: &'a Detection,
    pub packages: &'a [TsPackage],
    pub steam_running: bool,
    /// libssl.so.3 found on a native Linux system
    pub linux_libssl: Option<PathBuf>,
    pub steamodded_installed: bool,
    pub steamodded_version: Option<String>,
}

pub fn missing_libs(dir: &Path, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .filter(|n| !dir.join(n).is_file())
        .map(|n| n.to_string())
        .collect()
}

pub fn find_system_lib(name: &str, dirs: &[&str]) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| Path::new(d).join(name))
        .find(|p| p.exists())
}

/// Minimum Steamodded of the installed track, from the installed package version when it
/// is still listed, else from the latest
fn steamodded_min(detection: &Detection, packages: &[TsPackage], track: Track) -> Option<String> {
    let pkg = thunderstore::find_package(packages, track.package())?;
    let installed = detection
        .of(track.role())
        .find(|c| c.official)
        .and_then(|c| c.version().map(str::to_string));
    let version = installed
        .as_deref()
        .and_then(|v| thunderstore::find_version(pkg, v))
        .or_else(|| thunderstore::latest_active_version(pkg))?;
    version
        .dependencies
        .iter()
        .filter_map(|d| thunderstore::parse_dependency(d))
        .find(|d| {
            d.full_name()
                .eq_ignore_ascii_case(thunderstore::STEAMODDED_PACKAGE)
        })
        .map(|d| d.version)
}

pub fn readiness(inputs: &ReadinessInputs<'_>) -> Vec<ReadinessCheck> {
    use CheckStatus::*;
    let detection = inputs.detection;
    let mut checks = Vec::new();
    if detection.conflict() {
        checks.push(check(
            "track_conflict",
            "One Multiplayer track at a time",
            Error,
            Some("Legacy Multiplayer and MultiplayerAPI are both installed, keep only one".into()),
        ));
    }
    let Some(track) = detection.active_track().or_else(|| {
        detection
            .has(Track::Api)
            .then_some(Track::Api)
            .or_else(|| detection.has(Track::Legacy).then_some(Track::Legacy))
    }) else {
        return checks;
    };

    let min = steamodded_min(detection, inputs.packages, track);
    let smods_ok = inputs.steamodded_installed
        && min.as_deref().is_none_or(|min| {
            inputs
                .steamodded_version
                .as_deref()
                .is_some_and(|v| bmm_lib::smods_installer::satisfies_min(v, min))
        });
    let min_label = min
        .as_deref()
        .and_then(thunderstore::steamodded_version_to_tag)
        .or(min.clone());
    checks.push(check(
        "steamodded",
        "Steamodded",
        if smods_ok { Ok } else { Error },
        match (&min_label, smods_ok) {
            (_, true) => inputs.steamodded_version.clone(),
            (Some(min), false) => Some(format!("Needs Steamodded {min} or newer")),
            (None, false) => Some("Steamodded is not installed".into()),
        },
    ));

    if track != Track::Api {
        return checks;
    }
    let api_dir = detection.api_dir();
    let networking = api_dir.as_ref().map(|d| d.join(NETWORKING_DIR));
    let native = |names: &[&str], label: &str| {
        let missing = networking
            .as_deref()
            .map(|n| missing_libs(n, names))
            .unwrap_or_else(|| names.iter().map(|n| n.to_string()).collect());
        if missing.is_empty() {
            check("native_libs", label, Ok, None)
        } else {
            check(
                "native_libs",
                label,
                Error,
                Some(format!(
                    "Missing from the MultiplayerAPI networking folder: {}, reinstall MultiplayerAPI",
                    missing.join(", ")
                )),
            )
        }
    };
    match inputs.platform {
        LaunchPlatform::Macos => checks.push(native(&MACOS_LIBS, "Bundled OpenSSL libraries")),
        LaunchPlatform::Windows => {
            let mut c = native(&WINDOWS_LIBS, "Bundled OpenSSL DLLs");
            if c.status == Ok {
                c.detail = Some(
                    "BMM adds the networking folder to PATH when it launches the game, not tested on real Windows yet".into(),
                );
            }
            checks.push(c);
        }
        LaunchPlatform::LinuxWine => {
            let mut c = native(&WINDOWS_LIBS, "Bundled OpenSSL DLLs");
            if c.status == Ok {
                c.detail = Some(
                    "Passed through WINEPATH, or copied to the save folder for Steam launches, not tested on real hardware yet".into(),
                );
            }
            checks.push(c);
        }
        LaunchPlatform::Linux => checks.push(match &inputs.linux_libssl {
            Some(path) => check(
                "native_libs",
                "System OpenSSL 3",
                Ok,
                Some(path.to_string_lossy().to_string()),
            ),
            None => check(
                "native_libs",
                "System OpenSSL 3",
                Warning,
                Some(format!(
                    "{LINUX_LIBSSL} was not found, install OpenSSL 3 from your distribution"
                )),
            ),
        }),
    }
    checks.push(check(
        "steam_running",
        "Steam is running",
        if inputs.steam_running { Ok } else { Warning },
        (!inputs.steam_running)
            .then(|| "MultiplayerAPI needs Steam running to sign in".to_string()),
    ));
    checks
}

// ---------------------------------------------------------------------------
// Launch environment
// ---------------------------------------------------------------------------

/// How the game process is started, which decides how the API finds its DLLs
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchKind {
    /// The mod loads its dylibs from its own folder by absolute path, nothing to pass
    Macos,
    /// The mod searches the default DLL path, so its folder is prepended to PATH
    Windows,
    /// System OpenSSL through the loader path
    LinuxNative,
    /// Windows build under Proton or Wine, Steam's `-applaunch` hands the launch to the
    /// running Steam client, which drops our environment
    Wine { env_passable: bool },
}

/// Environment for the game process plus the DLL copy fallback
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchEnv {
    pub env: Vec<(String, String)>,
    pub copy_dlls_to_save_dir: bool,
}

/// The installed MultiplayerAPI as the launcher needs it
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiInstall {
    pub folder: String,
    pub networking: PathBuf,
}

#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub fn installed_api(found: &[FoundMod]) -> Option<ApiInstall> {
    found
        .iter()
        .filter(|m| m.id.eq_ignore_ascii_case(API_MOD_ID) && !m.disabled)
        .map(|m| ApiInstall {
            folder: m.folder.clone(),
            networking: m.path.join(NETWORKING_DIR),
        })
        .find(|a| a.networking.is_dir())
}

pub fn is_bet_key(key: &str) -> bool {
    key.len() >= BET_ENV_PREFIX.len()
        && key[..BET_ENV_PREFIX.len()].eq_ignore_ascii_case(BET_ENV_PREFIX)
}

/// Drops BET_ variables, such as those in a user supplied launch prefix
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub fn strip_bet_keys(env: Vec<(String, String)>) -> Vec<(String, String)> {
    env.into_iter()
        .filter(|(key, _)| {
            let bet = is_bet_key(key);
            if bet {
                log::warn!("Not passing {key} to Balatro, BMM never sets BET variables");
            }
            !bet
        })
        .collect()
}

/// BET_ variables the game would inherit from this process, to remove from its command
pub fn inherited_bet_keys() -> Vec<std::ffi::OsString> {
    std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| k.to_str().is_some_and(is_bet_key))
        .collect()
}

/// Prepends `entries` to a search path list, dropping duplicates of them further down
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub fn prepend_search_path(
    existing: Option<&str>,
    entries: &[String],
    sep: char,
    case_insensitive: bool,
) -> String {
    let norm = |s: &str| {
        let s = s.trim().trim_end_matches(['\\', '/']);
        if case_insensitive {
            s.to_lowercase()
        } else {
            s.to_string()
        }
    };
    let added: HashSet<String> = entries.iter().map(|e| norm(e)).collect();
    let mut parts: Vec<String> = entries.to_vec();
    parts.extend(
        existing
            .unwrap_or("")
            .split(sep)
            .filter(|p| !p.trim().is_empty() && !added.contains(&norm(p)))
            .map(str::to_string),
    );
    parts.join(&sep.to_string())
}

/// `Z:` drive path of a host path as Wine maps the root filesystem
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub fn wine_host_path(host: &Path) -> String {
    format!("Z:{}", host.to_string_lossy().replace('/', "\\"))
}

/// Networking folder as the game sees it inside a Proton prefix, where Mods is linked to the
/// host Mods folder
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub fn proton_networking_path(folder: &str) -> String {
    format!("C:\\users\\steamuser\\AppData\\Roaming\\Balatro\\Mods\\{folder}\\{NETWORKING_DIR}")
}

/// Variables to add for the API track, `inherited` is the value this process would pass on
/// for PATH or WINEPATH, the result never contains a BET_ key
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub fn launch_env(
    kind: LaunchKind,
    api: Option<&ApiInstall>,
    inherited: Option<&str>,
) -> LaunchEnv {
    let Some(api) = api else {
        return LaunchEnv::default();
    };
    let env = match kind {
        LaunchKind::Macos | LaunchKind::LinuxNative => Vec::new(),
        LaunchKind::Windows => vec![(
            "PATH".to_string(),
            prepend_search_path(
                inherited,
                &[api.networking.to_string_lossy().to_string()],
                ';',
                true,
            ),
        )],
        LaunchKind::Wine { env_passable: true } => vec![(
            "WINEPATH".to_string(),
            prepend_search_path(
                inherited,
                &[
                    proton_networking_path(&api.folder),
                    wine_host_path(&api.networking),
                ],
                ';',
                true,
            ),
        )],
        LaunchKind::Wine {
            env_passable: false,
        } => {
            return LaunchEnv {
                env: Vec::new(),
                copy_dlls_to_save_dir: true,
            };
        }
    };
    LaunchEnv {
        env: strip_bet_keys(env),
        copy_dlls_to_save_dir: false,
    }
}

/// Save folder of the Steam Proton prefix for a game installed in `steamapps/common/Balatro`
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn proton_save_dir(game_path: &Path) -> Option<PathBuf> {
    let common = game_path.parent()?;
    if common.file_name()?.to_str()? != "common" {
        return None;
    }
    let steamapps = common.parent()?;
    if steamapps.file_name()?.to_str()? != "steamapps" {
        return None;
    }
    Some(
        steamapps
            .join("compatdata")
            .join(BALATRO_APP_ID)
            .join("pfx/drive_c/users/steamuser/AppData/Roaming/Balatro"),
    )
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_tracked(path: &Path, tracked: &[PathBuf]) -> bool {
    tracked.iter().any(|t| same_path(t, path))
}

/// Copies the bundled DLLs into the save folder, which the mod searches when the launch
/// environment cannot be passed, leaves files BMM did not copy alone and returns every
/// file BMM now owns there
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn copy_dlls_to_save_dir(
    networking: &Path,
    save_dir: &Path,
    tracked: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(save_dir)
        .map_err(|e| format!("Failed to create {}: {e}", save_dir.display()))?;
    let mut owned = Vec::new();
    for name in WINDOWS_LIBS {
        let source = networking.join(name);
        let target = save_dir.join(name);
        if !source.is_file() {
            return Err(format!("{} is missing", source.display()));
        }
        if target.exists() && !is_tracked(&target, tracked) {
            log::info!("Keeping {} that BMM did not copy", target.display());
            continue;
        }
        std::fs::copy(&source, &target)
            .map_err(|e| format!("Failed to copy {name} to {}: {e}", save_dir.display()))?;
        owned.push(target);
    }
    Ok(owned)
}

/// Removes DLLs BMM copied, only the two OpenSSL names are ever touched
pub fn remove_copied_dlls(tracked: &[PathBuf]) -> Vec<PathBuf> {
    tracked
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| WINDOWS_LIBS.iter().any(|l| l.eq_ignore_ascii_case(n)))
        })
        .filter(|p| match std::fs::remove_file(p) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                log::warn!("Failed to remove {}: {e}", p.display());
                false
            }
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thunderstore::tests::fixture_packages;
    use bmm_lib::database::SOURCE_INDEX;

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn legacy_github(mods: &Path) -> PathBuf {
        let dir = mods.join("Multiplayer");
        write(
            &dir.join("Multiplayer.json"),
            r#"{"id":"Multiplayer","version":"0.5.4","dependencies":["Steamodded (>=1.0.0~BETA-1620a)"]}"#,
        );
        write(&dir.join("core.lua"), "-- mod");
        dir
    }

    fn api_package(mods: &Path, with_libs: &[&str]) -> PathBuf {
        let dir = mods.join("BalatroMultiplayer-MultiplayerAPI");
        write(
            &dir.join("MultiplayerAPI.json"),
            r#"{"id":"MultiplayerAPI","version":"1.0.0","dependencies":["Steamodded (>=1.0.0~BETA-1221a)"]}"#,
        );
        write(
            &dir.join("manifest.json"),
            r#"{"name":"MultiplayerAPI","version_number":"0.1.2","website_url":"","description":"","dependencies":["Steamodded-Steamodded-1.1814.0"],}"#,
        );
        write(
            &dir.join(REGISTRY_FILE),
            "state.official_mods = state.official_mods or {\n\t{ id = 'MultiplayerPvP', name = 'PvP', colour = G.C.RED, download_url = 'https://x/{y}', coming_soon = true },\n\t{ id = 'MultiplayerSPDRN', name = 'Speedrun', colour = G.C.GREEN, download_url = 'https://x' },\n\t{ id = 'MultiplayerCoop', name = 'Co-op', colour = G.C.BLUE, coming_soon = true },\n}\n",
        );
        for lib in with_libs {
            write(&dir.join(NETWORKING_DIR).join(lib), "bin");
        }
        std::fs::create_dir_all(dir.join(NETWORKING_DIR)).unwrap();
        dir
    }

    fn speedrun_package(mods: &Path) -> PathBuf {
        let dir = mods.join("BalatroMultiplayer-MultiplayerSpeedrun");
        write(
            &dir.join("BalatroSpeedrunning.json"),
            r#"{"id":"MultiplayerSPDRN","version":"1.0.0","dependencies":["Steamodded (>=1.0.0~BETA-1221a)","MultiplayerAPI"],"conflicts":["Multiplayer"]}"#,
        );
        write(
            &dir.join("manifest.json"),
            r#"{"name":"MultiplayerSpeedrun","version_number":"0.1.0","dependencies":[]}"#,
        );
        dir
    }

    fn row(
        name: &str,
        path: &Path,
        source: &str,
        source_id: Option<&str>,
        v: &str,
    ) -> InstalledMod {
        InstalledMod {
            name: name.into(),
            path: path.to_string_lossy().to_string(),
            dependencies: Vec::new(),
            current_version: Some(v.into()),
            orphaned: false,
            source: source.into(),
            source_id: source_id.map(str::to_string),
            installed_at: None,
        }
    }

    #[test]
    fn gamemodes_are_discovered_from_the_listing() {
        let packages = fixture_packages();
        let modes: Vec<&str> = gamemode_packages(&packages)
            .iter()
            .map(|p| p.full_name.as_str())
            .collect();
        assert_eq!(modes, ["BalatroMultiplayer-MultiplayerSpeedrun"]);
        assert!(resolve_gamemode(&packages, "MultiplayerSpeedrun").is_some());
        assert!(resolve_gamemode(&packages, "ts:BalatroMultiplayer-MultiplayerSpeedrun").is_some());
        assert!(resolve_gamemode(&packages, API_PACKAGE).is_none());
        assert!(resolve_gamemode(&packages, "nh6574-JokerDisplay").is_none());
    }

    #[test]
    fn registry_coming_soon_modes_are_parsed() {
        let td = tempfile::tempdir().unwrap();
        let api = api_package(td.path(), &[]);
        let modes = read_registry_modes(&api);
        assert_eq!(
            modes,
            [
                RegistryMode {
                    id: "MultiplayerPvP".into(),
                    name: "PvP".into(),
                    coming_soon: true
                },
                RegistryMode {
                    id: "MultiplayerSPDRN".into(),
                    name: "Speedrun".into(),
                    coming_soon: false
                },
                RegistryMode {
                    id: "MultiplayerCoop".into(),
                    name: "Co-op".into(),
                    coming_soon: true
                },
            ]
        );
        assert!(parse_registry_modes("return {}").is_empty());
    }

    #[test]
    fn tracks_are_detected_from_folders_and_rows() {
        let td = tempfile::tempdir().unwrap();
        let mods = td.path().join("Mods");
        let legacy = legacy_github(&mods);
        write(
            &mods.join("JokerDisplay/JokerDisplay.json"),
            r#"{"id":"JokerDisplay"}"#,
        );
        let packages = fixture_packages();
        let installed = [
            row("Multiplayer", &legacy, SOURCE_INDEX, None, "v0.5.4"),
            row(
                "MultiplayerAPI",
                &mods.join("BalatroMultiplayer-MultiplayerAPI"),
                SOURCE_THUNDERSTORE,
                Some(API_PACKAGE),
                "0.1.1",
            ),
        ];

        let detection = detect(&scan_mods_dir(&mods), &installed, &packages);
        assert_eq!(detection.copies.len(), 1);
        assert_eq!(detection.active_track(), Some(Track::Legacy));
        let manual = detection.legacy_manual().expect("manual legacy");
        assert_eq!(manual.tracked_name.as_deref(), Some("Multiplayer"));
        assert_eq!(manual.version(), Some("0.5.4"));
        assert!(!manual.official);
        // The API row points at a folder that is gone
        assert_eq!(stale_rows_of(&detection, Track::Api), ["MultiplayerAPI"]);
        assert!(stale_rows_of(&detection, Track::Legacy).is_empty());
        let info = track_info(&detection, &packages, Track::Legacy);
        assert!(info.installed && !info.official && info.update_available);
        assert_eq!(info.latest_version.as_deref(), Some("0.5.7"));

        api_package(&mods, &MACOS_LIBS);
        speedrun_package(&mods);
        let detection = detect(&scan_mods_dir(&mods), &installed, &packages);
        assert!(detection.conflict());
        assert_eq!(detection.active_track(), None);
        let api = detection.of(Role::Api).next().unwrap();
        assert!(api.official);
        assert_eq!(
            api.version(),
            Some("0.1.1"),
            "the tracked package version wins"
        );
        assert_eq!(api.mod_version.as_deref(), Some("1.0.0"));
        let mode = detection.of(Role::Gamemode).next().unwrap();
        assert_eq!(mode.mod_id, "MultiplayerSPDRN");
        assert!(mode.official, "official package files even without a row");
        assert!(detection.stale_rows.is_empty());

        let registry = read_registry_modes(&detection.api_dir().unwrap());
        let infos = gamemode_infos(&detection, &packages, &registry);
        let titles: Vec<(&str, bool, bool)> = infos
            .iter()
            .map(|i| (i.title.as_str(), i.installed, i.coming_soon))
            .collect();
        assert_eq!(
            titles,
            [
                ("Speedrun", true, false),
                ("PvP", false, true),
                ("Co-op", false, true)
            ]
        );
        assert!(infos[0].update_available);
        assert_eq!(infos[0].mod_id.as_deref(), Some("MultiplayerSPDRN"));
        // Without registry data only the packages are listed
        assert_eq!(gamemode_infos(&detection, &packages, &[]).len(), 1);
    }

    #[test]
    fn tracks_exclude_each_other() {
        let td = tempfile::tempdir().unwrap();
        let mods = td.path().join("Mods");
        let packages = fixture_packages();
        api_package(&mods, &[]);
        speedrun_package(&mods);
        let detection = detect(&scan_mods_dir(&mods), &[], &packages);

        let plan = plan_track(&detection, &packages, Track::Legacy, &[]).expect("plan");
        assert_eq!(plan.packages, [LEGACY_PACKAGE]);
        assert!(plan.needs_confirmation);
        let removed: Vec<(Role, &str)> = plan
            .remove
            .iter()
            .map(|r| (r.role, r.folder.as_str()))
            .collect();
        assert_eq!(
            removed,
            [
                (Role::Api, "BalatroMultiplayer-MultiplayerAPI"),
                (Role::Gamemode, "BalatroMultiplayer-MultiplayerSpeedrun")
            ]
        );
        assert!(plan.replace.is_empty());

        // Updating the installed track needs no confirmation
        let plan = plan_track(
            &detection,
            &packages,
            Track::Api,
            &["MultiplayerSpeedrun".to_string()],
        )
        .expect("plan");
        assert_eq!(
            plan.packages,
            [API_PACKAGE, "BalatroMultiplayer-MultiplayerSpeedrun"]
        );
        assert!(!plan.needs_confirmation && plan.remove.is_empty());

        assert!(
            plan_track(
                &detection,
                &packages,
                Track::Legacy,
                &["MultiplayerSpeedrun".into()]
            )
            .is_err()
        );
        assert!(plan_track(&detection, &packages, Track::Api, &["Nope".into()]).is_err());
        assert!(plan_track(&detection, &[], Track::Api, &[]).is_err());

        let targets = uninstall_targets(&detection, Track::Api);
        assert_eq!(targets.len(), 2, "game modes go with the API");
        assert!(uninstall_targets(&detection, Track::Legacy).is_empty());
    }

    #[test]
    fn legacy_github_install_migrates_to_the_official_package() {
        let td = tempfile::tempdir().unwrap();
        let mods = td.path().join("Mods");
        let legacy = legacy_github(&mods);
        let packages = fixture_packages();
        let mut legacy_row = row("Multiplayer", &legacy, SOURCE_INDEX, None, "v0.5.4");
        legacy_row.orphaned = true;
        let detection = detect(&scan_mods_dir(&mods), &[legacy_row], &packages);

        let plan = plan_legacy_migration(&detection, &packages).expect("plan");
        assert_eq!(plan.track, Track::Legacy);
        assert_eq!(plan.packages, [LEGACY_PACKAGE]);
        assert!(!plan.needs_confirmation && plan.remove.is_empty());
        assert_eq!(plan.replace.len(), 1);
        assert_eq!(plan.replace[0].tracked_name.as_deref(), Some("Multiplayer"));
        assert_eq!(plan.replace[0].replaced_by.as_deref(), Some(LEGACY_PACKAGE));
        assert_eq!(plan.replace[0].path, legacy.to_string_lossy());

        // Switching to the API removes the manual copy instead, after confirmation
        let plan = plan_track(&detection, &packages, Track::Api, &[]).expect("plan");
        assert!(plan.needs_confirmation);
        assert_eq!(plan.remove[0].folder, "Multiplayer");

        // Nothing to migrate once the official package is installed
        let official = mods.join("BalatroMultiplayer-Multiplayer");
        write(
            &official.join("Multiplayer.json"),
            r#"{"id":"Multiplayer","version":"0.5.5"}"#,
        );
        write(
            &official.join("manifest.json"),
            r#"{"name":"Multiplayer","version_number":"0.5.7"}"#,
        );
        std::fs::remove_dir_all(&legacy).unwrap();
        let detection = detect(&scan_mods_dir(&mods), &[], &packages);
        assert!(detection.legacy_manual().is_none());
        assert!(plan_legacy_migration(&detection, &packages).is_err());
        let info = track_info(&detection, &packages, Track::Legacy);
        assert_eq!(info.installed_version.as_deref(), Some("0.5.7"));
        assert!(!info.update_available);
    }

    fn readiness_for(
        platform: LaunchPlatform,
        detection: &Detection,
        steam_running: bool,
    ) -> Vec<ReadinessCheck> {
        readiness(&ReadinessInputs {
            platform,
            detection,
            packages: &fixture_packages(),
            steam_running,
            linux_libssl: None,
            steamodded_installed: true,
            steamodded_version: Some("1.0.0~BETA-1016c".into()),
        })
    }

    fn status_of<'a>(checks: &'a [ReadinessCheck], id: &str) -> Option<&'a ReadinessCheck> {
        checks.iter().find(|c| c.id == id)
    }

    #[test]
    fn readiness_checks_follow_the_platform() {
        let td = tempfile::tempdir().unwrap();
        let mods = td.path().join("Mods");
        api_package(&mods, &MACOS_LIBS);
        let detection = detect(&scan_mods_dir(&mods), &[], &fixture_packages());

        let mac = readiness_for(LaunchPlatform::Macos, &detection, false);
        assert_eq!(
            status_of(&mac, "native_libs").unwrap().status,
            CheckStatus::Ok
        );
        assert_eq!(
            status_of(&mac, "steam_running").unwrap().status,
            CheckStatus::Warning
        );
        // 1016c is older than the 1814a the API asks for
        let smods = status_of(&mac, "steamodded").unwrap();
        assert_eq!(smods.status, CheckStatus::Error);
        assert!(
            smods
                .detail
                .as_deref()
                .unwrap()
                .contains("1.0.0-beta-1814a")
        );

        let win = readiness_for(LaunchPlatform::Windows, &detection, true);
        let libs = status_of(&win, "native_libs").unwrap();
        assert_eq!(libs.status, CheckStatus::Error);
        assert!(libs.detail.as_deref().unwrap().contains("libssl-3-x64.dll"));
        assert_eq!(
            status_of(&win, "steam_running").unwrap().status,
            CheckStatus::Ok
        );

        let linux = readiness_for(LaunchPlatform::Linux, &detection, true);
        assert_eq!(
            status_of(&linux, "native_libs").unwrap().status,
            CheckStatus::Warning
        );

        api_package(&mods, &WINDOWS_LIBS);
        let detection = detect(&scan_mods_dir(&mods), &[], &fixture_packages());
        let wine = readiness_for(LaunchPlatform::LinuxWine, &detection, true);
        assert_eq!(
            status_of(&wine, "native_libs").unwrap().status,
            CheckStatus::Ok
        );

        legacy_github(&mods);
        let detection = detect(&scan_mods_dir(&mods), &[], &fixture_packages());
        let both = readiness_for(LaunchPlatform::Macos, &detection, true);
        assert_eq!(
            status_of(&both, "track_conflict").unwrap().status,
            CheckStatus::Error
        );

        let empty = Detection::default();
        assert!(readiness_for(LaunchPlatform::Macos, &empty, false).is_empty());
    }

    #[test]
    fn legacy_track_only_checks_steamodded() {
        let td = tempfile::tempdir().unwrap();
        let mods = td.path().join("Mods");
        legacy_github(&mods);
        let detection = detect(&scan_mods_dir(&mods), &[], &fixture_packages());
        let checks = readiness_for(LaunchPlatform::Windows, &detection, false);
        assert_eq!(checks.len(), 1);
        // 1016c is older than the 1620 minimum of the legacy package
        assert_eq!(checks[0].status, CheckStatus::Error);
    }

    #[test]
    fn windows_path_puts_the_networking_folder_first() {
        let api = ApiInstall {
            folder: "BalatroMultiplayer-MultiplayerAPI".into(),
            networking: PathBuf::from(
                r"C:\Users\me\AppData\Roaming\Balatro\Mods\BalatroMultiplayer-MultiplayerAPI\networking",
            ),
        };
        let env = launch_env(
            LaunchKind::Windows,
            Some(&api),
            Some(
                r"C:\Windows\system32;c:\users\me\appdata\roaming\balatro\mods\balatromultiplayer-multiplayerapi\networking\;C:\Tools",
            ),
        );
        assert!(!env.copy_dlls_to_save_dir);
        assert_eq!(
            env.env,
            [(
                "PATH".to_string(),
                r"C:\Users\me\AppData\Roaming\Balatro\Mods\BalatroMultiplayer-MultiplayerAPI\networking;C:\Windows\system32;C:\Tools".to_string()
            )]
        );
        assert_eq!(
            prepend_search_path(None, &["A".into()], ';', true),
            "A",
            "no trailing separator without an inherited PATH"
        );
    }

    #[test]
    fn wine_launches_use_winepath_or_the_dll_copy() {
        let api = ApiInstall {
            folder: "BalatroMultiplayer-MultiplayerAPI".into(),
            networking: PathBuf::from(
                "/home/me/.local/share/Balatro/Mods/BalatroMultiplayer-MultiplayerAPI/networking",
            ),
        };
        let env = launch_env(LaunchKind::Wine { env_passable: true }, Some(&api), None);
        assert_eq!(
            env.env,
            [(
                "WINEPATH".to_string(),
                r"C:\users\steamuser\AppData\Roaming\Balatro\Mods\BalatroMultiplayer-MultiplayerAPI\networking;Z:\home\me\.local\share\Balatro\Mods\BalatroMultiplayer-MultiplayerAPI\networking".to_string()
            )]
        );
        let applaunch = launch_env(
            LaunchKind::Wine {
                env_passable: false,
            },
            Some(&api),
            None,
        );
        assert!(applaunch.env.is_empty() && applaunch.copy_dlls_to_save_dir);
        assert_eq!(
            launch_env(LaunchKind::Windows, None, None),
            LaunchEnv::default()
        );
        assert!(
            launch_env(LaunchKind::Macos, Some(&api), None)
                .env
                .is_empty()
        );
        assert!(
            launch_env(LaunchKind::LinuxNative, Some(&api), None)
                .env
                .is_empty()
        );

        assert_eq!(
            proton_save_dir(Path::new("/s/steamapps/common/Balatro")),
            Some(PathBuf::from(
                "/s/steamapps/compatdata/2379780/pfx/drive_c/users/steamuser/AppData/Roaming/Balatro"
            ))
        );
        assert_eq!(proton_save_dir(Path::new("/games/Balatro")), None);
    }

    #[test]
    fn launch_env_never_contains_bet_keys() {
        let api = ApiInstall {
            folder: "BalatroMultiplayer-MultiplayerAPI".into(),
            networking: PathBuf::from("/m/BalatroMultiplayer-MultiplayerAPI/networking"),
        };
        let kinds = [
            LaunchKind::Macos,
            LaunchKind::Windows,
            LaunchKind::LinuxNative,
            LaunchKind::Wine { env_passable: true },
            LaunchKind::Wine {
                env_passable: false,
            },
        ];
        for kind in kinds {
            for inherited in [None, Some("BET_RANKED_SUPERVISOR_PORT=1;C:\\x")] {
                let env = launch_env(kind, Some(&api), inherited);
                assert!(
                    env.env.iter().all(|(k, _)| !is_bet_key(k)),
                    "{kind:?} sets a BET_ key"
                );
            }
        }
        let user_prefix = vec![
            ("BET_OPENSSL_DYLIB_DIR".to_string(), "/x".to_string()),
            ("bet_ranked_supervisor_secret".to_string(), "s".to_string()),
            ("WINEDLLOVERRIDES".to_string(), "winmm=n,b".to_string()),
        ];
        assert_eq!(
            strip_bet_keys(user_prefix),
            [("WINEDLLOVERRIDES".to_string(), "winmm=n,b".to_string())]
        );
        assert!(is_bet_key("BET_"));
        assert!(!is_bet_key("BETTER"));
        assert!(!is_bet_key("ALPHABET_X"));
    }

    #[test]
    fn dll_copies_are_tracked_and_user_files_kept() {
        let td = tempfile::tempdir().unwrap();
        let networking = td.path().join("api/networking");
        for lib in WINDOWS_LIBS {
            write(&networking.join(lib), lib);
        }
        let save = td.path().join("save");
        write(&save.join("libssl-3-x64.dll"), "user copy");

        let owned = copy_dlls_to_save_dir(&networking, &save, &[]).expect("copy");
        assert_eq!(owned, [save.join("libcrypto-3-x64.dll")]);
        assert_eq!(
            std::fs::read_to_string(save.join("libssl-3-x64.dll")).unwrap(),
            "user copy"
        );
        // A later launch refreshes its own copy
        let owned = copy_dlls_to_save_dir(&networking, &save, &owned).expect("copy");
        assert_eq!(owned, [save.join("libcrypto-3-x64.dll")]);

        let mut tracked = owned.clone();
        tracked.push(save.join("other.dll"));
        assert_eq!(remove_copied_dlls(&tracked), owned);
        assert!(!save.join("libcrypto-3-x64.dll").exists());
        assert!(save.join("libssl-3-x64.dll").exists());

        assert!(copy_dlls_to_save_dir(&td.path().join("missing"), &save, &[]).is_err());
    }

    #[test]
    fn installed_api_skips_disabled_copies() {
        let td = tempfile::tempdir().unwrap();
        let mods = td.path().join("Mods");
        let dir = api_package(&mods, &WINDOWS_LIBS);
        let api = installed_api(&scan_mods_dir(&mods)).expect("api");
        assert_eq!(api.folder, "BalatroMultiplayer-MultiplayerAPI");
        assert_eq!(api.networking, dir.join(NETWORKING_DIR));
        write(&dir.join(".lovelyignore"), "");
        assert_eq!(installed_api(&scan_mods_dir(&mods)), None);
    }

    #[test]
    fn steamodded_dependency_alternatives_are_read() {
        let td = tempfile::tempdir().unwrap();
        let dir = td.path().join("Mode");
        write(
            &dir.join("mode.json"),
            r#"{"id":"Mode","dependencies":[["MultiplayerAPI (>=1.0)", "Other"]]}"#,
        );
        let found = read_mod_folder(&dir).unwrap();
        assert!(depends_on_api(&found));
        assert_eq!(
            dependency_id("Steamodded (>=1.0.0~BETA-1221a)"),
            "Steamodded"
        );
        assert_eq!(dependency_id("MultiplayerAPI>=1"), "MultiplayerAPI");
    }
}
