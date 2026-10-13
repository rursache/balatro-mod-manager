//! The mod catalog shown to users: the GitHub mod index merged with Thunderstore
//!
//! Index entries keep their `Author@Title` id, Thunderstore only entries use
//! `ts:Namespace-Name` and install through `ts://Namespace-Name[@version]`

use std::collections::{HashMap, HashSet};

use once_cell::sync::Lazy;
use serde::Serialize;

use crate::mod_index;
use crate::models::{ArchiveModItem, CatalogSource, ModMeta};
use crate::thunderstore::{self, TsPackage};

pub const TS_ID_PREFIX: &str = "ts:";
pub const TS_URL_SCHEME: &str = "ts://";
pub const STEAMODDED_ID: &str = "Steamodded@smods";

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SourceStatus {
    pub ok: bool,
    pub synced_at: u64,
    pub error: Option<String>,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CatalogStatus {
    pub index: SourceStatus,
    pub thunderstore: SourceStatus,
}

#[derive(Default)]
struct CatalogState {
    items: Vec<ArchiveModItem>,
    index_count: usize,
    thunderstore_count: usize,
}

static CATALOG: Lazy<tokio::sync::Mutex<Option<CatalogState>>> =
    Lazy::new(|| tokio::sync::Mutex::new(None));

// ---------------------------------------------------------------------------
// Ids and URLs
// ---------------------------------------------------------------------------

pub fn ts_catalog_id(full_name: &str) -> String {
    format!("{TS_ID_PREFIX}{full_name}")
}

pub fn ts_install_url(full_name: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("{TS_URL_SCHEME}{full_name}@{v}"),
        None => format!("{TS_URL_SCHEME}{full_name}"),
    }
}

/// `Namespace-Name[@version]` from a `ts://` URL or `ts:` id
pub fn parse_ts_ref(reference: &str) -> Option<(String, Option<String>)> {
    let reference = reference.trim();
    let rest = reference
        .strip_prefix(TS_URL_SCHEME)
        .or_else(|| reference.strip_prefix(TS_ID_PREFIX))
        .unwrap_or(reference)
        .trim_end_matches('/');
    let (full_name, version) = match rest.split_once('@') {
        Some((n, v)) => (n, Some(v.trim())),
        None => (rest, None),
    };
    let (namespace, name) = thunderstore::split_full_name(full_name)?;
    if version.is_some_and(|v| thunderstore::parse_version(v).is_none()) {
        return None;
    }
    Some((format!("{namespace}-{name}"), version.map(str::to_string)))
}

fn compact(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// `host/owner/repo` for GitHub, GitLab and Codeberg URLs, lowercase without `.git`,
/// `www.` or anything after the repo segment
pub fn normalize_repo_url(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let mut parts = rest.split(['/', '?', '#']);
    let host = match parts.next()? {
        "github.com" | "raw.githubusercontent.com" | "codeload.github.com" => "github.com",
        "gitlab.com" => "gitlab.com",
        "codeberg.org" => "codeberg.org",
        _ => return None,
    };
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let repo = parts.next()?.trim_end_matches(".git");
    if repo.is_empty() {
        return None;
    }
    Some(format!("{host}/{owner}/{repo}"))
}

/// The GitHub release of the Multiplayer mod is a stub that crashes on purpose, only the
/// official Thunderstore packages may be installed
pub fn is_blocked_download(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let file = lower
        .split(['?', '#'])
        .next()
        .and_then(|u| u.trim_end_matches('/').rsplit('/').next())
        .unwrap_or("");
    if matches!(
        file,
        "balatromultiplayer.zip" | "balatromultiplayer-raw.zip"
    ) {
        return true;
    }
    let path = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let path = path.strip_prefix("www.").unwrap_or(path);
    [
        "github.com/balatro-multiplayer/",
        "codeload.github.com/balatro-multiplayer/",
        "api.github.com/repos/balatro-multiplayer/",
        "raw.githubusercontent.com/balatro-multiplayer/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn is_blocked_index_item(item: &ArchiveModItem) -> bool {
    is_blocked_download(&item.meta.repo)
        || item
            .meta
            .download_url
            .as_deref()
            .is_some_and(is_blocked_download)
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

fn index_repo_key(item: &ArchiveModItem) -> Option<String> {
    normalize_repo_url(&item.meta.repo).or_else(|| {
        item.meta
            .download_url
            .as_deref()
            .and_then(normalize_repo_url)
    })
}

fn index_title_keys(item: &ArchiveModItem) -> Vec<String> {
    let after_at = item.dir_name.split_once('@').map_or("", |(_, t)| t);
    [item.meta.title.as_str(), after_at]
        .iter()
        .map(|s| compact(s))
        .filter(|s| !s.is_empty())
        .collect()
}

fn owner_matches(owner: &str, item: &ArchiveModItem) -> bool {
    let owner = compact(owner);
    if owner.is_empty() {
        return false;
    }
    let dir_author = item.dir_name.split_once('@').map_or("", |(a, _)| a);
    std::iter::once(item.meta.author.as_str())
        .chain(item.meta.author.split([',', '&', '/']))
        .chain(std::iter::once(dir_author))
        .any(|author| compact(author) == owner)
}

/// Thunderstore only catalog entry
pub fn package_item(pkg: &TsPackage) -> Option<ArchiveModItem> {
    let info = thunderstore::package_info(pkg)?;
    let latest = thunderstore::latest_active_version(pkg)?;
    let deps: Vec<thunderstore::TsDependency> = latest
        .dependencies
        .iter()
        .filter_map(|d| thunderstore::parse_dependency(d))
        .collect();
    let meta = ModMeta {
        title: thunderstore::display_title(&pkg.name),
        author: pkg.owner.clone(),
        repo: normalize_repo_url(&info.website_url)
            .map(|_| info.website_url.clone())
            .unwrap_or_default(),
        download_url: Some(ts_install_url(&info.full_name, None)),
        folder_name: info.full_name.clone(),
        version: info.latest_version.clone(),
        categories: thunderstore::map_categories(&pkg.categories),
        last_updated: thunderstore::parse_timestamp(&pkg.date_updated).unwrap_or(0),
        requires_steamodded: deps
            .iter()
            .any(|d| thunderstore::is_steamodded(&d.full_name())),
        requires_talisman: deps.iter().any(|d| d.name.eq_ignore_ascii_case("Talisman")),
        automatic_version_check: false,
    };
    Some(ArchiveModItem {
        dir_name: ts_catalog_id(&info.full_name),
        meta,
        description: info.description.clone(),
        image_url: info.icon_url.clone(),
        has_thumbnail: !info.icon_url.is_empty(),
        source: CatalogSource::Thunderstore,
        downloads: Some(info.downloads_total),
        thunderstore: Some(info),
    })
}

/// Adds the Thunderstore side to an index entry, index fields win where both exist
fn attach(item: &mut ArchiveModItem, pkg: &TsPackage) -> bool {
    let Some(info) = thunderstore::package_info(pkg) else {
        return false;
    };
    if !item.has_thumbnail && !info.icon_url.is_empty() {
        item.image_url = info.icon_url.clone();
        item.has_thumbnail = true;
    }
    if item.description.trim().is_empty() {
        item.description = info.description.clone();
    }
    // Miscellaneous is the catch all, it only adds noise next to the index categories
    let had_categories = !item.meta.categories.is_empty();
    for cat in thunderstore::map_categories(&pkg.categories) {
        if !(had_categories && cat == "Miscellaneous") && !item.meta.categories.contains(&cat) {
            item.meta.categories.push(cat);
        }
    }
    let ts_updated = thunderstore::parse_timestamp(&pkg.date_updated).unwrap_or(0);
    item.meta.last_updated = item.meta.last_updated.max(ts_updated);
    item.downloads = Some(info.downloads_total);
    item.source = CatalogSource::Merged;
    item.thunderstore = Some(info);
    true
}

/// One entry per mod: repo URL match first, then compacted name plus owner equal to the
/// index author. Deprecated packages are never merged, Lovely is never listed, Steamodded
/// packages alias the index Steamodded entry and Multiplayer packages are left to their
/// own section
pub fn merge(index: Vec<ArchiveModItem>, packages: &[TsPackage]) -> Vec<ArchiveModItem> {
    let mut items: Vec<ArchiveModItem> = index
        .into_iter()
        .filter(|i| !is_blocked_index_item(i))
        .collect();

    let mut by_repo: HashMap<String, usize> = HashMap::new();
    for (pos, item) in items.iter().enumerate() {
        if let Some(key) = index_repo_key(item) {
            by_repo.entry(key).or_insert(pos);
        }
    }
    let smods = items.iter().position(|i| i.dir_name == STEAMODDED_ID);

    let mut claimed: HashSet<usize> = HashSet::new();
    let mut ts_only: Vec<ArchiveModItem> = Vec::new();
    for pkg in packages {
        if thunderstore::is_lovely(&pkg.full_name) || thunderstore::is_multiplayer(pkg) {
            continue;
        }
        if thunderstore::is_steamodded(&pkg.full_name) {
            if pkg.full_name == thunderstore::STEAMODDED_PACKAGE
                && !pkg.is_deprecated
                && let Some(pos) = smods
                && claimed.insert(pos)
            {
                attach(&mut items[pos], pkg);
            }
            continue;
        }
        let Some(latest) = thunderstore::latest_active_version(pkg) else {
            continue;
        };
        if !pkg.is_deprecated {
            let by_repo_match = normalize_repo_url(&latest.website_url)
                .and_then(|key| by_repo.get(&key).copied())
                .filter(|pos| !claimed.contains(pos));
            let name = compact(&pkg.name);
            let target = by_repo_match.or_else(|| {
                (0..items.len()).find(|pos| {
                    !claimed.contains(pos)
                        && index_title_keys(&items[*pos]).contains(&name)
                        && owner_matches(&pkg.owner, &items[*pos])
                })
            });
            if let Some(pos) = target
                && attach(&mut items[pos], pkg)
            {
                claimed.insert(pos);
                continue;
            }
        }
        if let Some(item) = package_item(pkg) {
            ts_only.push(item);
        }
    }

    items.extend(ts_only);
    mod_index::sort_items(&mut items);
    disambiguate_titles(&mut items);
    items
}

/// Titles are the frontend's mod key and the name of the installed row, so a Thunderstore
/// only package sharing a title with another entry is listed as "Title (Namespace)". The
/// index entry keeps the plain title, else the package that sorts first by id
fn disambiguate_titles(items: &mut [ArchiveModItem]) {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (pos, item) in items.iter().enumerate() {
        let key = compact(&item.meta.title);
        if !key.is_empty() {
            groups.entry(key).or_default().push(pos);
        }
    }
    for positions in groups.values().filter(|p| p.len() > 1) {
        let owner = positions
            .iter()
            .copied()
            .find(|&p| items[p].source != CatalogSource::Thunderstore)
            .or_else(|| {
                positions
                    .iter()
                    .copied()
                    .min_by(|&a, &b| items[a].dir_name.cmp(&items[b].dir_name))
            });
        for &pos in positions {
            if Some(pos) == owner || items[pos].source != CatalogSource::Thunderstore {
                continue;
            }
            let Some(namespace) = items[pos]
                .thunderstore
                .as_ref()
                .map(|t| t.namespace.clone())
                .filter(|n| !n.is_empty())
            else {
                continue;
            };
            let title = &mut items[pos].meta.title;
            *title = format!("{title} ({namespace})");
        }
    }
}

/// Official Multiplayer packages, kept out of the general catalog
pub fn multiplayer_packages_in(packages: &[TsPackage]) -> Vec<TsPackage> {
    packages
        .iter()
        .filter(|p| thunderstore::is_multiplayer(p))
        .cloned()
        .collect()
}

// Consumed by the Multiplayer section
#[allow(dead_code)]
pub async fn multiplayer_packages() -> Result<Vec<TsPackage>, String> {
    Ok(multiplayer_packages_in(
        &thunderstore::cached_packages().await?,
    ))
}

// ---------------------------------------------------------------------------
// Sync and lookups
// ---------------------------------------------------------------------------

/// Syncs the GitHub index and Thunderstore concurrently and returns the merged catalog,
/// one failing source still yields the other
pub async fn sync_all(force: bool) -> Result<Vec<ArchiveModItem>, String> {
    let (index, packages) = tokio::join!(mod_index::sync_index(force), thunderstore::sync(force));
    store(index, packages).await
}

async fn store(
    index: Result<Vec<ArchiveModItem>, String>,
    packages: Result<Vec<TsPackage>, String>,
) -> Result<Vec<ArchiveModItem>, String> {
    let (index, packages) = match (index, packages) {
        (Err(a), Err(b)) => return Err(format!("{a}; {b}")),
        (index, packages) => {
            if let Err(e) = &index {
                log::warn!("Catalog: serving Thunderstore only, {e}");
            }
            if let Err(e) = &packages {
                log::warn!("Catalog: serving the mod index only, {e}");
            }
            (index.unwrap_or_default(), packages.unwrap_or_default())
        }
    };
    let state = CatalogState {
        index_count: index.len(),
        thunderstore_count: packages.len(),
        items: merge(index, &packages),
    };
    let items = state.items.clone();
    *CATALOG.lock().await = Some(state);
    Ok(items)
}

/// The merged catalog as currently known, built from the source caches (each syncs only
/// when it has nothing cached) when no sync ran yet
pub async fn catalog_items() -> Result<Vec<ArchiveModItem>, String> {
    if let Some(state) = CATALOG.lock().await.as_ref()
        && !state.items.is_empty()
    {
        return Ok(state.items.clone());
    }
    let (index, packages) =
        tokio::join!(mod_index::catalog_items(), thunderstore::cached_packages());
    store(index, packages).await
}

pub async fn status() -> CatalogStatus {
    let (index_count, thunderstore_count) = CATALOG
        .lock()
        .await
        .as_ref()
        .map_or((0, 0), |s| (s.index_count, s.thunderstore_count));
    let source = |synced_at: u64, error: Option<String>, count: usize| SourceStatus {
        ok: error.is_none() && synced_at > 0,
        synced_at,
        error,
        count,
    };
    CatalogStatus {
        index: source(
            mod_index::synced_at().await,
            mod_index::last_sync_error(),
            index_count,
        ),
        thunderstore: source(
            thunderstore::synced_at().await,
            thunderstore::last_sync_error(),
            thunderstore_count,
        ),
    }
}

pub async fn reset_cache() {
    *CATALOG.lock().await = None;
    mod_index::reset_cache().await;
    thunderstore::reset_cache().await;
}

/// Looks up by catalog id, Thunderstore `Namespace-Name` (with or without `ts:`), title
/// (index entries first) and finally the fuzzy index id rules
pub fn find_in<'a>(items: &'a [ArchiveModItem], key: &str) -> Option<&'a ArchiveModItem> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    if let Some(item) = items.iter().find(|i| i.dir_name == key) {
        return Some(item);
    }
    if let Some((full_name, _)) = parse_ts_ref(key) {
        if thunderstore::is_steamodded(&full_name) {
            return items.iter().find(|i| i.dir_name == STEAMODDED_ID);
        }
        if let Some(item) = items.iter().find(|i| {
            i.thunderstore
                .as_ref()
                .is_some_and(|t| t.full_name.eq_ignore_ascii_case(&full_name))
        }) {
            return Some(item);
        }
    }
    items
        .iter()
        .filter(|i| i.source != CatalogSource::Thunderstore)
        .find(|i| i.meta.title.eq_ignore_ascii_case(key))
        .or_else(|| {
            items
                .iter()
                .find(|i| i.meta.title.eq_ignore_ascii_case(key))
        })
        .or_else(|| mod_index::resolve_index_id(key, items))
}

pub async fn find_item(key: &str) -> Option<ArchiveModItem> {
    let items = catalog_items().await.ok()?;
    find_in(&items, key).cloned()
}

/// A `ts://` reference resolved against the listing, with the package's canonical names
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsTarget {
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub download_url: String,
}

/// Resolves `ts://Namespace-Name[@version]` to an active version, Ok(None) when the package
/// is not in `packages`
pub fn resolve_ts_target_in(
    reference: &str,
    packages: &[TsPackage],
) -> Result<Option<TsTarget>, String> {
    let (full_name, version) = parse_ts_ref(reference)
        .ok_or_else(|| format!("'{reference}' is not a valid Thunderstore package reference"))?;
    if thunderstore::is_lovely(&full_name) {
        return Err("Lovely is managed by the app and is not installed from Thunderstore".into());
    }
    if thunderstore::is_steamodded(&full_name) {
        return Err("Steamodded is installed through the Steamodded installer".into());
    }
    let Some(pkg) = thunderstore::find_package(packages, &full_name) else {
        return Ok(None);
    };
    let version = match version {
        Some(v) => thunderstore::find_version(pkg, &v)
            .ok_or_else(|| format!("{full_name} has no active version {v}"))?,
        None => thunderstore::latest_active_version(pkg)
            .ok_or_else(|| format!("{full_name} has no active version"))?,
    };
    let (namespace, name) = thunderstore::split_full_name(&pkg.full_name)
        .ok_or_else(|| format!("invalid package name '{}'", pkg.full_name))?;
    Ok(Some(TsTarget {
        namespace: namespace.to_string(),
        name: name.to_string(),
        version: version.version_number.clone(),
        download_url: thunderstore::download_url(namespace, name, &version.version_number),
    }))
}

/// Official download URL of `ts://Namespace-Name[@version]`, Ok(None) when the package is
/// not in `packages`
#[cfg(test)]
pub fn resolve_ts_in(reference: &str, packages: &[TsPackage]) -> Result<Option<String>, String> {
    Ok(resolve_ts_target_in(reference, packages)?.map(|t| t.download_url))
}

pub async fn resolve_ts_target(reference: &str) -> Result<TsTarget, String> {
    if let Some(target) = resolve_ts_target_in(reference, &thunderstore::cached_packages().await?)?
    {
        return Ok(target);
    }
    resolve_ts_target_in(reference, &thunderstore::sync(true).await?)?.ok_or_else(|| {
        format!("'{reference}' is no longer on Thunderstore, refresh the catalog and reinstall")
    })
}

/// Turns `index://`, legacy `bmi://` and `ts://` URLs into real download URLs, other URLs
/// pass through. The Multiplayer GitHub release is always refused
pub async fn resolve_install_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    let resolved = if url.starts_with(TS_URL_SCHEME) {
        resolve_ts_target(url).await?.download_url
    } else if mod_index::strip_index_scheme(url).is_some() {
        mod_index::resolve_install_url(url).await?
    } else {
        url.to_string()
    };
    if is_blocked_download(&resolved) {
        return Err(
            "Multiplayer can only be installed from its official Thunderstore packages".into(),
        );
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thunderstore::tests::fixture_packages;

    fn index_item(dir: &str, title: &str, author: &str, repo: &str) -> ArchiveModItem {
        ArchiveModItem {
            dir_name: dir.into(),
            meta: ModMeta {
                title: title.into(),
                author: author.into(),
                repo: repo.into(),
                download_url: Some(format!("{repo}/archive/refs/heads/main.zip")),
                categories: vec!["Content".into()],
                last_updated: 5,
                ..Default::default()
            },
            description: format!("{title} from the index"),
            image_url: format!("https://raw.githubusercontent.com/x/y/z/{title}.jpg"),
            has_thumbnail: true,
            ..Default::default()
        }
    }

    fn index_fixture() -> Vec<ArchiveModItem> {
        vec![
            index_item(
                "Steamodded@smods",
                "Steamodded",
                "Steamodded",
                "https://github.com/Steamodded/smods",
            ),
            index_item(
                "InertSteak@Pokermon",
                "Pokermon",
                "InertSteak",
                "https://github.com/InertSteak/Pokermon",
            ),
            index_item(
                "Sonfive@PokermonPlus",
                "PokermonPlus",
                "Sonfive",
                "https://github.com/SonfiveTV/PokermonPlus",
            ),
            index_item(
                "frostice482@Amulet",
                "Amulet",
                "frostice482",
                "https://github.com/frostice482/amulet",
            ),
        ]
    }

    fn by_id<'a>(items: &'a [ArchiveModItem], id: &str) -> Option<&'a ArchiveModItem> {
        items.iter().find(|i| i.dir_name == id)
    }

    #[test]
    fn normalize_repo_url_variants() {
        let n = |u: &str| normalize_repo_url(u);
        assert_eq!(
            n("https://github.com/InertSteak/Pokermon").as_deref(),
            Some("github.com/inertsteak/pokermon")
        );
        for u in [
            "https://www.github.com/InertSteak/Pokermon.git",
            "http://github.com/inertsteak/pokermon/tree/main/src",
            "github.com/InertSteak/Pokermon/",
            "https://github.com/InertSteak/Pokermon/archive/refs/heads/main.zip",
            "https://github.com/InertSteak/Pokermon?tab=readme",
        ] {
            assert_eq!(
                n(u).as_deref(),
                Some("github.com/inertsteak/pokermon"),
                "{u}"
            );
        }
        assert_eq!(
            n("https://gitlab.com/Owner/Repo/-/tree/main").as_deref(),
            Some("gitlab.com/owner/repo")
        );
        assert_eq!(
            n("https://codeberg.org/a/b.git").as_deref(),
            Some("codeberg.org/a/b")
        );
        assert_eq!(n("https://smods.dev"), None);
        assert_eq!(n("https://github.nh6574.com/JokerDisplay"), None);
        assert_eq!(n("https://github.com/OnlyOwner"), None);
        assert_eq!(n(""), None);
    }

    #[test]
    fn merge_matches_by_repo_and_rejects_pokermonplus() {
        let items = merge(index_fixture(), &fixture_packages());

        let pokermon = by_id(&items, "InertSteak@Pokermon").unwrap();
        assert_eq!(pokermon.source, CatalogSource::Merged);
        assert_eq!(
            pokermon.thunderstore.as_ref().unwrap().full_name,
            "Pokermon-Pokermon"
        );

        // Deprecated, same compacted name as InertSteak@Pokermon but another owner, and its
        // repo matches Sonfive@PokermonPlus which must stay unmerged too
        let plus = by_id(&items, "ts:PokermonPlus-Pokermon").expect("standalone");
        assert_eq!(plus.source, CatalogSource::Thunderstore);
        assert_eq!(plus.meta.title, "Pokermon (PokermonPlus)");
        assert!(plus.thunderstore.as_ref().unwrap().deprecated);
        let sonfive = by_id(&items, "Sonfive@PokermonPlus").unwrap();
        assert_eq!(sonfive.source, CatalogSource::Index);
        assert!(sonfive.thunderstore.is_none());

        let amulet = by_id(&items, "frostice482@Amulet").unwrap();
        assert_eq!(amulet.source, CatalogSource::Merged);
        assert!(by_id(&items, "ts:just_frostice482-Amulet").is_none());
    }

    #[test]
    fn merge_name_fallback_needs_the_same_owner() {
        let index = vec![
            index_item("nh6574@JokerDisplay", "Joker Display", "nh6574", ""),
            index_item("Someone@Cryptid", "Cryptid", "Someone", ""),
        ];
        let items = merge(index, &fixture_packages());
        let joker = by_id(&items, "nh6574@JokerDisplay").unwrap();
        assert_eq!(joker.source, CatalogSource::Merged);
        assert!(by_id(&items, "ts:nh6574-JokerDisplay").is_none());

        assert_eq!(
            by_id(&items, "Someone@Cryptid").unwrap().source,
            CatalogSource::Index
        );
        assert!(by_id(&items, "ts:spectralpack-Cryptid").is_some());
    }

    #[test]
    fn merge_aliases_steamodded_and_hides_lovely_and_multiplayer() {
        let items = merge(index_fixture(), &fixture_packages());
        let smods = by_id(&items, STEAMODDED_ID).unwrap();
        assert_eq!(smods.source, CatalogSource::Merged);
        assert_eq!(
            smods.thunderstore.as_ref().unwrap().full_name,
            thunderstore::STEAMODDED_PACKAGE
        );
        assert_eq!(
            smods.meta.download_url.as_deref(),
            Some("https://github.com/Steamodded/smods/archive/refs/heads/main.zip")
        );
        for hidden in [
            "ts:Steamodded-Steamodded",
            "ts:Steamopollys-Steamodded",
            "ts:Thunderstore-lovely",
            "ts:BalatroMultiplayer-Multiplayer",
            "ts:BalatroMultiplayer-MultiplayerAPI",
            "ts:BalatroMultiplayer-MultiplayerSpeedrun",
        ] {
            assert!(
                by_id(&items, hidden).is_none(),
                "{hidden} must not be listed"
            );
        }
        assert!(
            items
                .iter()
                .all(|i| !i.dir_name.to_lowercase().contains("multiplayer"))
        );

        // Without an index entry Steamodded is not listed at all
        let items = merge(Vec::new(), &fixture_packages());
        assert!(items.iter().all(|i| !i.meta.title.contains("Steamodded")));
    }

    #[test]
    fn merge_never_merges_deprecated_packages() {
        let index = vec![index_item(
            "SuperDZ555@balatro_horse_mod",
            "balatro horse mod",
            "SuperDZ555",
            "",
        )];
        let items = merge(index, &fixture_packages());
        assert_eq!(
            by_id(&items, "SuperDZ555@balatro_horse_mod")
                .unwrap()
                .source,
            CatalogSource::Index
        );
        let ts = by_id(&items, "ts:SuperDZ555-balatro_horse_mod").expect("standalone");
        assert!(ts.thunderstore.as_ref().unwrap().deprecated);
    }

    #[test]
    fn colliding_thunderstore_titles_get_their_namespace() {
        let mut packages = fixture_packages();
        let mut other = thunderstore::find_package(&packages, "nh6574-JokerDisplay")
            .unwrap()
            .clone();
        other.full_name = "Another-JokerDisplay".into();
        other.owner = "Another".into();
        packages.push(other);

        let items = merge(Vec::new(), &packages);
        assert_eq!(
            by_id(&items, "ts:Another-JokerDisplay").unwrap().meta.title,
            "JokerDisplay"
        );
        assert_eq!(
            by_id(&items, "ts:nh6574-JokerDisplay").unwrap().meta.title,
            "JokerDisplay (nh6574)"
        );

        // An index entry always keeps the plain title
        let index = vec![index_item(
            "Someone@JokerDisplay",
            "Joker Display",
            "Someone",
            "",
        )];
        let items = merge(index, &packages);
        assert_eq!(
            by_id(&items, "Someone@JokerDisplay").unwrap().meta.title,
            "Joker Display"
        );
        for (id, title) in [
            ("ts:Another-JokerDisplay", "JokerDisplay (Another)"),
            ("ts:nh6574-JokerDisplay", "JokerDisplay (nh6574)"),
        ] {
            assert_eq!(by_id(&items, id).unwrap().meta.title, title);
        }
    }

    #[test]
    fn merge_field_priority() {
        let mut index = index_fixture();
        index[1].has_thumbnail = false;
        index[1].image_url.clear();
        index[1].description.clear();
        index[1].meta.categories = vec!["Joker".into()];
        let items = merge(index, &fixture_packages());
        let packages = fixture_packages();
        let ts = thunderstore::find_package(&packages, "Pokermon-Pokermon").unwrap();
        let merged = by_id(&items, "InertSteak@Pokermon").unwrap();

        assert_eq!(merged.meta.title, "Pokermon");
        assert_eq!(merged.image_url, thunderstore::icon_url(ts));
        assert!(merged.has_thumbnail);
        assert_eq!(
            merged.description,
            thunderstore::latest_active_version(ts).unwrap().description
        );
        assert_eq!(merged.meta.categories[0], "Joker");
        assert!(merged.meta.categories.len() > 1);
        assert_eq!(
            merged.meta.last_updated,
            thunderstore::parse_timestamp(&ts.date_updated).unwrap()
        );
        assert_eq!(merged.downloads, Some(thunderstore::total_downloads(ts)));

        // Index fields win when present
        let amulet = by_id(&items, "frostice482@Amulet").unwrap();
        assert_eq!(amulet.description, "Amulet from the index");
        assert!(amulet.image_url.contains("raw.githubusercontent.com"));
        assert_eq!(amulet.meta.title, "Amulet");
    }

    #[test]
    fn thunderstore_only_items_carry_install_data() {
        let items = merge(Vec::new(), &fixture_packages());
        let joker = by_id(&items, "ts:nh6574-JokerDisplay").unwrap();
        assert_eq!(joker.source, CatalogSource::Thunderstore);
        assert_eq!(joker.meta.title, "JokerDisplay");
        assert_eq!(joker.meta.author, "nh6574");
        assert_eq!(joker.meta.folder_name, "nh6574-JokerDisplay");
        assert_eq!(
            joker.meta.download_url.as_deref(),
            Some("ts://nh6574-JokerDisplay")
        );
        assert!(joker.has_thumbnail);
        assert!(joker.downloads.unwrap() > 0);
        assert!(joker.meta.last_updated > 0);

        let cryptid = by_id(&items, "ts:spectralpack-Cryptid").unwrap();
        assert!(cryptid.meta.requires_steamodded);
        assert_eq!(cryptid.meta.repo, "https://github.com/SpectralPack/Cryptid");

        let lust = by_id(&items, "ts:crimsonseraphim-Lust").unwrap();
        assert!(lust.thunderstore.as_ref().unwrap().nsfw);
        let ai = by_id(&items, "ts:vibez-RankAndFile").unwrap();
        assert!(ai.thunderstore.as_ref().unwrap().ai_generated);
        assert!(!ai.meta.categories.iter().any(|c| c == "AI Generated"));
    }

    #[test]
    fn multiplayer_packages_are_exposed_separately() {
        let mp = multiplayer_packages_in(&fixture_packages());
        let names: Vec<&str> = mp.iter().map(|p| p.full_name.as_str()).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"BalatroMultiplayer-MultiplayerAPI"));
        assert!(names.contains(&"BalatroMultiplayer-Multiplayer"));
        assert!(names.contains(&"BalatroMultiplayer-MultiplayerSpeedrun"));
    }

    #[test]
    fn multiplayer_github_downloads_are_blocked() {
        let mut index = index_fixture();
        index.push(index_item(
            "Virtualized@Multiplayer",
            "Multiplayer",
            "Virtualized",
            "https://github.com/Balatro-Multiplayer/BalatroMultiplayer",
        ));
        let items = merge(index, &fixture_packages());
        assert!(by_id(&items, "Virtualized@Multiplayer").is_none());

        for url in [
            "https://github.com/Balatro-Multiplayer/BalatroMultiplayer/releases/download/v0.5.5/BalatroMultiplayer.zip",
            "https://github.com/Balatro-Multiplayer/BalatroMultiplayer/releases/download/v0.5.5/BalatroMultiplayer-raw.zip",
            "https://codeload.github.com/Balatro-Multiplayer/BalatroMultiplayer/zip/refs/heads/main",
            "https://example.com/mirror/BalatroMultiplayer.zip?x=1",
        ] {
            assert!(is_blocked_download(url), "{url}");
        }
        for url in [
            "https://thunderstore.io/package/download/BalatroMultiplayer/MultiplayerAPI/0.1.2/",
            "https://github.com/InertSteak/Pokermon/archive/refs/heads/main.zip",
        ] {
            assert!(!is_blocked_download(url), "{url}");
        }
    }

    #[test]
    fn parse_ts_ref_forms() {
        assert_eq!(
            parse_ts_ref("ts://nh6574-JokerDisplay"),
            Some(("nh6574-JokerDisplay".into(), None))
        );
        assert_eq!(
            parse_ts_ref("ts://just_frostice482-Amulet@3.6.1"),
            Some(("just_frostice482-Amulet".into(), Some("3.6.1".into())))
        );
        assert_eq!(
            parse_ts_ref("ts:nh6574-JokerDisplay"),
            Some(("nh6574-JokerDisplay".into(), None))
        );
        assert_eq!(parse_ts_ref("ts://nh6574-JokerDisplay@../../x"), None);
        assert_eq!(parse_ts_ref("ts://a/b-c"), None);
        assert_eq!(parse_ts_ref("ts://"), None);
    }

    #[test]
    fn ts_urls_resolve_to_official_downloads() {
        let packages = fixture_packages();
        let joker = thunderstore::find_package(&packages, "nh6574-JokerDisplay").unwrap();
        let latest = thunderstore::latest_active_version(joker)
            .unwrap()
            .version_number
            .clone();
        assert_eq!(
            resolve_ts_in("ts://nh6574-JokerDisplay", &packages).unwrap(),
            Some(format!(
                "https://thunderstore.io/package/download/nh6574/JokerDisplay/{latest}/"
            ))
        );
        let older = joker.versions.last().unwrap().version_number.clone();
        assert_eq!(
            resolve_ts_in(&format!("ts://nh6574-JokerDisplay@{older}"), &packages).unwrap(),
            Some(format!(
                "https://thunderstore.io/package/download/nh6574/JokerDisplay/{older}/"
            ))
        );
        assert_eq!(
            resolve_ts_in("ts://BalatroMultiplayer-MultiplayerAPI", &packages)
                .unwrap()
                .map(|u| u.starts_with(
                    "https://thunderstore.io/package/download/BalatroMultiplayer/MultiplayerAPI/"
                )),
            Some(true)
        );
        assert!(resolve_ts_in("ts://nh6574-JokerDisplay@99.0.0", &packages).is_err());
        assert_eq!(resolve_ts_in("ts://nobody-Nothing", &packages), Ok(None));
        assert!(resolve_ts_in("ts://Thunderstore-lovely", &packages).is_err());
        assert!(resolve_ts_in("ts://Steamodded-Steamodded", &packages).is_err());
        assert!(resolve_ts_in("ts://not valid", &packages).is_err());
    }

    #[test]
    fn ts_targets_use_canonical_names_accepted_by_the_installer() {
        let packages = fixture_packages();
        let target = resolve_ts_target_in("ts://NH6574-jokerdisplay", &packages)
            .unwrap()
            .unwrap();
        assert_eq!(
            (target.namespace.as_str(), target.name.as_str()),
            ("nh6574", "JokerDisplay")
        );
        bmm_lib::installer::validate_thunderstore_download_url(
            &target.download_url,
            &target.namespace,
            &target.name,
        )
        .unwrap();
        for pkg in &packages {
            if let Ok(Some(t)) = resolve_ts_target_in(&format!("ts://{}", pkg.full_name), &packages)
            {
                assert!(
                    bmm_lib::installer::validate_thunderstore_download_url(
                        &t.download_url,
                        &t.namespace,
                        &t.name
                    )
                    .is_ok(),
                    "{} rejected by the installer",
                    pkg.full_name
                );
            }
        }
    }

    #[test]
    fn index_and_bmi_ids_resolve_against_merged_items() {
        let items = merge(index_fixture(), &fixture_packages());
        for url in [
            "index://InertSteak@Pokermon",
            "bmi://InertSteak@Pokermon",
            "bmi://inertsteak-pokermon",
        ] {
            let id = mod_index::strip_index_scheme(url).unwrap();
            let item = mod_index::resolve_index_id(id, &items).expect(url);
            assert_eq!(item.dir_name, "InertSteak@Pokermon");
            assert_eq!(
                item.meta.download_url.as_deref(),
                Some("https://github.com/InertSteak/Pokermon/archive/refs/heads/main.zip")
            );
        }
    }

    #[test]
    fn find_in_prefers_ids_then_thunderstore_names_then_index_titles() {
        let items = merge(index_fixture(), &fixture_packages());
        let find = |k: &str| find_in(&items, k).map(|i| i.dir_name.as_str());
        assert_eq!(find("InertSteak@Pokermon"), Some("InertSteak@Pokermon"));
        assert_eq!(
            find("ts:PokermonPlus-Pokermon"),
            Some("ts:PokermonPlus-Pokermon")
        );
        assert_eq!(find("Pokermon-Pokermon"), Some("InertSteak@Pokermon"));
        assert_eq!(
            find("ts://nh6574-JokerDisplay@1.0.0"),
            Some("ts:nh6574-JokerDisplay")
        );
        assert_eq!(find("Steamopollys-Steamodded"), Some(STEAMODDED_ID));
        assert_eq!(find("Pokermon"), Some("InertSteak@Pokermon"));
        assert_eq!(find("jokerdisplay"), Some("ts:nh6574-JokerDisplay"));
        assert_eq!(find("missing mod"), None);
        assert_eq!(find(""), None);
    }
}
