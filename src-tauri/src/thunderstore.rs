//! Reader for the Balatro community on Thunderstore
//!
//! The listing index always redirects to a content addressed blob that lists gzip chunks,
//! so an unchanged redirect `Location` means the cached packages are current and no chunk
//! is downloaded. The deprecated v1 package API is the fallback

use std::cmp::Ordering;
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::Duration;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::models::{ThunderstoreInfo, ThunderstoreVersionInfo};

pub const SITE: &str = "https://thunderstore.io";
pub const COMMUNITY: &str = "balatro";
pub const MULTIPLAYER_NAMESPACE: &str = "BalatroMultiplayer";
/// Windows only Lovely build (winmm.dll), the app manages Lovely itself
pub const LOVELY_PACKAGE: &str = "Thunderstore-lovely";
pub const STEAMODDED_PACKAGE: &str = "Steamodded-Steamodded";
/// Deprecated upload of the old 0.8/0.9 Steamodded API
pub const LEGACY_STEAMODDED_PACKAGE: &str = "Steamopollys-Steamodded";
pub const CACHE_FILE: &str = "thunderstore_balatro_v1.json";

const CACHE_SCHEMA: u32 = 1;
const MIN_REFRESH_SECS: u64 = 300;
const FAILED_RETRY_SECS: u64 = 60;
const MAX_BLOB_BYTES: u64 = 64 * 1024 * 1024;

pub fn source_id() -> String {
    format!("thunderstore:{COMMUNITY}")
}

pub fn listing_index_url() -> String {
    format!("{SITE}/c/{COMMUNITY}/api/v1/package-listing-index/")
}

pub fn v1_listing_url() -> String {
    format!("{SITE}/c/{COMMUNITY}/api/v1/package/")
}

pub fn readme_url(namespace: &str, name: &str, version: &str) -> String {
    format!("{SITE}/api/experimental/package/{namespace}/{name}/{version}/readme/")
}

/// Official download endpoint, it redirects to the Thunderstore CDN
pub fn download_url(namespace: &str, name: &str, version: &str) -> String {
    format!("{SITE}/package/download/{namespace}/{name}/{version}/")
}

static NO_REDIRECT_HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(format!(
            "balatro-mod-manager/{} (+{})",
            env!("CARGO_PKG_VERSION"),
            crate::links::APP_REPO_URL
        ))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client")
});

static CACHE: Lazy<tokio::sync::Mutex<Option<TsCache>>> =
    Lazy::new(|| tokio::sync::Mutex::new(None));
static SYNC: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));
static CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);
static LAST_FAILED_SYNC: AtomicU64 = AtomicU64::new(0);
static LAST_ERROR: Lazy<std::sync::Mutex<Option<String>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TsVersion {
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub icon: String,
    pub version_number: String,
    pub dependencies: Vec<String>,
    pub download_url: String,
    pub downloads: u64,
    pub date_created: String,
    pub website_url: String,
    pub is_active: bool,
    pub uuid4: String,
    pub file_size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TsPackage {
    pub name: String,
    pub full_name: String,
    pub owner: String,
    pub package_url: String,
    pub donation_link: Option<String>,
    pub date_created: String,
    pub date_updated: String,
    pub uuid4: String,
    pub rating_score: i64,
    pub is_pinned: bool,
    pub is_deprecated: bool,
    pub has_nsfw_content: bool,
    pub categories: Vec<String>,
    pub versions: Vec<TsVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TsCache {
    pub schema: u32,
    pub source: String,
    pub index_location: Option<String>,
    pub chunk_urls: Vec<String>,
    pub synced_at: u64,
    pub packages: Vec<TsPackage>,
}

impl TsCache {
    pub fn empty() -> Self {
        Self {
            schema: CACHE_SCHEMA,
            source: source_id(),
            index_location: None,
            chunk_urls: Vec::new(),
            synced_at: 0,
            packages: Vec::new(),
        }
    }
}

/// `Namespace-Name-x.y.z` as written in a manifest dependency list
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsDependency {
    pub namespace: String,
    pub name: String,
    pub version: String,
}

impl TsDependency {
    pub fn full_name(&self) -> String {
        format!("{}-{}", self.namespace, self.name)
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

fn is_ident(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Splits `Namespace-Name-x.y.z` into exactly three parts from the right, so namespaces
/// with underscores (`just_frostice482`) stay intact. The version is a minimum
pub fn parse_dependency(dep: &str) -> Option<TsDependency> {
    let mut parts = dep.trim().rsplitn(3, '-');
    let version = parts.next()?;
    let name = parts.next()?;
    let namespace = parts.next()?;
    if !is_ident(namespace) || !is_ident(name) || parse_version(version).is_none() {
        return None;
    }
    Some(TsDependency {
        namespace: namespace.to_string(),
        name: name.to_string(),
        version: version.to_string(),
    })
}

/// Splits `Namespace-Name` into its two parts
pub fn split_full_name(full_name: &str) -> Option<(&str, &str)> {
    let (namespace, name) = full_name.trim().split_once('-')?;
    (is_ident(namespace) && is_ident(name)).then_some((namespace, name))
}

/// Numeric `x.y.z` components, None when any part is not a number
pub fn parse_version(version: &str) -> Option<Vec<u64>> {
    let version = version.trim();
    if version.is_empty() {
        return None;
    }
    version.split('.').map(|p| p.parse::<u64>().ok()).collect()
}

/// Semver style comparison of Thunderstore versions, missing parts count as zero and
/// unparsable versions sort before any valid one
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    match (parse_version(a), parse_version(b)) {
        (Some(a), Some(b)) => {
            let len = a.len().max(b.len());
            (0..len)
                .map(|i| {
                    a.get(i)
                        .copied()
                        .unwrap_or(0)
                        .cmp(&b.get(i).copied().unwrap_or(0))
                })
                .find(|o| o.is_ne())
                .unwrap_or(Ordering::Equal)
        }
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => a.cmp(b),
    }
}

pub fn version_at_least(version: &str, minimum: &str) -> bool {
    compare_versions(version, minimum) != Ordering::Less
}

/// Highest active version, inactive ones were pulled by moderators
pub fn latest_active_version(pkg: &TsPackage) -> Option<&TsVersion> {
    pkg.versions
        .iter()
        .filter(|v| v.is_active && parse_version(&v.version_number).is_some())
        .max_by(|a, b| compare_versions(&a.version_number, &b.version_number))
}

pub fn find_version<'a>(pkg: &'a TsPackage, version: &str) -> Option<&'a TsVersion> {
    pkg.versions
        .iter()
        .find(|v| v.is_active && v.version_number == version.trim())
}

/// Thunderstore Steamodded version to the matching smods GitHub tag,
/// `1.1814.0` is `1.0.0-beta-1814a`, from 26.x on both use the same string
pub fn steamodded_version_to_tag(version: &str) -> Option<String> {
    bmm_lib::smods_installer::ts_version_to_tag(version)
}

/// Reverse of [`steamodded_version_to_tag`], also accepts the `~BETA-` spelling of mod headers
#[allow(dead_code)]
pub fn steamodded_tag_to_version(tag: &str) -> Option<String> {
    let (major, minor, patch) = bmm_lib::smods_installer::parse_smods_version(tag)?;
    Some(format!("{major}.{minor}.{patch}"))
}

/// Maps Thunderstore categories to the app categories, `AI Generated` is a flag instead
pub fn map_categories(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |cat: &str| {
        if !out.iter().any(|c| c == cat) {
            out.push(cat.to_string());
        }
    };
    for cat in raw {
        match cat.trim() {
            "Content Mods" | "New Mechanics" => push("Content"),
            "Quality of Life" => push("Quality of Life"),
            "Tools" => push("Technical"),
            "Libraries" => push("API"),
            "Textures" | "Audio" => push("Resource Packs"),
            "Gameplay Tweaks" | "Misc" | "Translation" | "Text Changes" => push("Miscellaneous"),
            _ => {}
        }
    }
    // Almost every package is tagged `Mods`, it only counts when nothing else mapped
    if out.is_empty() {
        out.push("Miscellaneous".to_string());
    }
    out
}

pub fn is_ai_generated(raw: &[String]) -> bool {
    raw.iter()
        .any(|c| c.trim().eq_ignore_ascii_case("AI Generated"))
}

/// The experimental endpoint reports -1, so the total is summed over versions
pub fn total_downloads(pkg: &TsPackage) -> u64 {
    pkg.versions.iter().map(|v| v.downloads).sum()
}

pub fn icon_url(pkg: &TsPackage) -> String {
    latest_active_version(pkg)
        .map(|v| v.icon.trim().to_string())
        .unwrap_or_default()
}

pub fn is_multiplayer(pkg: &TsPackage) -> bool {
    pkg.owner.eq_ignore_ascii_case(MULTIPLAYER_NAMESPACE)
}

pub fn is_lovely(full_name: &str) -> bool {
    full_name.trim().eq_ignore_ascii_case(LOVELY_PACKAGE)
}

pub fn is_steamodded(full_name: &str) -> bool {
    let full_name = full_name.trim();
    full_name.eq_ignore_ascii_case(STEAMODDED_PACKAGE)
        || full_name.eq_ignore_ascii_case(LEGACY_STEAMODDED_PACKAGE)
}

/// Display title, Thunderstore names use underscores for spaces
pub fn display_title(name: &str) -> String {
    name.replace('_', " ").trim().to_string()
}

/// Seconds since the epoch of an RFC 3339 timestamp such as `2026-10-02T20:51:59.547381+00:00`
pub fn parse_timestamp(ts: &str) -> Option<u64> {
    let ts = ts.trim();
    let (date, rest) = ts.split_once(['T', 't', ' '])?;
    let mut d = date.splitn(3, '-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    if rest.len() < 8 {
        return None;
    }
    let mut t = rest[..8].splitn(3, ':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next()?.parse().ok()?;
    let zone = rest[8..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match zone {
        "" | "Z" | "z" => 0,
        z if z.len() == 6 && (z.starts_with('+') || z.starts_with('-')) => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let h: i64 = z[1..3].parse().ok()?;
            let m: i64 = z[4..6].parse().ok()?;
            sign * (h * 3600 + m * 60)
        }
        _ => return None,
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from civil, Howard Hinnant's algorithm
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset;
    u64::try_from(secs).ok()
}

/// Thunderstore part of a catalog entry, None when the package has no active version
pub fn package_info(pkg: &TsPackage) -> Option<ThunderstoreInfo> {
    let latest = latest_active_version(pkg)?;
    let (namespace, name) = split_full_name(&pkg.full_name)
        .map(|(ns, n)| (ns.to_string(), n.to_string()))
        .unwrap_or_else(|| (pkg.owner.clone(), pkg.name.clone()));
    let mut versions: Vec<&TsVersion> = pkg.versions.iter().filter(|v| v.is_active).collect();
    versions.sort_by(|a, b| compare_versions(&b.version_number, &a.version_number));
    Some(ThunderstoreInfo {
        download_url: download_url(&namespace, &name, &latest.version_number),
        namespace,
        name,
        full_name: pkg.full_name.clone(),
        owner: pkg.owner.clone(),
        package_url: pkg.package_url.clone(),
        latest_version: latest.version_number.clone(),
        versions: versions
            .into_iter()
            .map(|v| ThunderstoreVersionInfo {
                version: v.version_number.clone(),
                deps: v.dependencies.clone(),
                date: v.date_created.clone(),
                size: v.file_size,
            })
            .collect(),
        website_url: latest.website_url.trim().to_string(),
        icon_url: icon_url(pkg),
        downloads_total: total_downloads(pkg),
        rating: pkg.rating_score,
        deprecated: pkg.is_deprecated,
        nsfw: pkg.has_nsfw_content,
        ai_generated: is_ai_generated(&pkg.categories),
        categories_raw: pkg.categories.clone(),
        date_updated: pkg.date_updated.clone(),
        description: latest.description.trim().to_string(),
    })
}

/// Listing blobs are gzip without a Content-Encoding header, plain JSON is accepted too
pub fn decode_blob(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(MAX_BLOB_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("invalid gzip blob: {e}"))?;
    if out.len() as u64 > MAX_BLOB_BYTES {
        return Err("Thunderstore listing blob is too large".to_string());
    }
    Ok(out)
}

pub fn parse_chunk_urls(bytes: &[u8]) -> Result<Vec<String>, String> {
    let urls: Vec<String> = serde_json::from_slice(&decode_blob(bytes)?)
        .map_err(|e| format!("invalid chunk list: {e}"))?;
    for url in &urls {
        if !reqwest::Url::parse(url).is_ok_and(|u| u.scheme() == "https") {
            return Err(format!("unexpected chunk URL '{url}'"));
        }
    }
    Ok(urls)
}

pub fn parse_packages(bytes: &[u8]) -> Result<Vec<TsPackage>, String> {
    serde_json::from_slice(&decode_blob(bytes)?).map_err(|e| format!("invalid package list: {e}"))
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FetchResponse {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// HTTP access used by the sync, injectable so tests run without the network
pub trait Fetcher: Sync {
    fn get(
        &self,
        url: &str,
        follow_redirects: bool,
    ) -> impl Future<Output = Result<FetchResponse, String>> + Send;
}

pub struct HttpFetcher;

impl Fetcher for HttpFetcher {
    async fn get(&self, url: &str, follow_redirects: bool) -> Result<FetchResponse, String> {
        let client = if follow_redirects {
            crate::mod_index::http_client()
        } else {
            &*NO_REDIRECT_HTTP
        };
        let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = if (300..400).contains(&status) {
            Vec::new()
        } else {
            resp.bytes().await.map_err(|e| e.to_string())?.to_vec()
        };
        Ok(FetchResponse {
            status,
            location,
            body,
        })
    }
}

fn success_body(url: &str, resp: FetchResponse) -> Result<Vec<u8>, String> {
    if (200..300).contains(&resp.status) {
        Ok(resp.body)
    } else {
        Err(format!("HTTP {} from {url}", resp.status))
    }
}

/// Redirect target of the listing index, it changes only when the listing changes
async fn listing_location<F: Fetcher>(fetcher: &F) -> Result<String, String> {
    let url = listing_index_url();
    let resp = fetcher.get(&url, false).await?;
    if !(300..400).contains(&resp.status) {
        return Err(format!(
            "listing index returned HTTP {} instead of a redirect",
            resp.status
        ));
    }
    let location = resp
        .location
        .filter(|l| !l.trim().is_empty())
        .ok_or_else(|| "listing index redirect has no Location".to_string())?;
    let base = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
    base.join(location.trim())
        .map(|u| u.to_string())
        .map_err(|e| format!("invalid listing Location: {e}"))
}

async fn fetch_chunked<F: Fetcher>(
    fetcher: &F,
    location: &str,
) -> Result<(Vec<String>, Vec<TsPackage>), String> {
    let index = success_body(location, fetcher.get(location, true).await?)?;
    let chunk_urls = parse_chunk_urls(&index)?;
    let mut packages = Vec::new();
    for url in &chunk_urls {
        let body = success_body(url, fetcher.get(url, true).await?)?;
        packages.extend(parse_packages(&body)?);
    }
    Ok((chunk_urls, packages))
}

async fn fetch_v1<F: Fetcher>(fetcher: &F) -> Result<Vec<TsPackage>, String> {
    let url = v1_listing_url();
    parse_packages(&success_body(&url, fetcher.get(&url, true).await?)?)
}

/// Brings `cache` up to date, skipping the chunk downloads when the listing is unchanged
pub(crate) async fn refresh<F: Fetcher>(
    cache: &mut TsCache,
    fetcher: &F,
    now: u64,
) -> Result<(), String> {
    let chunked = match listing_location(fetcher).await {
        Ok(location) => {
            if cache.index_location.as_deref() == Some(location.as_str())
                && !cache.packages.is_empty()
            {
                cache.synced_at = now;
                return Ok(());
            }
            fetch_chunked(fetcher, &location)
                .await
                .map(|(chunks, packages)| (location, chunks, packages))
        }
        Err(e) => Err(e),
    };
    match chunked {
        Ok((location, chunk_urls, packages)) => {
            log::info!(
                "Thunderstore: {} packages from {} chunk(s)",
                packages.len(),
                chunk_urls.len()
            );
            cache.index_location = Some(location);
            cache.chunk_urls = chunk_urls;
            cache.packages = packages;
            cache.synced_at = now;
            Ok(())
        }
        Err(e) => {
            log::warn!("Thunderstore: listing index failed ({e}), falling back to the v1 API");
            let packages = fetch_v1(fetcher)
                .await
                .map_err(|v1| format!("{e}; v1 fallback failed: {v1}"))?;
            cache.index_location = None;
            cache.chunk_urls.clear();
            cache.packages = packages;
            cache.synced_at = now;
            Ok(())
        }
    }
}

/// Refreshes `cache`, on failure a non empty cache is still served and the error returned
/// alongside, an empty one makes the failure fatal
pub(crate) async fn refresh_or_keep<F: Fetcher>(
    cache: &mut TsCache,
    fetcher: &F,
    now: u64,
) -> Result<Option<String>, String> {
    match refresh(cache, fetcher, now).await {
        Ok(()) => Ok(None),
        Err(e) if !cache.packages.is_empty() => Ok(Some(e)),
        Err(e) => Err(e),
    }
}

pub async fn fetch_readme(namespace: &str, name: &str, version: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Readme {
        markdown: Option<String>,
    }
    if !is_ident(namespace) || !is_ident(name) || parse_version(version).is_none() {
        return Err(format!("invalid package {namespace}-{name}-{version}"));
    }
    let url = readme_url(namespace, name, version);
    let body = success_body(&url, HttpFetcher.get(&url, true).await?)?;
    let readme: Readme = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    Ok(readme.markdown.unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Persistence and shared state
// ---------------------------------------------------------------------------

fn cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Balatro").join("mod_index_cache").join(CACHE_FILE))
}

pub(crate) fn load_cache_from(path: &Path) -> Option<TsCache> {
    let text = std::fs::read_to_string(path).ok()?;
    let cache: TsCache = serde_json::from_str(&text).ok()?;
    if cache.schema != CACHE_SCHEMA || cache.source != source_id() {
        log::info!("Thunderstore: discarding cache from another source or schema");
        return None;
    }
    Some(cache)
}

pub(crate) fn save_cache_to(path: &Path, cache: &TsCache) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec(cache).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn load_or_empty(guard: &mut Option<TsCache>) -> &mut TsCache {
    guard.get_or_insert_with(|| {
        cache_path()
            .as_deref()
            .and_then(load_cache_from)
            .unwrap_or_else(TsCache::empty)
    })
}

fn cache_is_fresh(cache: &TsCache, last_failed: u64, now: u64) -> bool {
    !cache.packages.is_empty()
        && (now.saturating_sub(cache.synced_at) < MIN_REFRESH_SECS
            || now.saturating_sub(last_failed) < FAILED_RETRY_SECS)
}

fn set_last_error(error: Option<String>) {
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = error;
}

pub fn last_sync_error() -> Option<String> {
    LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub async fn synced_at() -> u64 {
    CACHE.lock().await.as_ref().map_or(0, |c| c.synced_at)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// All packages of the community, refreshed at most every few minutes (always with
/// `force`), served from the on-disk cache when Thunderstore is unreachable
pub async fn sync(force: bool) -> Result<Vec<TsPackage>, String> {
    let _sync = SYNC.lock().await;
    let now = now_secs();
    let (mut working, generation) = {
        let mut guard = CACHE.lock().await;
        let cache = load_or_empty(&mut guard);
        let last_failed = LAST_FAILED_SYNC.load(AtomicOrdering::Relaxed);
        if !force && cache_is_fresh(cache, last_failed, now) {
            return Ok(cache.packages.clone());
        }
        (
            cache.clone(),
            CACHE_GENERATION.load(AtomicOrdering::Relaxed),
        )
    };

    let result = refresh_or_keep(&mut working, &HttpFetcher, now).await;

    let mut guard = CACHE.lock().await;
    let current = CACHE_GENERATION.load(AtomicOrdering::Relaxed) == generation;
    match result {
        Ok(None) => {
            LAST_FAILED_SYNC.store(0, AtomicOrdering::Relaxed);
            set_last_error(None);
            let packages = working.packages.clone();
            if current {
                if let Some(path) = cache_path()
                    && let Err(e) = save_cache_to(&path, &working)
                {
                    log::warn!("Thunderstore: failed to persist cache: {e}");
                }
                *guard = Some(working);
            }
            Ok(packages)
        }
        Ok(Some(e)) => {
            LAST_FAILED_SYNC.store(now, AtomicOrdering::Relaxed);
            log::warn!("Thunderstore: sync failed, serving cached packages: {e}");
            set_last_error(Some(e));
            Ok(working.packages)
        }
        Err(e) => {
            LAST_FAILED_SYNC.store(now, AtomicOrdering::Relaxed);
            set_last_error(Some(e.clone()));
            Err(format!("Could not load Thunderstore packages: {e}"))
        }
    }
}

/// Packages as currently known, syncing only when nothing is cached yet
pub async fn cached_packages() -> Result<Vec<TsPackage>, String> {
    {
        let mut guard = CACHE.lock().await;
        let cache = load_or_empty(&mut guard);
        if !cache.packages.is_empty() {
            return Ok(cache.packages.clone());
        }
    }
    sync(false).await
}

/// Packages from memory or the on-disk cache, empty when nothing is cached. Never hits the network
pub async fn cached_packages_offline() -> Vec<TsPackage> {
    let mut guard = CACHE.lock().await;
    load_or_empty(&mut guard).packages.clone()
}

/// Latest active version per package keyed by lowercase `Namespace-Name`
pub fn latest_versions(packages: &[TsPackage]) -> std::collections::HashMap<String, String> {
    packages
        .iter()
        .filter_map(|p| {
            latest_active_version(p)
                .map(|v| (p.full_name.to_ascii_lowercase(), v.version_number.clone()))
        })
        .collect()
}

pub fn find_package<'a>(packages: &'a [TsPackage], full_name: &str) -> Option<&'a TsPackage> {
    let full_name = full_name.trim();
    packages
        .iter()
        .find(|p| p.full_name == full_name)
        .or_else(|| {
            packages
                .iter()
                .find(|p| p.full_name.eq_ignore_ascii_case(full_name))
        })
}

/// Forgets the in-memory packages so the next request reloads them from disk or the network
pub async fn reset_cache() {
    let mut guard = CACHE.lock().await;
    CACHE_GENERATION.fetch_add(1, AtomicOrdering::Relaxed);
    LAST_FAILED_SYNC.store(0, AtomicOrdering::Relaxed);
    set_last_error(None);
    *guard = None;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;
    use std::sync::Mutex;
    use tempfile::TempDir;

    pub const FIXTURE: &str = include_str!("../tests/fixtures/thunderstore_listing.json");
    const BLOB: &str = "https://ccdn.thunderstore.io/live/blob-storage/sha256/abc.blob";
    const CHUNK: &str = "https://ccdn.thunderstore.io/live/blob-storage/sha256/chunk1.blob";

    pub fn fixture_packages() -> Vec<TsPackage> {
        parse_packages(FIXTURE.as_bytes()).expect("fixture parses")
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    /// Serves canned responses and records every requested URL
    #[derive(Default)]
    struct FakeFetcher {
        routes: HashMap<String, FetchResponse>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeFetcher {
        fn route(mut self, url: &str, status: u16, location: Option<&str>, body: Vec<u8>) -> Self {
            self.routes.insert(
                url.to_string(),
                FetchResponse {
                    status,
                    location: location.map(str::to_string),
                    body,
                },
            );
            self
        }

        fn listing(location: &str, packages_json: &str) -> Self {
            Self::default()
                .route(&listing_index_url(), 302, Some(location), Vec::new())
                .route(
                    location,
                    200,
                    None,
                    gzip(format!("[\"{CHUNK}\"]").as_bytes()),
                )
                .route(CHUNK, 200, None, gzip(packages_json.as_bytes()))
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Fetcher for FakeFetcher {
        async fn get(&self, url: &str, _follow: bool) -> Result<FetchResponse, String> {
            self.calls.lock().unwrap().push(url.to_string());
            self.routes
                .get(url)
                .cloned()
                .ok_or_else(|| format!("offline: {url}"))
        }
    }

    fn block_on<T>(f: impl Future<Output = T>) -> T {
        tokio_test::block_on(f)
    }

    fn pkg<'a>(packages: &'a [TsPackage], full_name: &str) -> &'a TsPackage {
        find_package(packages, full_name).expect(full_name)
    }

    #[test]
    fn fixture_has_the_expected_shape() {
        let packages = fixture_packages();
        let smods = pkg(&packages, STEAMODDED_PACKAGE);
        assert_eq!(smods.owner, "Steamodded");
        assert!(smods.is_pinned);
        assert!(!smods.versions.is_empty());
        assert!(pkg(&packages, LEGACY_STEAMODDED_PACKAGE).is_deprecated);
        assert!(pkg(&packages, "crimsonseraphim-Lust").has_nsfw_content);
        assert_eq!(
            packages.iter().filter(|p| is_multiplayer(p)).count(),
            3,
            "three Multiplayer packages"
        );
    }

    #[test]
    fn parse_dependency_splits_from_the_right() {
        let dep = parse_dependency("just_frostice482-Amulet-3.6.1").expect("parses");
        assert_eq!(dep.namespace, "just_frostice482");
        assert_eq!(dep.name, "Amulet");
        assert_eq!(dep.version, "3.6.1");
        assert_eq!(dep.full_name(), "just_frostice482-Amulet");

        let dep = parse_dependency("Thunderstore-lovely-0.9.0").unwrap();
        assert_eq!(dep.full_name(), LOVELY_PACKAGE);
        assert_eq!(
            parse_dependency(" Steamodded-Steamodded-1.1814.0 ")
                .unwrap()
                .version,
            "1.1814.0"
        );
        assert_eq!(parse_dependency("Amulet-3.6.1"), None);
        assert_eq!(parse_dependency("a-b-c"), None);
        assert_eq!(parse_dependency("a b-c-1.0.0"), None);
        assert_eq!(parse_dependency(""), None);
    }

    #[test]
    fn every_fixture_dependency_parses() {
        for p in fixture_packages() {
            for v in &p.versions {
                for d in &v.dependencies {
                    assert!(parse_dependency(d).is_some(), "{d} in {}", v.full_name);
                }
            }
        }
    }

    #[test]
    fn split_full_name_rejects_garbage() {
        assert_eq!(
            split_full_name("BalatroMultiplayer-MultiplayerAPI"),
            Some(("BalatroMultiplayer", "MultiplayerAPI"))
        );
        assert_eq!(
            split_full_name("just_frostice482-Amulet"),
            Some(("just_frostice482", "Amulet"))
        );
        assert_eq!(split_full_name("NoDash"), None);
        assert_eq!(split_full_name("a-b/../c"), None);
        assert_eq!(split_full_name("-b"), None);
    }

    #[test]
    fn compare_versions_is_numeric() {
        assert_eq!(compare_versions("1.10.0", "1.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("26.1002.0", "1.1814.0"), Ordering::Greater);
        assert_eq!(compare_versions("0.5.7", "0.5.7"), Ordering::Equal);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.1.2", "0.1.10"), Ordering::Less);
        assert_eq!(compare_versions("garbage", "0.0.1"), Ordering::Less);
        assert!(version_at_least("1.1814.0", "1.1620.0"));
        assert!(version_at_least("0.9.0", "0.9.0"));
        assert!(!version_at_least("0.8.0", "0.9.0"));
    }

    #[test]
    fn latest_active_version_skips_inactive_and_ignores_order() {
        let mut p = TsPackage::default();
        for (v, active) in [("1.2.0", true), ("1.10.0", true), ("2.0.0", false)] {
            p.versions.push(TsVersion {
                version_number: v.into(),
                is_active: active,
                ..Default::default()
            });
        }
        assert_eq!(latest_active_version(&p).unwrap().version_number, "1.10.0");
        assert!(find_version(&p, "2.0.0").is_none());
        assert!(find_version(&p, "1.2.0").is_some());
        p.versions.iter_mut().for_each(|v| v.is_active = false);
        assert!(latest_active_version(&p).is_none());
    }

    #[test]
    fn steamodded_version_maps_to_tags() {
        let tag = steamodded_version_to_tag;
        assert_eq!(tag("1.1814.0").as_deref(), Some("1.0.0-beta-1814a"));
        assert_eq!(tag("1.1606.1").as_deref(), Some("1.0.0-beta-1606b"));
        assert_eq!(tag("1.1016.2").as_deref(), Some("1.0.0-beta-1016c"));
        assert_eq!(tag("1.827.2").as_deref(), Some("1.0.0-beta-0827c"));
        assert_eq!(tag("26.1002.0").as_deref(), Some("26.1002.0"));
        assert_eq!(tag("0.9.8").as_deref(), Some("0.9.8"));
        assert_eq!(
            tag("1.1531.999999").as_deref(),
            Some("1.0.0-beta-1531zeebee")
        );
        assert_eq!(tag("1.2"), None);

        let back = steamodded_tag_to_version;
        assert_eq!(back("1.0.0-beta-1814a").as_deref(), Some("1.1814.0"));
        assert_eq!(back("1.0.0~BETA-0827c").as_deref(), Some("1.827.2"));
        assert_eq!(back("26.1002.0").as_deref(), Some("26.1002.0"));
        assert_eq!(back("nonsense"), None);
        for v in ["1.1814.0", "1.1606.1", "26.1002.0"] {
            assert_eq!(back(&tag(v).unwrap()).as_deref(), Some(v));
        }
    }

    #[test]
    fn categories_map_to_app_categories() {
        let cats =
            |raw: &[&str]| map_categories(&raw.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(cats(&["Mods"]), vec!["Miscellaneous"]);
        assert_eq!(cats(&[]), vec!["Miscellaneous"]);
        assert_eq!(
            cats(&["Mods", "Tools", "Libraries"]),
            vec!["Technical", "API"]
        );
        assert_eq!(cats(&["Content Mods", "New Mechanics"]), vec!["Content"]);
        assert_eq!(cats(&["Textures", "Audio"]), vec!["Resource Packs"]);
        assert_eq!(cats(&["Quality of Life"]), vec!["Quality of Life"]);
        assert_eq!(
            cats(&["Gameplay Tweaks", "Misc", "Translation", "Text Changes"]),
            vec!["Miscellaneous"]
        );
        assert_eq!(cats(&["AI Generated", "Mods"]), vec!["Miscellaneous"]);
        assert!(is_ai_generated(&["AI Generated".to_string()]));
        assert!(!is_ai_generated(&["Mods".to_string()]));
    }

    #[test]
    fn package_info_reads_flags_downloads_and_icon() {
        let packages = fixture_packages();
        let api = pkg(&packages, "BalatroMultiplayer-MultiplayerAPI");
        let info = package_info(api).expect("active");
        assert_eq!(info.namespace, "BalatroMultiplayer");
        assert_eq!(info.name, "MultiplayerAPI");
        assert!(info.ai_generated);
        assert!(!info.nsfw);
        assert_eq!(
            info.downloads_total,
            api.versions.iter().map(|v| v.downloads).sum::<u64>()
        );
        assert!(info.icon_url.starts_with("https://"));
        assert_eq!(
            info.download_url,
            format!(
                "https://thunderstore.io/package/download/BalatroMultiplayer/MultiplayerAPI/{}/",
                info.latest_version
            )
        );
        assert_eq!(info.versions[0].version, info.latest_version);

        let lust = package_info(pkg(&packages, "crimsonseraphim-Lust")).unwrap();
        assert!(lust.nsfw);
        let legacy = package_info(pkg(&packages, LEGACY_STEAMODDED_PACKAGE)).unwrap();
        assert!(legacy.deprecated);
    }

    #[test]
    fn parse_timestamp_handles_offsets_and_fractions() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_timestamp("2026-10-02T20:51:59.547381+00:00"),
            Some(1_790_974_319)
        );
        assert_eq!(
            parse_timestamp("2026-10-02T22:51:59+02:00"),
            Some(1_790_974_319)
        );
        assert_eq!(parse_timestamp("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        assert_eq!(parse_timestamp("not a date"), None);
        assert_eq!(parse_timestamp("2024-13-01T00:00:00Z"), None);
    }

    #[test]
    fn decode_blob_accepts_gzip_and_plain() {
        assert_eq!(decode_blob(&gzip(b"[1]")).unwrap(), b"[1]");
        assert_eq!(decode_blob(b"[1]").unwrap(), b"[1]");
        assert!(decode_blob(&[0x1f, 0x8b, 0, 0]).is_err());
        assert!(parse_chunk_urls(&gzip(b"[\"http://insecure/x\"]")).is_err());
        assert_eq!(
            parse_chunk_urls(&gzip(format!("[\"{CHUNK}\"]").as_bytes())).unwrap(),
            vec![CHUNK.to_string()]
        );
    }

    #[test]
    fn refresh_follows_listing_location_and_stores_chunks() {
        let fetcher = FakeFetcher::listing(BLOB, FIXTURE);
        let mut cache = TsCache::empty();
        block_on(refresh(&mut cache, &fetcher, 100)).expect("refresh");
        assert_eq!(cache.index_location.as_deref(), Some(BLOB));
        assert_eq!(cache.chunk_urls, vec![CHUNK.to_string()]);
        assert_eq!(cache.packages.len(), fixture_packages().len());
        assert_eq!(cache.synced_at, 100);
    }

    #[test]
    fn unchanged_location_skips_chunk_fetch() {
        let fetcher = FakeFetcher::listing(BLOB, FIXTURE);
        let mut cache = TsCache::empty();
        block_on(refresh(&mut cache, &fetcher, 100)).unwrap();
        let before = fetcher.calls().len();

        block_on(refresh(&mut cache, &fetcher, 500)).unwrap();
        let calls = fetcher.calls();
        assert_eq!(calls[before..], [listing_index_url()]);
        assert_eq!(cache.synced_at, 500);
        assert_eq!(cache.packages.len(), fixture_packages().len());
    }

    #[test]
    fn changed_location_refetches_chunks() {
        let mut cache = TsCache::empty();
        cache.index_location = Some("https://ccdn.thunderstore.io/old.blob".into());
        cache.packages = vec![TsPackage::default()];
        let fetcher = FakeFetcher::listing(BLOB, FIXTURE);
        block_on(refresh(&mut cache, &fetcher, 1)).unwrap();
        assert!(fetcher.calls().contains(&CHUNK.to_string()));
        assert_eq!(cache.packages.len(), fixture_packages().len());
    }

    #[test]
    fn relative_location_is_resolved_against_the_site() {
        let blob = "https://thunderstore.io/blob/rel.blob";
        let fetcher = FakeFetcher::default()
            .route(
                &listing_index_url(),
                302,
                Some("/blob/rel.blob"),
                Vec::new(),
            )
            .route(blob, 200, None, gzip(format!("[\"{CHUNK}\"]").as_bytes()))
            .route(CHUNK, 200, None, FIXTURE.as_bytes().to_vec());
        let mut cache = TsCache::empty();
        block_on(refresh(&mut cache, &fetcher, 1)).unwrap();
        assert_eq!(cache.index_location.as_deref(), Some(blob));
    }

    #[test]
    fn falls_back_to_v1_when_the_listing_index_fails() {
        let fetcher = FakeFetcher::default()
            .route(&listing_index_url(), 500, None, Vec::new())
            .route(&v1_listing_url(), 200, None, FIXTURE.as_bytes().to_vec());
        let mut cache = TsCache::empty();
        cache.index_location = Some(BLOB.into());
        block_on(refresh(&mut cache, &fetcher, 7)).expect("v1 fallback");
        assert_eq!(cache.index_location, None);
        assert!(cache.chunk_urls.is_empty());
        assert_eq!(cache.packages.len(), fixture_packages().len());

        // A broken chunk also falls back
        let fetcher = FakeFetcher::default()
            .route(&listing_index_url(), 302, Some(BLOB), Vec::new())
            .route(BLOB, 200, None, gzip(b"not json"))
            .route(&v1_listing_url(), 200, None, FIXTURE.as_bytes().to_vec());
        let mut cache = TsCache::empty();
        block_on(refresh(&mut cache, &fetcher, 7)).expect("v1 fallback");
        assert_eq!(cache.packages.len(), fixture_packages().len());
    }

    #[test]
    fn offline_serves_the_disk_cache_and_reports_the_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("mod_index_cache").join(CACHE_FILE);
        let mut cache = TsCache::empty();
        block_on(refresh(
            &mut cache,
            &FakeFetcher::listing(BLOB, FIXTURE),
            10,
        ))
        .unwrap();
        save_cache_to(&path, &cache).unwrap();

        let mut loaded = load_cache_from(&path).expect("cache on disk");
        let offline = FakeFetcher::default();
        let served = block_on(refresh_or_keep(&mut loaded, &offline, 999)).expect("served");
        assert!(served.is_some_and(|e| e.contains("offline")));
        assert_eq!(loaded.packages.len(), fixture_packages().len());
        assert_eq!(loaded.synced_at, 10);

        let mut empty = TsCache::empty();
        assert!(block_on(refresh_or_keep(&mut empty, &offline, 999)).is_err());
    }

    #[test]
    fn cache_roundtrip_discards_other_schema_or_source() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join(CACHE_FILE);
        let mut cache = TsCache::empty();
        cache.index_location = Some(BLOB.into());
        cache.chunk_urls = vec![CHUNK.into()];
        cache.synced_at = 42;
        cache.packages = fixture_packages();
        save_cache_to(&path, &cache).unwrap();
        assert!(!path.with_extension("json.tmp").exists());

        let loaded = load_cache_from(&path).expect("loads");
        assert_eq!(loaded.index_location.as_deref(), Some(BLOB));
        assert_eq!(loaded.chunk_urls, cache.chunk_urls);
        assert_eq!(loaded.synced_at, 42);
        assert_eq!(loaded.packages, cache.packages);

        cache.source = "thunderstore:lethal-company".into();
        save_cache_to(&path, &cache).unwrap();
        assert!(load_cache_from(&path).is_none());

        cache.source = source_id();
        cache.schema = CACHE_SCHEMA + 1;
        save_cache_to(&path, &cache).unwrap();
        assert!(load_cache_from(&path).is_none());

        std::fs::write(&path, "{broken").unwrap();
        assert!(load_cache_from(&path).is_none());
    }

    #[test]
    fn cache_freshness_and_failure_backoff() {
        let now = 10_000;
        let mut cache = TsCache::empty();
        cache.synced_at = now - 10;
        assert!(
            !cache_is_fresh(&cache, 0, now),
            "empty cache is never fresh"
        );
        cache.packages = vec![TsPackage::default()];
        assert!(cache_is_fresh(&cache, 0, now));
        cache.synced_at = now - MIN_REFRESH_SECS;
        assert!(!cache_is_fresh(&cache, 0, now));
        assert!(cache_is_fresh(&cache, now - 30, now));
        assert!(!cache_is_fresh(&cache, now - FAILED_RETRY_SECS, now));
    }

    /// Hits the real Thunderstore: `cargo test live_thunderstore -- --ignored --nocapture`
    #[test]
    #[ignore = "network: fetches the live Thunderstore listing"]
    fn live_thunderstore_smoke() {
        tauri::async_runtime::block_on(async {
            let mut cache = TsCache::empty();
            refresh(&mut cache, &HttpFetcher, now_secs())
                .await
                .expect("sync");
            println!(
                "live Thunderstore: {} packages, location {:?}",
                cache.packages.len(),
                cache.index_location
            );
            assert!(cache.index_location.is_some(), "listing index was used");
            assert!(cache.packages.len() > 150);
            assert!(find_package(&cache.packages, "nh6574-JokerDisplay").is_some());
            let api = find_package(&cache.packages, "BalatroMultiplayer-MultiplayerAPI")
                .expect("MultiplayerAPI");
            assert!(latest_active_version(api).is_some());

            let location = cache.index_location.clone();
            refresh(&mut cache, &HttpFetcher, now_secs())
                .await
                .expect("second sync");
            assert_eq!(cache.index_location, location);

            let joker = find_package(&cache.packages, "nh6574-JokerDisplay").unwrap();
            let version = latest_active_version(joker).unwrap().version_number.clone();
            let readme = fetch_readme("nh6574", "JokerDisplay", &version)
                .await
                .expect("readme");
            assert!(readme.contains("JokerDisplay"));
        });
    }
}
