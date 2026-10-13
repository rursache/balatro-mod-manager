//! Serverless reader for the community mod index hosted on GitHub
//!
//! The head commit is resolved with a git protocol v2 `ls-refs` request (no API quota),
//! the file listing comes from one git trees API call per index commit and only the
//! `meta.json` / `description.md` blobs whose SHA changed are downloaded from
//! commit pinned raw URLs, with jsDelivr as a mirror

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::{StreamExt, stream};
use once_cell::sync::Lazy;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::assets::safe_slug;
use crate::models::{ArchiveModItem, ModMeta};

pub const INDEX_OWNER: &str = "kasimeka";
pub const INDEX_NAME: &str = "balatro-mod-index";
pub const INDEX_BRANCH: &str = "main";

const INDEX_MIN_REFRESH_SECS: u64 = 300;
const INDEX_FAILED_RETRY_SECS: u64 = 60;
const FETCH_CONCURRENCY: usize = 16;
const CACHE_SCHEMA: u32 = 1;
const CACHE_FILE: &str = "github_index_v1.json";
const SOURCE_MARKER_FILE: &str = "catalog_source";
const GITHUB_API_VERSION: &str = "2022-11-28";

pub fn index_source_id() -> String {
    format!("github:{INDEX_OWNER}/{INDEX_NAME}@{INDEX_BRANCH}")
}

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(format!(
            "balatro-mod-manager/{} (+{})",
            env!("CARGO_PKG_VERSION"),
            crate::links::APP_REPO_URL
        ))
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client")
});

/// Shared HTTP client for GitHub and mirror traffic
pub fn http_client() -> &'static reqwest::Client {
    &HTTP
}

static INDEX: Lazy<tokio::sync::Mutex<Option<IndexCache>>> =
    Lazy::new(|| tokio::sync::Mutex::new(None));
// Serializes syncs so INDEX is only locked briefly and lookups never wait on the network
static SYNC: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));
// Bumped by reset_cache so a sync that started before the reset does not write back
static CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);
static LAST_FAILED_SYNC: AtomicU64 = AtomicU64::new(0);
static LAST_ERROR: Lazy<std::sync::Mutex<Option<String>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

fn set_last_error(error: Option<String>) {
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = error;
}

/// Error of the most recent failed sync, cleared by the next successful one
pub fn last_sync_error() -> Option<String> {
    LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// When the in-memory catalog last synced with GitHub, 0 when nothing is loaded
pub async fn synced_at() -> u64 {
    INDEX.lock().await.as_ref().map_or(0, |c| c.synced_at)
}

#[derive(Debug, Deserialize)]
pub struct TreeResp {
    #[serde(default)]
    pub truncated: bool,
    pub tree: Vec<TreeEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub sha: String,
}

/// (path, blob sha) of the files the manager cares about in one `mods/<dir>`
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DirFiles {
    pub meta: Option<(String, String)>,
    pub desc: Option<(String, String)>,
    pub thumb: Option<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub meta_sha: String,
    pub desc_sha: Option<String>,
    pub thumb_path: Option<String>,
    pub thumb_sha: Option<String>,
    pub item: ArchiveModItem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexCache {
    pub schema: u32,
    pub source: String,
    pub commit_sha: Option<String>,
    pub synced_at: u64,
    pub entries: BTreeMap<String, IndexEntry>,
}

impl IndexCache {
    fn empty() -> Self {
        Self {
            schema: CACHE_SCHEMA,
            source: index_source_id(),
            commit_sha: None,
            synced_at: 0,
            entries: BTreeMap::new(),
        }
    }

    fn sorted_items(&self) -> Vec<ArchiveModItem> {
        let mut items: Vec<ArchiveModItem> =
            self.entries.values().map(|e| e.item.clone()).collect();
        sort_items(&mut items);
        items
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    Meta,
    Desc,
}

#[derive(Debug, Default)]
pub struct SyncPlan {
    pub to_fetch: Vec<(String, FileKind, String)>,
    pub removed_dirs: Vec<String>,
    pub thumbs_changed: Vec<String>,
}

type FetchResult = Result<Option<Vec<u8>>, String>;

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

fn pkt_line(payload: &str) -> String {
    format!("{:04x}{payload}", payload.len() + 4)
}

pub fn build_ls_refs_body(branch: &str) -> Vec<u8> {
    let mut body = pkt_line("command=ls-refs\n");
    body.push_str("0001");
    body.push_str(&pkt_line(&format!("ref-prefix refs/heads/{branch}\n")));
    body.push_str("0000");
    body.into_bytes()
}

/// Extracts the SHA of `refs/heads/<branch>` from a protocol v2 ls-refs response
pub fn parse_ls_refs(bytes: &[u8], branch: &str) -> Option<String> {
    let wanted = format!("refs/heads/{branch}");
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let len = usize::from_str_radix(std::str::from_utf8(&bytes[i..i + 4]).ok()?, 16).ok()?;
        // 0000 flush, 0001 delim, 0002 response end
        if len < 4 {
            i += 4;
            continue;
        }
        if i + len > bytes.len() {
            return None;
        }
        let line = std::str::from_utf8(&bytes[i + 4..i + len]).ok()?;
        let line = line.trim_end_matches('\n');
        if let Some((sha, rest)) = line.split_once(' ')
            && rest.split(' ').next() == Some(wanted.as_str())
            && is_sha(sha)
        {
            return Some(sha.to_ascii_lowercase());
        }
        i += len;
    }
    None
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

const IMAGE_EXTS: [&str; 5] = ["jpg", "jpeg", "png", "webp", "gif"];

fn split_ext(name: &str) -> Option<(&str, &str)> {
    name.rsplit_once('.')
}

/// Picks the thumbnail among the files of a mod dir, preferring `thumbnail.<ext>`
pub fn pick_thumbnail<'a>(names: &[&'a str]) -> Option<&'a str> {
    let mut images: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| {
            split_ext(n)
                .is_some_and(|(_, ext)| IMAGE_EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
        })
        .collect();
    images.sort_unstable();
    images
        .iter()
        .copied()
        .find(|n| split_ext(n).is_some_and(|(stem, _)| stem.eq_ignore_ascii_case("thumbnail")))
        .or_else(|| images.first().copied())
}

pub fn group_tree(tree: &TreeResp) -> BTreeMap<String, DirFiles> {
    let mut files: BTreeMap<&str, Vec<(&str, &str, &str)>> = BTreeMap::new();
    for entry in &tree.tree {
        if entry.kind != "blob" {
            continue;
        }
        let mut parts = entry.path.split('/');
        let (Some("mods"), Some(dir), Some(file), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if dir.is_empty() || file.is_empty() {
            continue;
        }
        files
            .entry(dir)
            .or_default()
            .push((file, entry.path.as_str(), entry.sha.as_str()));
    }

    let mut out = BTreeMap::new();
    for (dir, list) in files {
        let find = |name: &str| {
            list.iter()
                .find(|(f, _, _)| *f == name)
                .map(|(_, p, s)| (p.to_string(), s.to_string()))
        };
        let Some(meta) = find("meta.json") else {
            continue;
        };
        let names: Vec<&str> = list.iter().map(|(f, _, _)| *f).collect();
        let thumb = pick_thumbnail(&names).and_then(find);
        out.insert(
            dir.to_string(),
            DirFiles {
                meta: Some(meta),
                desc: find("description.md"),
                thumb,
            },
        );
    }
    out
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| urlencoding::encode(seg).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn raw_url(commit_sha: &str, path: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/{INDEX_OWNER}/{INDEX_NAME}/{commit_sha}/{}",
        encode_path(path)
    )
}

pub fn mirror_url(commit_sha: &str, path: &str) -> String {
    format!(
        "https://cdn.jsdelivr.net/gh/{INDEX_OWNER}/{INDEX_NAME}@{commit_sha}/{}",
        encode_path(path)
    )
}

/// Rewrites a commit pinned raw.githubusercontent.com index URL to its jsDelivr equivalent
pub fn mirror_for(raw: &str) -> Option<String> {
    let prefix = format!("https://raw.githubusercontent.com/{INDEX_OWNER}/{INDEX_NAME}/");
    let (sha, path) = raw.strip_prefix(&prefix)?.split_once('/')?;
    if sha.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!(
        "https://cdn.jsdelivr.net/gh/{INDEX_OWNER}/{INDEX_NAME}@{sha}/{path}"
    ))
}

/// Best effort fix for the hand edited JSON mistakes seen in the index: trailing commas
/// and a string missing its closing quote before `,` + newline
pub fn repair_json(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len() + 8);
    let mut in_str = false;
    let mut escaped = false;
    for (i, &c) in chars.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
                out.push(c);
            } else if c == '\\' {
                escaped = true;
                out.push(c);
            } else if c == '"' {
                in_str = false;
                out.push(c);
            } else if c == '\r' || c == '\n' {
                let kept = out.trim_end_matches([' ', '\t']).len();
                if out[..kept].ends_with(',') {
                    out.truncate(kept - 1);
                    out.push_str("\",");
                    out.push(c);
                    in_str = false;
                } else {
                    out.push_str(if c == '\n' { "\\n" } else { "\\r" });
                }
            } else if (c as u32) < 0x20 {
                out.push_str(&format!("\\u{:04x}", c as u32));
            } else {
                out.push(c);
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == ',' {
            let next = chars[i + 1..].iter().find(|ch| !ch.is_whitespace());
            if !matches!(next, Some('}') | Some(']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse_meta(bytes: &[u8]) -> Result<ModMeta, String> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}');
    match serde_json::from_str::<ModMeta>(text) {
        Ok(meta) => Ok(meta),
        Err(first) => serde_json::from_str::<ModMeta>(&repair_json(text))
            .map_err(|second| format!("{first} (after repair: {second})")),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn meta_to_item(
    dir: &str,
    mut meta: ModMeta,
    description: String,
    thumb_path: Option<&str>,
    commit_sha: &str,
    prev: Option<&IndexEntry>,
    meta_changed: bool,
    initial_sync: bool,
    now: u64,
) -> ArchiveModItem {
    let (dir_author, dir_title) = dir.split_once('@').unwrap_or(("", dir));
    if meta.title.trim().is_empty() {
        meta.title = dir_title.to_string();
    }
    if meta.author.trim().is_empty() {
        meta.author = dir_author.to_string();
    }
    if meta.categories.is_empty() {
        meta.categories = vec!["Miscellaneous".to_string()];
    }
    meta.download_url = meta
        .download_url
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    if meta.last_updated == 0 {
        meta.last_updated = match prev {
            Some(p) if !meta_changed => p.item.meta.last_updated,
            _ if initial_sync => 0,
            _ => now,
        };
    }
    ArchiveModItem {
        dir_name: dir.to_string(),
        meta,
        description,
        image_url: thumb_path
            .map(|p| raw_url(commit_sha, p))
            .unwrap_or_default(),
        has_thumbnail: thumb_path.is_some(),
        ..Default::default()
    }
}

pub fn plan_sync(
    entries: &BTreeMap<String, IndexEntry>,
    groups: &BTreeMap<String, DirFiles>,
) -> SyncPlan {
    let mut plan = SyncPlan::default();
    for (dir, files) in groups {
        let prev = entries.get(dir);
        if let Some((path, sha)) = &files.meta
            && prev.is_none_or(|p| &p.meta_sha != sha)
        {
            plan.to_fetch
                .push((dir.clone(), FileKind::Meta, path.clone()));
        }
        if let Some((path, sha)) = &files.desc
            && prev.is_none_or(|p| p.desc_sha.as_ref() != Some(sha))
        {
            plan.to_fetch
                .push((dir.clone(), FileKind::Desc, path.clone()));
        }
        if let Some(p) = prev
            && p.thumb_sha.is_some()
            && p.thumb_sha.as_ref() != files.thumb.as_ref().map(|(_, s)| s)
        {
            plan.thumbs_changed.push(dir.clone());
        }
    }
    plan.removed_dirs = entries
        .keys()
        .filter(|k| !groups.contains_key(*k))
        .cloned()
        .collect();
    plan
}

/// Builds the new entry map from the tree and the fetched blobs. Returns false
/// when a network failure means the commit must not be marked as synced
fn build_entries(
    prev: &BTreeMap<String, IndexEntry>,
    groups: &BTreeMap<String, DirFiles>,
    fetched: &HashMap<(String, FileKind), FetchResult>,
    head: &str,
    initial_sync: bool,
    now: u64,
) -> (BTreeMap<String, IndexEntry>, bool) {
    let mut out = BTreeMap::new();
    let mut all_ok = true;
    for (dir, files) in groups {
        let Some((_, meta_sha)) = &files.meta else {
            continue;
        };
        let old = prev.get(dir);
        let keep_old = |out: &mut BTreeMap<String, IndexEntry>| {
            if let Some(o) = old {
                out.insert(dir.clone(), o.clone());
            }
        };

        let (meta, meta_changed) = match fetched.get(&(dir.clone(), FileKind::Meta)) {
            None => match old {
                Some(o) => (o.item.meta.clone(), false),
                None => continue,
            },
            Some(Ok(Some(bytes))) => match parse_meta(bytes) {
                Ok(meta) => (meta, true),
                Err(e) => {
                    log::warn!("Mod index: invalid meta.json in '{dir}': {e}");
                    keep_old(&mut out);
                    continue;
                }
            },
            Some(Ok(None)) => {
                log::warn!("Mod index: meta.json for '{dir}' was not found");
                keep_old(&mut out);
                continue;
            }
            Some(Err(e)) => {
                log::warn!("Mod index: failed to fetch meta.json for '{dir}': {e}");
                all_ok = false;
                keep_old(&mut out);
                continue;
            }
        };

        let (description, desc_sha) = match &files.desc {
            None => (String::new(), None),
            Some((_, sha)) => match fetched.get(&(dir.clone(), FileKind::Desc)) {
                None => (
                    old.map(|o| o.item.description.clone()).unwrap_or_default(),
                    Some(sha.clone()),
                ),
                Some(Ok(Some(bytes))) => (
                    String::from_utf8_lossy(bytes).into_owned(),
                    Some(sha.clone()),
                ),
                Some(Ok(None)) => (String::new(), Some(sha.clone())),
                Some(Err(e)) => {
                    log::warn!("Mod index: failed to fetch description.md for '{dir}': {e}");
                    all_ok = false;
                    (
                        old.map(|o| o.item.description.clone()).unwrap_or_default(),
                        old.and_then(|o| o.desc_sha.clone()),
                    )
                }
            },
        };

        let thumb_path = files.thumb.as_ref().map(|(p, _)| p.clone());
        let item = meta_to_item(
            dir,
            meta,
            description,
            thumb_path.as_deref(),
            head,
            old,
            meta_changed,
            initial_sync,
            now,
        );
        out.insert(
            dir.clone(),
            IndexEntry {
                meta_sha: meta_sha.clone(),
                desc_sha,
                thumb_path,
                thumb_sha: files.thumb.as_ref().map(|(_, s)| s.clone()),
                item,
            },
        );
    }
    (out, all_ok)
}

fn compact(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Resolves an `index://` id (or a legacy BMI id) against the catalog
pub fn resolve_index_id<'a>(id: &str, items: &'a [ArchiveModItem]) -> Option<&'a ArchiveModItem> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    if let Some(item) = items.iter().find(|i| i.dir_name == id) {
        return Some(item);
    }
    if let Some(item) = items.iter().find(|i| i.dir_name.eq_ignore_ascii_case(id)) {
        return Some(item);
    }
    let key = compact(id);
    if key.is_empty() {
        return None;
    }
    items.iter().find(|i| {
        let after_at = i.dir_name.split_once('@').map(|(_, t)| t).unwrap_or("");
        [
            i.dir_name.as_str(),
            after_at,
            i.meta.title.as_str(),
            i.meta.folder_name.as_str(),
        ]
        .iter()
        .any(|candidate| !candidate.is_empty() && compact(candidate) == key)
    })
}

pub fn strip_index_scheme(url: &str) -> Option<&str> {
    url.strip_prefix("index://")
        .or_else(|| url.strip_prefix("bmi://"))
}

pub fn sort_items(items: &mut [ArchiveModItem]) {
    items.sort_by(|a, b| {
        a.meta
            .title
            .to_lowercase()
            .cmp(&b.meta.title.to_lowercase())
            .then_with(|| a.dir_name.cmp(&b.dir_name))
    });
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

fn cache_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Balatro").join("mod_index_cache"))
}

fn load_cache_from(path: &Path) -> Option<IndexCache> {
    let text = std::fs::read_to_string(path).ok()?;
    let cache: IndexCache = serde_json::from_str(&text).ok()?;
    if cache.schema != CACHE_SCHEMA || cache.source != index_source_id() {
        log::info!("Mod index: discarding cache from another source or schema");
        return None;
    }
    Some(cache)
}

fn save_cache_to(path: &Path, cache: &IndexCache) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec(cache).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Removes caches written by the BMI era builds. Runs once per index source
pub fn migrate_legacy_caches() {
    if let Some(config) = dirs::config_dir() {
        migrate_legacy_caches_in(&config.join("Balatro"));
    }
}

fn migrate_legacy_caches_in(balatro_dir: &Path) -> bool {
    let index_dir = balatro_dir.join("mod_index_cache");
    let marker = index_dir.join(SOURCE_MARKER_FILE);
    let source = index_source_id();
    if std::fs::read_to_string(&marker).is_ok_and(|s| s.trim() == source) {
        return false;
    }
    if let Ok(entries) = std::fs::read_dir(&index_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == CACHE_FILE) {
                continue;
            }
            let res = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if let Err(e) = res {
                log::warn!("Failed to remove legacy cache {}: {e}", path.display());
            }
        }
    }
    for sub in ["mod_details", "descriptions"] {
        let dir = balatro_dir.join("mod_assets").join(sub);
        if dir.exists()
            && let Err(e) = std::fs::remove_dir_all(&dir)
        {
            log::warn!("Failed to remove legacy cache {}: {e}", dir.display());
        }
    }
    if let Err(e) =
        std::fs::create_dir_all(&index_dir).and_then(|_| std::fs::write(&marker, &source))
    {
        log::warn!("Failed to write catalog source marker: {e}");
    }
    log::info!("Cleared legacy mod catalog caches");
    true
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

async fn ls_refs_head(client: &reqwest::Client) -> Result<String, String> {
    let url = format!("https://github.com/{INDEX_OWNER}/{INDEX_NAME}.git/git-upload-pack");
    let resp = client
        .post(url)
        .header("Content-Type", "application/x-git-upload-pack-request")
        .header("Accept", "application/x-git-upload-pack-result")
        .header("Git-Protocol", "version=2")
        .body(build_ls_refs_body(INDEX_BRANCH))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("ls-refs HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    parse_ls_refs(&bytes, INDEX_BRANCH).ok_or_else(|| "ls-refs returned no head".to_string())
}

async fn api_head(client: &reqwest::Client) -> Result<String, String> {
    let url =
        format!("https://api.github.com/repos/{INDEX_OWNER}/{INDEX_NAME}/commits/{INDEX_BRANCH}");
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github.sha")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(api_error("commits", &resp));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let sha = text.trim();
    if is_sha(sha) {
        Ok(sha.to_ascii_lowercase())
    } else {
        Err("GitHub returned an invalid commit SHA".to_string())
    }
}

fn api_error(what: &str, resp: &reqwest::Response) -> String {
    let status = resp.status();
    let exhausted = resp
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim() == "0");
    if status == StatusCode::TOO_MANY_REQUESTS || (status == StatusCode::FORBIDDEN && exhausted) {
        format!("GitHub API rate limited while loading the mod index ({what})")
    } else {
        format!("GitHub API {what} request failed: HTTP {status}")
    }
}

async fn resolve_head(client: &reqwest::Client) -> Result<String, String> {
    match ls_refs_head(client).await {
        Ok(sha) => Ok(sha),
        Err(e) => {
            log::warn!("Mod index: ls-refs failed ({e}), falling back to the GitHub API");
            api_head(client).await
        }
    }
}

async fn fetch_tree(client: &reqwest::Client, commit_sha: &str) -> Result<TreeResp, String> {
    let url = format!(
        "https://api.github.com/repos/{INDEX_OWNER}/{INDEX_NAME}/git/trees/{commit_sha}?recursive=1"
    );
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(api_error("trees", &resp));
    }
    resp.json::<TreeResp>().await.map_err(|e| e.to_string())
}

async fn get_optional(client: &reqwest::Client, url: &str) -> Result<Option<Vec<u8>>, StatusOrNet> {
    let resp = client.get(url).send().await.map_err(|_| StatusOrNet::Net)?;
    let status = resp.status();
    if status.is_success() {
        return resp
            .bytes()
            .await
            .map(|b| Some(b.to_vec()))
            .map_err(|_| StatusOrNet::Net);
    }
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Err(StatusOrNet::Status(status))
}

enum StatusOrNet {
    Status(StatusCode),
    Net,
}

async fn fetch_file(client: &reqwest::Client, commit_sha: &str, path: &str) -> FetchResult {
    let raw = raw_url(commit_sha, path);
    for attempt in 0..2 {
        match get_optional(client, &raw).await {
            Ok(found) => return Ok(found),
            Err(StatusOrNet::Status(s))
                if s == StatusCode::FORBIDDEN || s == StatusCode::TOO_MANY_REQUESTS =>
            {
                break;
            }
            Err(StatusOrNet::Status(s)) if !s.is_server_error() => break,
            Err(_) if attempt == 0 => tokio::time::sleep(Duration::from_secs(1)).await,
            Err(_) => break,
        }
    }
    match get_optional(client, &mirror_url(commit_sha, path)).await {
        Ok(found) => Ok(found),
        Err(StatusOrNet::Status(s)) => Err(format!("HTTP {s} from raw and mirror")),
        Err(StatusOrNet::Net) => Err("network error on raw and mirror".to_string()),
    }
}

async fn fetch_all(
    client: &reqwest::Client,
    commit_sha: &str,
    to_fetch: &[(String, FileKind, String)],
) -> HashMap<(String, FileKind), FetchResult> {
    stream::iter(to_fetch.iter().cloned())
        .map(|(dir, kind, path)| async move {
            let result = fetch_file(client, commit_sha, &path).await;
            ((dir, kind), result)
        })
        .buffer_unordered(FETCH_CONCURRENCY)
        .collect()
        .await
}

/// Brings `cache` up to the current index head. Returns the titles whose cached
/// thumbnails are stale because the image changed
async fn refresh(
    cache: &mut IndexCache,
    client: &reqwest::Client,
    now: u64,
) -> Result<Vec<String>, String> {
    let head = resolve_head(client).await?;
    if cache.commit_sha.as_deref() == Some(head.as_str()) && !cache.entries.is_empty() {
        cache.synced_at = now;
        return Ok(Vec::new());
    }

    let tree = fetch_tree(client, &head).await?;
    if tree.truncated {
        return Err("GitHub returned a truncated tree for the mod index".to_string());
    }
    let groups = group_tree(&tree);
    let plan = plan_sync(&cache.entries, &groups);
    log::info!(
        "Mod index: head {head}, {} mods, fetching {} files, {} removed",
        groups.len(),
        plan.to_fetch.len(),
        plan.removed_dirs.len()
    );

    let fetched = fetch_all(client, &head, &plan.to_fetch).await;
    let initial_sync = cache.commit_sha.is_none();
    let (entries, all_ok) =
        build_entries(&cache.entries, &groups, &fetched, &head, initial_sync, now);

    // Thumbnails used to be keyed by title and are now keyed by catalog id, drop both
    let mut stale_titles: Vec<String> = Vec::new();
    for dir in &plan.thumbs_changed {
        stale_titles.push(dir.clone());
        for entry in [cache.entries.get(dir), entries.get(dir)]
            .into_iter()
            .flatten()
        {
            if !stale_titles.contains(&entry.item.meta.title) {
                stale_titles.push(entry.item.meta.title.clone());
            }
        }
    }

    cache.entries = entries;
    if all_ok {
        cache.commit_sha = Some(head);
    } else {
        log::warn!("Mod index: some files failed, they will be retried on the next sync");
    }
    cache.synced_at = now;
    Ok(stale_titles)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn remove_cached_thumbnails(titles: &[String]) {
    if titles.is_empty() {
        return;
    }
    let Ok((thumbs, _)) = crate::assets::ensure_assets_dirs() else {
        return;
    };
    for title in titles {
        let path = thumbs.join(format!("{}.jpg", safe_slug(title)));
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn load_or_empty(guard: &mut Option<IndexCache>) -> &mut IndexCache {
    guard.get_or_insert_with(|| {
        cache_dir()
            .map(|d| d.join(CACHE_FILE))
            .as_deref()
            .and_then(load_cache_from)
            .unwrap_or_else(IndexCache::empty)
    })
}

/// Whether a non-forced sync may skip the network and serve the cached catalog
fn cache_is_fresh(cache: &IndexCache, last_failed: u64, now: u64) -> bool {
    !cache.entries.is_empty()
        && (now.saturating_sub(cache.synced_at) < INDEX_MIN_REFRESH_SECS
            || now.saturating_sub(last_failed) < INDEX_FAILED_RETRY_SECS)
}

/// Returns the catalog, syncing with GitHub when the cache is older than a few minutes
/// (or always when `force` is set). Falls back to the cached catalog when offline
pub async fn sync_index(force: bool) -> Result<Vec<ArchiveModItem>, String> {
    let _sync = SYNC.lock().await;

    let now = now_secs();
    let (mut working, generation) = {
        let mut guard = INDEX.lock().await;
        let cache = load_or_empty(&mut guard);
        if !force && cache_is_fresh(cache, LAST_FAILED_SYNC.load(Ordering::Relaxed), now) {
            return Ok(cache.sorted_items());
        }
        (cache.clone(), CACHE_GENERATION.load(Ordering::Relaxed))
    };

    let result = refresh(&mut working, http_client(), now).await;

    let mut guard = INDEX.lock().await;
    let current = CACHE_GENERATION.load(Ordering::Relaxed) == generation;
    match result {
        Ok(stale) => {
            LAST_FAILED_SYNC.store(0, Ordering::Relaxed);
            set_last_error(None);
            if !current {
                return Ok(working.sorted_items());
            }
            remove_cached_thumbnails(&stale);
            if let Some(path) = cache_dir().map(|d| d.join(CACHE_FILE))
                && let Err(e) = save_cache_to(&path, &working)
            {
                log::warn!("Mod index: failed to persist cache: {e}");
            }
            let items = working.sorted_items();
            *guard = Some(working);
            Ok(items)
        }
        Err(e) if !working.entries.is_empty() => {
            LAST_FAILED_SYNC.store(now, Ordering::Relaxed);
            set_last_error(Some(e.clone()));
            log::warn!("Mod index: sync failed, serving cached catalog: {e}");
            Ok(working.sorted_items())
        }
        Err(e) => {
            LAST_FAILED_SYNC.store(now, Ordering::Relaxed);
            set_last_error(Some(e.clone()));
            Err(format!("Could not load the mod index from GitHub: {e}"))
        }
    }
}

/// Forgets the in-memory catalog so the next request reloads it from disk or GitHub
pub async fn reset_cache() {
    let mut guard = INDEX.lock().await;
    CACHE_GENERATION.fetch_add(1, Ordering::Relaxed);
    LAST_FAILED_SYNC.store(0, Ordering::Relaxed);
    set_last_error(None);
    *guard = None;
}

/// The catalog as currently known, syncing only when nothing is cached yet
pub async fn catalog_items() -> Result<Vec<ArchiveModItem>, String> {
    {
        let mut guard = INDEX.lock().await;
        let cache = load_or_empty(&mut guard);
        if !cache.entries.is_empty() {
            return Ok(cache.sorted_items());
        }
    }
    sync_index(false).await
}

fn download_url_of(item: &ArchiveModItem) -> Result<String, String> {
    item.meta
        .download_url
        .clone()
        .filter(|u| !u.is_empty())
        .ok_or_else(|| format!("'{}' has no download URL in the mod index", item.meta.title))
}

/// Turns `index://<dir>` (and legacy `bmi://<id>`) into the mod's real download URL
pub async fn resolve_install_url(url: &str) -> Result<String, String> {
    let Some(id) = strip_index_scheme(url) else {
        return Ok(url.to_string());
    };
    let id = id.trim();
    if id.is_empty() {
        return Err("Mod download is missing its index id".to_string());
    }
    let items = catalog_items().await?;
    if let Some(item) = resolve_index_id(id, &items) {
        return download_url_of(item);
    }
    let items = sync_index(true).await?;
    match resolve_index_id(id, &items) {
        Some(item) => download_url_of(item),
        None => Err(format!(
            "'{id}' is no longer in the mod index, refresh the catalog and reinstall"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const SHA: &str = "622e39aa9a29f8adec562c71aaa3e7213210a62a";

    fn blob(path: &str, sha: &str) -> TreeEntry {
        TreeEntry {
            path: path.into(),
            kind: "blob".into(),
            sha: sha.into(),
        }
    }

    fn item(dir: &str, title: &str, folder: &str) -> ArchiveModItem {
        ArchiveModItem {
            dir_name: dir.into(),
            meta: ModMeta {
                title: title.into(),
                folder_name: folder.into(),
                download_url: Some(format!("https://example.com/{title}.zip")),
                ..Default::default()
            },
            description: String::new(),
            image_url: String::new(),
            has_thumbnail: false,
            ..Default::default()
        }
    }

    fn entry(
        meta_sha: &str,
        desc_sha: Option<&str>,
        thumb_sha: Option<&str>,
        it: ArchiveModItem,
    ) -> IndexEntry {
        IndexEntry {
            meta_sha: meta_sha.into(),
            desc_sha: desc_sha.map(Into::into),
            thumb_path: thumb_sha.map(|_| format!("mods/{}/thumbnail.jpg", it.dir_name)),
            thumb_sha: thumb_sha.map(Into::into),
            item: it,
        }
    }

    #[test]
    fn ls_refs_body_matches_known_bytes_for_main() {
        assert_eq!(
            build_ls_refs_body("main"),
            b"0014command=ls-refs\n0001001fref-prefix refs/heads/main\n0000".to_vec()
        );
    }

    #[test]
    fn parse_ls_refs_returns_sha() {
        let body = b"003d622e39aa9a29f8adec562c71aaa3e7213210a62a refs/heads/main\n0000";
        assert_eq!(parse_ls_refs(body, "main").as_deref(), Some(SHA));
    }

    #[test]
    fn parse_ls_refs_none_on_empty_or_other_ref() {
        assert_eq!(parse_ls_refs(b"0000", "main"), None);
        assert_eq!(parse_ls_refs(b"", "main"), None);
        let other = b"003d622e39aa9a29f8adec562c71aaa3e7213210a62a refs/heads/mainx\n0000";
        assert_eq!(parse_ls_refs(other, "main"), None);
        assert_eq!(parse_ls_refs(b"zzzzgarbage", "main"), None);
    }

    #[test]
    fn group_tree_selects_meta_desc_thumb() {
        let tree = TreeResp {
            truncated: false,
            tree: vec![
                blob("mods/A@X/meta.json", "m1"),
                blob("mods/A@X/description.md", "d1"),
                blob("mods/A@X/thumbnail.jpg", "t1"),
                blob("mods/A@X/sub/x.json", "ignored"),
                blob("mods/Jaye@JokeyPony/meta.json", "m2"),
                blob("mods/Jaye@JokeyPony/Thumbnail.jpg", "t2"),
                blob("mods/misenrol@TerrariaXBalatro/meta.json", "m3"),
                blob("mods/misenrol@TerrariaXBalatro/logo.jpg", "t3"),
                blob("mods/OnlyDesc@Mod/description.md", "d4"),
                blob("schema/meta.schema.json", "s"),
                TreeEntry {
                    path: "mods/A@X".into(),
                    kind: "tree".into(),
                    sha: "tree".into(),
                },
            ],
        };
        let groups = group_tree(&tree);
        assert_eq!(groups.len(), 3);
        let a = &groups["A@X"];
        assert_eq!(a.meta, Some(("mods/A@X/meta.json".into(), "m1".into())));
        assert_eq!(
            a.desc,
            Some(("mods/A@X/description.md".into(), "d1".into()))
        );
        assert_eq!(
            a.thumb,
            Some(("mods/A@X/thumbnail.jpg".into(), "t1".into()))
        );
        assert_eq!(
            groups["Jaye@JokeyPony"]
                .thumb
                .as_ref()
                .map(|t| t.0.as_str()),
            Some("mods/Jaye@JokeyPony/Thumbnail.jpg")
        );
        assert_eq!(groups["Jaye@JokeyPony"].desc, None);
        assert_eq!(
            groups["misenrol@TerrariaXBalatro"]
                .thumb
                .as_ref()
                .map(|t| t.1.as_str()),
            Some("t3")
        );
        assert!(!groups.contains_key("OnlyDesc@Mod"));
    }

    #[test]
    fn tree_json_fixture_parses_and_groups() {
        let json = format!(
            r#"{{"sha":"{SHA}","url":"x","truncated":false,"tree":[
                {{"path":"mods","mode":"040000","type":"tree","sha":"a","url":"x"}},
                {{"path":"mods/B@Y/meta.json","mode":"100644","type":"blob","sha":"b1","size":200,"url":"x"}},
                {{"path":"mods/B@Y/thumbnail.png","mode":"100644","type":"blob","sha":"b2","size":9,"url":"x"}}
            ]}}"#
        );
        let tree: TreeResp = serde_json::from_str(&json).expect("tree json");
        let groups = group_tree(&tree);
        assert_eq!(
            groups["B@Y"].thumb.as_ref().map(|t| t.1.as_str()),
            Some("b2")
        );
    }

    #[test]
    fn pick_thumbnail_prefers_thumbnail_over_other_images() {
        assert_eq!(
            pick_thumbnail(&["a.png", "meta.json", "thumbnail.png"]),
            Some("thumbnail.png")
        );
        assert_eq!(
            pick_thumbnail(&["banner.jpg", "THUMBNAIL.JPG", "z.webp"]),
            Some("THUMBNAIL.JPG")
        );
        assert_eq!(
            pick_thumbnail(&["logo.jpg", "description.md"]),
            Some("logo.jpg")
        );
        assert_eq!(pick_thumbnail(&["meta.json", "description.md"]), None);
    }

    #[test]
    fn raw_url_encodes_segments() {
        assert_eq!(
            raw_url(SHA, "mods/SLDTyp0@Typ0's Random Jokers/meta.json"),
            format!(
                "https://raw.githubusercontent.com/kasimeka/balatro-mod-index/{SHA}/mods/SLDTyp0%40Typ0%27s%20Random%20Jokers/meta.json"
            )
        );
        assert!(
            raw_url(SHA, "mods/Kars&sup3p@HandsomeDevils/meta.json").contains("Kars%26sup3p%40")
        );
    }

    #[test]
    fn mirror_url_and_mirror_for_roundtrip() {
        let path = "mods/A B@C/thumbnail.jpg";
        assert_eq!(mirror_for(&raw_url(SHA, path)), Some(mirror_url(SHA, path)));
        assert_eq!(
            mirror_url(SHA, path),
            format!(
                "https://cdn.jsdelivr.net/gh/kasimeka/balatro-mod-index@{SHA}/mods/A%20B%40C/thumbnail.jpg"
            )
        );
        assert_eq!(mirror_for("https://example.com/x.jpg"), None);
    }

    #[test]
    fn parse_meta_strict_ok() {
        let json = br#"{
            "title": "Steamodded",
            "requires-steamodded": false,
            "requires-talisman": false,
            "categories": ["Technical", "API"],
            "author": "Steamodded",
            "repo": "https://github.com/Steamodded/smods",
            "downloadURL": "https://github.com/Steamodded/smods/archive/refs/heads/main.zip",
            "folderName": "smods",
            "automatic-version-check": true,
            "version": "1.0.0~BETA-0827c",
            "last-updated": 1760000000
        }"#;
        let meta = parse_meta(json).expect("valid meta");
        assert_eq!(meta.title, "Steamodded");
        assert_eq!(meta.folder_name, "smods");
        assert_eq!(meta.categories, vec!["Technical", "API"]);
        assert!(meta.automatic_version_check);
        assert_eq!(meta.last_updated, 1_760_000_000);
    }

    #[test]
    fn parse_meta_repairs_trailing_comma() {
        let json = "{\r\n  \"title\": \"Friends of Phoenix Wright\",\r\n  \"requires-steamodded\": true,\r\n  \"requires-talisman\": false,\r\n  \"categories\": [\"Content\",],\r\n  \"author\": \"Khasm\",\r\n  \"repo\": \"https://github.com/Khasm/x\",\r\n  \"downloadURL\": \"https://github.com/Khasm/x/archive/refs/heads/main.zip\",\r\n  \"version\": \"1.0\",\r\n}";
        let meta = parse_meta(json.as_bytes()).expect("repaired meta");
        assert_eq!(meta.title, "Friends of Phoenix Wright");
        assert_eq!(meta.categories, vec!["Content"]);
        assert!(meta.requires_steamodded);
    }

    #[test]
    fn parse_meta_repairs_unterminated_string() {
        let json = "{\n  \"title\": \"Quality\",\n  \"requires-steamodded\": true,\n  \"requires-talisman\": false,\n  \"categories\": [\"Content\"],\n  \"author\": \"Rageinater\",\n  \"repo\": \"https://github.com/RageinaterCode/Balatro-Quality\",\n  \"downloadURL\": \"https://github.com/RageinaterCode/Balatro-Quality/releases/latest/download/quality.zip,\n  \"version\": \"1.2\"\n}";
        let meta = parse_meta(json.as_bytes()).expect("repaired meta");
        assert_eq!(
            meta.download_url.as_deref(),
            Some(
                "https://github.com/RageinaterCode/Balatro-Quality/releases/latest/download/quality.zip"
            )
        );
        assert_eq!(meta.version, "1.2");
    }

    #[test]
    fn parse_meta_rejects_garbage() {
        assert!(parse_meta(b"not json at all").is_err());
        assert!(parse_meta(b"{\"title\": [}").is_err());
    }

    #[test]
    fn meta_to_item_maps_fields() {
        let meta = ModMeta {
            download_url: Some("  https://github.com/a/b/archive/main.zip ".into()),
            version: "1.0".into(),
            ..Default::default()
        };
        let it = meta_to_item(
            "Author@Cool Mod",
            meta,
            "desc".into(),
            Some("mods/Author@Cool Mod/thumbnail.jpg"),
            SHA,
            None,
            true,
            true,
            42,
        );
        assert_eq!(it.dir_name, "Author@Cool Mod");
        assert_eq!(it.meta.title, "Cool Mod");
        assert_eq!(it.meta.author, "Author");
        assert_eq!(it.meta.categories, vec!["Miscellaneous"]);
        assert_eq!(it.meta.folder_name, "");
        assert_eq!(
            it.meta.download_url.as_deref(),
            Some("https://github.com/a/b/archive/main.zip")
        );
        assert_eq!(it.description, "desc");
        assert!(it.has_thumbnail);
        assert_eq!(
            it.image_url,
            format!(
                "https://raw.githubusercontent.com/kasimeka/balatro-mod-index/{SHA}/mods/Author%40Cool%20Mod/thumbnail.jpg"
            )
        );

        let no_thumb = meta_to_item(
            "A@B",
            ModMeta {
                download_url: Some("   ".into()),
                ..Default::default()
            },
            String::new(),
            None,
            SHA,
            None,
            true,
            true,
            42,
        );
        assert!(!no_thumb.has_thumbnail);
        assert_eq!(no_thumb.image_url, "");
        assert_eq!(no_thumb.meta.download_url, None);
    }

    #[test]
    fn last_updated_rules() {
        let with_ts = ModMeta {
            last_updated: 7,
            ..Default::default()
        };
        let run = |meta: ModMeta, prev: Option<&IndexEntry>, changed: bool, initial: bool| {
            meta_to_item(
                "a@b",
                meta,
                String::new(),
                None,
                SHA,
                prev,
                changed,
                initial,
                99,
            )
            .meta
            .last_updated
        };
        assert_eq!(run(with_ts, None, true, true), 7);

        let mut prev_item = item("a@b", "b", "");
        prev_item.meta.last_updated = 55;
        let prev = entry("m", None, None, prev_item);
        assert_eq!(run(ModMeta::default(), Some(&prev), false, false), 55);
        assert_eq!(run(ModMeta::default(), None, true, true), 0);
        assert_eq!(run(ModMeta::default(), Some(&prev), true, false), 99);
        assert_eq!(run(ModMeta::default(), None, true, false), 99);
    }

    #[test]
    fn plan_sync_fetches_only_changed_and_detects_removed_and_thumb_changes() {
        let mut entries = BTreeMap::new();
        entries.insert(
            "same@x".to_string(),
            entry("m1", Some("d1"), Some("t1"), item("same@x", "Same", "")),
        );
        entries.insert(
            "desc@x".to_string(),
            entry("m2", Some("d2"), Some("t2"), item("desc@x", "Desc", "")),
        );
        entries.insert(
            "gone@x".to_string(),
            entry("m3", None, None, item("gone@x", "Gone", "")),
        );

        let files = |m: &str, d: Option<&str>, t: Option<&str>, dir: &str| DirFiles {
            meta: Some((format!("mods/{dir}/meta.json"), m.into())),
            desc: d.map(|s| (format!("mods/{dir}/description.md"), s.into())),
            thumb: t.map(|s| (format!("mods/{dir}/thumbnail.jpg"), s.into())),
        };
        let mut groups = BTreeMap::new();
        groups.insert(
            "same@x".to_string(),
            files("m1", Some("d1"), Some("t1"), "same@x"),
        );
        groups.insert(
            "desc@x".to_string(),
            files("m2", Some("d2b"), Some("t2b"), "desc@x"),
        );
        groups.insert("new@x".to_string(), files("m4", Some("d4"), None, "new@x"));

        let plan = plan_sync(&entries, &groups);
        let fetch: Vec<(&str, FileKind)> = plan
            .to_fetch
            .iter()
            .map(|(d, k, _)| (d.as_str(), *k))
            .collect();
        assert_eq!(
            fetch,
            vec![
                ("desc@x", FileKind::Desc),
                ("new@x", FileKind::Meta),
                ("new@x", FileKind::Desc)
            ]
        );
        assert_eq!(plan.removed_dirs, vec!["gone@x".to_string()]);
        assert_eq!(plan.thumbs_changed, vec!["desc@x".to_string()]);
    }

    #[test]
    fn build_entries_keeps_previous_on_failure_and_reports_network_errors() {
        let mut prev = BTreeMap::new();
        let mut old = item("keep@x", "Keep", "");
        old.description = "old desc".into();
        prev.insert("keep@x".to_string(), entry("m1", Some("d1"), None, old));

        let mut groups = BTreeMap::new();
        groups.insert(
            "keep@x".to_string(),
            DirFiles {
                meta: Some(("mods/keep@x/meta.json".into(), "m1b".into())),
                desc: Some(("mods/keep@x/description.md".into(), "d1".into())),
                thumb: None,
            },
        );
        groups.insert(
            "new@x".to_string(),
            DirFiles {
                meta: Some(("mods/new@x/meta.json".into(), "m2".into())),
                desc: Some(("mods/new@x/description.md".into(), "d2".into())),
                thumb: Some(("mods/new@x/thumbnail.jpg".into(), "t2".into())),
            },
        );
        let mut fetched: HashMap<(String, FileKind), FetchResult> = HashMap::new();
        fetched.insert(("keep@x".into(), FileKind::Meta), Err("boom".into()));
        fetched.insert(
            ("new@x".into(), FileKind::Meta),
            Ok(Some(
                br#"{"title":"New","downloadURL":"https://e.com/n.zip","version":"1"}"#.to_vec(),
            )),
        );
        fetched.insert(
            ("new@x".into(), FileKind::Desc),
            Ok(Some(b"hello".to_vec())),
        );

        let (out, all_ok) = build_entries(&prev, &groups, &fetched, SHA, false, 10);
        assert!(!all_ok);
        assert_eq!(out["keep@x"].meta_sha, "m1");
        assert_eq!(out["keep@x"].item.description, "old desc");
        assert_eq!(out["new@x"].item.description, "hello");
        assert_eq!(out["new@x"].item.meta.last_updated, 10);
        assert!(out["new@x"].item.has_thumbnail);
    }

    #[test]
    fn resolve_index_id_exact_case_insensitive_after_at_title_folder_and_not_found() {
        let items = vec![
            item("Steamodded@smods", "Steamodded", "smods"),
            item("Author@Cool Mod", "Cool Mod Deluxe", ""),
            item("Other@Thing", "Another Title", "FolderX"),
        ];
        let dir = |id: &str| resolve_index_id(id, &items).map(|i| i.dir_name.as_str());
        assert_eq!(dir("Steamodded@smods"), Some("Steamodded@smods"));
        assert_eq!(dir("steamodded@SMODS"), Some("Steamodded@smods"));
        assert_eq!(dir("cool-mod"), Some("Author@Cool Mod"));
        assert_eq!(dir("cool_mod_deluxe"), Some("Author@Cool Mod"));
        assert_eq!(dir("folderx"), Some("Other@Thing"));
        assert_eq!(dir("missing"), None);
        assert_eq!(dir("   "), None);
        assert_eq!(dir("@@"), None);
    }

    #[test]
    fn legacy_bmi_urls_resolve_through_the_catalog() {
        let items = vec![item("Steamodded@smods", "Steamodded", "smods")];
        for url in [
            "bmi://Steamodded@smods",
            "bmi://steamodded-smods",
            "index://Steamodded@smods",
        ] {
            let id = strip_index_scheme(url).expect("scheme");
            let found = resolve_index_id(id, &items).expect("resolves");
            assert_eq!(
                download_url_of(found).unwrap(),
                "https://example.com/Steamodded.zip"
            );
        }
    }

    #[test]
    fn strip_index_scheme_handles_bmi_and_index() {
        assert_eq!(strip_index_scheme("bmi://abc"), Some("abc"));
        assert_eq!(strip_index_scheme("index://A@B"), Some("A@B"));
        assert_eq!(strip_index_scheme("https://github.com/x.zip"), None);
    }

    #[test]
    fn sort_items_orders_by_title_case_insensitive_then_dir() {
        let mut items = vec![
            item("z@b", "beta", ""),
            item("b@a", "Alpha", ""),
            item("a@a", "alpha", ""),
        ];
        sort_items(&mut items);
        let dirs: Vec<&str> = items.iter().map(|i| i.dir_name.as_str()).collect();
        assert_eq!(dirs, vec!["a@a", "b@a", "z@b"]);
    }

    #[test]
    fn cache_is_fresh_backs_off_after_a_failed_sync() {
        let mut cache = IndexCache::empty();
        let now = 10_000;
        cache.synced_at = now - INDEX_MIN_REFRESH_SECS - 1;
        assert!(!cache_is_fresh(&cache, 0, now));

        cache
            .entries
            .insert("a@b".into(), entry("m", None, None, item("a@b", "B", "")));
        assert!(!cache_is_fresh(&cache, 0, now));
        assert!(cache_is_fresh(&cache, now - 5, now));
        assert!(!cache_is_fresh(&cache, now - INDEX_FAILED_RETRY_SECS, now));

        cache.synced_at = now - 1;
        assert!(cache_is_fresh(&cache, 0, now));

        cache.entries.clear();
        assert!(!cache_is_fresh(&cache, now - 5, now));
    }

    #[test]
    fn cache_roundtrip_and_source_mismatch_discarded() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join(CACHE_FILE);
        let mut cache = IndexCache::empty();
        cache.commit_sha = Some(SHA.into());
        cache
            .entries
            .insert("a@b".into(), entry("m", None, None, item("a@b", "B", "")));
        save_cache_to(&path, &cache).unwrap();
        let loaded = load_cache_from(&path).expect("loads");
        assert_eq!(loaded.commit_sha.as_deref(), Some(SHA));
        assert_eq!(loaded.entries.len(), 1);

        cache.source = "github:someone/else@main".into();
        save_cache_to(&path, &cache).unwrap();
        assert!(load_cache_from(&path).is_none());

        cache.source = index_source_id();
        cache.schema = CACHE_SCHEMA + 1;
        save_cache_to(&path, &cache).unwrap();
        assert!(load_cache_from(&path).is_none());
    }

    #[test]
    fn migrate_legacy_caches_removes_bmi_files_once() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let index_dir = base.join("mod_index_cache");
        std::fs::create_dir_all(&index_dir).unwrap();
        std::fs::write(index_dir.join("bmi_mods_cache_default.json"), "{}").unwrap();
        std::fs::write(index_dir.join("mod_index_archive.json"), "{}").unwrap();
        std::fs::write(index_dir.join(CACHE_FILE), "{}").unwrap();
        let details = base.join("mod_assets").join("mod_details");
        let thumbs = base.join("mod_assets").join("thumbnails");
        std::fs::create_dir_all(&details).unwrap();
        std::fs::create_dir_all(&thumbs).unwrap();
        std::fs::write(thumbs.join("x.jpg"), "img").unwrap();

        assert!(migrate_legacy_caches_in(base));
        assert!(!index_dir.join("bmi_mods_cache_default.json").exists());
        assert!(!index_dir.join("mod_index_archive.json").exists());
        assert!(index_dir.join(CACHE_FILE).exists());
        assert!(!details.exists());
        assert!(thumbs.join("x.jpg").exists());

        std::fs::write(index_dir.join("bmi_mods_cache_x.json"), "{}").unwrap();
        assert!(!migrate_legacy_caches_in(base));
        assert!(index_dir.join("bmi_mods_cache_x.json").exists());
    }

    /// Hits the real index on GitHub: `cargo test live_index -- --ignored --nocapture`
    #[test]
    #[ignore = "network: fetches the live kasimeka/balatro-mod-index"]
    fn live_index_smoke() {
        tauri::async_runtime::block_on(async {
            let client = http_client();
            let head = ls_refs_head(client).await.expect("ls-refs head");
            assert!(is_sha(&head));

            let mut cache = IndexCache::empty();
            refresh(&mut cache, client, now_secs()).await.expect("sync");
            let items = cache.sorted_items();
            println!("live index: {} mods at {:?}", items.len(), cache.commit_sha);
            assert!(items.len() > 300, "only {} mods parsed", items.len());

            let bad_urls: Vec<&str> = items
                .iter()
                .filter(|i| {
                    !i.meta
                        .download_url
                        .as_deref()
                        .and_then(|u| reqwest::Url::parse(u).ok())
                        .is_some_and(|u| matches!(u.scheme(), "http" | "https"))
                })
                .map(|i| i.dir_name.as_str())
                .collect();
            assert!(bad_urls.is_empty(), "invalid downloadURL in {bad_urls:?}");

            let with_desc = items
                .iter()
                .filter(|i| !i.description.trim().is_empty())
                .count();
            assert!(with_desc > 300, "only {with_desc} descriptions");

            let thumbs: Vec<&ArchiveModItem> = items.iter().filter(|i| i.has_thumbnail).collect();
            assert!(thumbs.len() > 200);
            for it in thumbs.iter().step_by(thumbs.len() / 3).take(3) {
                let resp = client
                    .head(&it.image_url)
                    .send()
                    .await
                    .expect("HEAD thumbnail");
                assert!(
                    resp.status().is_success(),
                    "{} -> {}",
                    it.image_url,
                    resp.status()
                );
                let mirror = mirror_for(&it.image_url).expect("mirror url");
                let resp = client.head(&mirror).send().await.expect("HEAD mirror");
                assert!(resp.status().is_success(), "{mirror} -> {}", resp.status());
            }

            let smods = resolve_index_id("Steamodded@smods", &items)
                .or_else(|| items.iter().find(|i| i.meta.title == "Steamodded"))
                .expect("Steamodded is in the index");
            assert!(download_url_of(smods).is_ok());
        });
    }
}
