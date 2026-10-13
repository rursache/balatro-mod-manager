//! Install plans for catalog entries
//!
//! A Thunderstore install pulls in every package dependency (latest active version at
//! or above the requested minimum) in dependency order, while Lovely and Steamodded
//! dependencies become version checks handled by the app's own installers

use std::collections::{HashMap, HashSet};
use std::path::Path;

use bmm_lib::database::{InstalledMod, SOURCE_INDEX, SOURCE_THUNDERSTORE};
use serde::{Deserialize, Serialize};

use crate::catalog;
use crate::models::{ArchiveModItem, CatalogSource};
use crate::thunderstore::{self, TsPackage, TsVersion};

pub const LEGACY_STEAMODDED_WARNING: &str = "old Steamodded API, likely broken";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallSource {
    Index,
    Thunderstore,
}

impl InstallSource {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "index" => Ok(Self::Index),
            "thunderstore" => Ok(Self::Thunderstore),
            other => Err(format!("Unknown install source '{other}'")),
        }
    }

    pub fn db_source(self) -> &'static str {
        match self {
            Self::Index => SOURCE_INDEX,
            Self::Thunderstore => SOURCE_THUNDERSTORE,
        }
    }

    fn of_row(row: &InstalledMod) -> Self {
        if row.source == SOURCE_THUNDERSTORE {
            Self::Thunderstore
        } else {
            Self::Index
        }
    }
}

/// One mod to download and record, steps run in order so dependencies come first
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanStep {
    pub source: InstallSource,
    pub catalog_id: String,
    pub title: String,
    /// `Namespace-Name` of Thunderstore steps
    pub full_name: Option<String>,
    /// Empty for a Steamodded step without a pinned version, the latest is installed then
    pub version: String,
    /// Highest minimum asked for by the dependents, None for the requested mod
    pub min_version: Option<String>,
    /// Official Thunderstore download or the index install URL
    pub download_url: String,
    pub folder_name: String,
    /// Thunderstore packages this step depends on, `Namespace-Name`
    pub requires: Vec<String>,
    pub requires_steamodded: bool,
    pub requires_talisman: bool,
    pub deprecated: bool,
    /// Installed row this step updates or moves to another source
    pub tracked_name: Option<String>,
    pub tracked_source: Option<String>,
    pub installed_version: Option<String>,
    pub is_root: bool,
}

/// A package dependency that is already installed and recent enough
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SatisfiedDependency {
    pub full_name: String,
    pub title: String,
    pub tracked_name: String,
    pub installed_version: Option<String>,
    pub min_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LovelyRequirement {
    pub min_version: String,
    pub installed_version: Option<String>,
    pub satisfied: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SteamoddedRequirement {
    /// Thunderstore numbering such as `1.1814.0`, None when any version works
    pub min_version: Option<String>,
    /// GitHub release tag of the minimum, `1.0.0-beta-1814a`
    pub min_tag: Option<String>,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub satisfied: bool,
    /// Some package depends on the deprecated Steamopollys upload
    pub legacy_api: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TalismanRequirement {
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallPlan {
    pub mod_id: String,
    pub mod_title: String,
    pub source: InstallSource,
    pub version: String,
    pub steps: Vec<PlanStep>,
    pub satisfied: Vec<SatisfiedDependency>,
    pub lovely: Option<LovelyRequirement>,
    pub steamodded: Option<SteamoddedRequirement>,
    pub talisman: Option<TalismanRequirement>,
    /// Titles of everything the mod needs that is missing, as the DepPrompt lists them
    pub missing: Vec<String>,
    pub warnings: Vec<String>,
}

/// What the planner knows about the catalog and the local install
pub struct PlanContext<'a> {
    pub catalog: &'a [ArchiveModItem],
    pub packages: &'a [TsPackage],
    pub installed: &'a [InstalledMod],
    pub lovely_version: Option<String>,
    pub steamodded_installed: bool,
    pub steamodded_version: Option<String>,
    pub talisman_installed: bool,
}

/// The version inside a Steamodded `version.lua`, `return "1.0.0~BETA-1016c"`
pub fn parse_version_lua(content: &str) -> Option<String> {
    let start = content.find(['"', '\''])?;
    let quote = content[start..].chars().next()?;
    let rest = &content[start + 1..];
    let end = rest.find(quote)?;
    let version = rest[..end].trim();
    (!version.is_empty()).then(|| version.to_string())
}

pub fn read_steamodded_version(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("version.lua"))
        .ok()
        .and_then(|c| parse_version_lua(&c))
}

/// Whether Steamodded is installed and its version. The recorded release tag and
/// `version.lua` can disagree (some releases do not bump the file), the higher one wins
pub fn installed_steamodded(
    installed: &[InstalledMod],
    mods_dir: Option<&Path>,
) -> (bool, Option<String>) {
    use bmm_lib::smods_installer::parse_smods_version;
    if let Some(row) = installed
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case("steamodded"))
        && Path::new(&row.path).is_dir()
    {
        let from_file = read_steamodded_version(Path::new(&row.path));
        let recorded = row
            .current_version
            .clone()
            .filter(|v| parse_smods_version(v).is_some());
        let version = match (from_file, recorded) {
            (Some(file), Some(rec)) => {
                if parse_smods_version(&rec) > parse_smods_version(&file) {
                    Some(rec)
                } else {
                    Some(file)
                }
            }
            (file, rec) => file.or(rec),
        };
        return (true, version);
    }
    match mods_dir.and_then(|d| {
        bmm_lib::smods_installer::find_steamodded_dirs(d)
            .into_iter()
            .next()
    }) {
        Some(dir) => (true, read_steamodded_version(&dir)),
        None => (false, None),
    }
}

fn max_version(current: Option<String>, candidate: &str) -> Option<String> {
    match current {
        Some(c) if thunderstore::compare_versions(&c, candidate).is_ge() => Some(c),
        _ => Some(candidate.to_string()),
    }
}

/// Numeric comparison of an index version with a Thunderstore minimum, None when either
/// side is not a plain `x.y.z`
fn index_version_at_least(installed: &str, min: &str) -> Option<bool> {
    let installed = installed.trim();
    let installed = installed.strip_prefix(['v', 'V']).unwrap_or(installed);
    thunderstore::parse_version(installed)?;
    thunderstore::parse_version(min)?;
    Some(thunderstore::version_at_least(installed, min))
}

fn catalog_entry_of<'a>(ctx: &PlanContext<'a>, full_name: &str) -> Option<&'a ArchiveModItem> {
    ctx.catalog.iter().find(|i| {
        i.thunderstore
            .as_ref()
            .is_some_and(|t| t.full_name.eq_ignore_ascii_case(full_name))
    })
}

/// The installed row of a catalog entry: the Thunderstore package by source_id, else the
/// index row with the entry title. A Thunderstore only entry claims an index row only
/// when no index entry lists that title, the frontend shows such a row as that package
/// (a mod the index dropped) so installing it moves the row instead of adding a copy
fn tracked_row<'a>(ctx: &PlanContext<'a>, item: &ArchiveModItem) -> Option<&'a InstalledMod> {
    if let Some(ts) = &item.thunderstore
        && let Some(row) = ctx.installed.iter().find(|r| {
            r.source_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(&ts.full_name))
        })
    {
        return Some(row);
    }
    if item.source != CatalogSource::Thunderstore {
        return ctx
            .installed
            .iter()
            .find(|r| r.source == SOURCE_INDEX && r.name.eq_ignore_ascii_case(&item.meta.title));
    }
    // The Multiplayer section adopts manual copies of its packages itself
    if item.thunderstore.as_ref().is_none_or(|t| {
        t.namespace
            .eq_ignore_ascii_case(thunderstore::MULTIPLAYER_NAMESPACE)
    }) {
        return None;
    }
    let title = compact_title(&item.meta.title);
    if title.is_empty()
        || ctx.catalog.iter().any(|i| {
            i.source != CatalogSource::Thunderstore && compact_title(&i.meta.title) == title
        })
    {
        return None;
    }
    let index_rows = || ctx.installed.iter().filter(|r| r.source == SOURCE_INDEX);
    index_rows()
        .find(|r| r.name.eq_ignore_ascii_case(&item.meta.title))
        .or_else(|| index_rows().find(|r| compact_title(&r.name) == title))
}

/// Title key the frontend matches installed rows with, lowercase ASCII letters and digits
fn compact_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn tracked_package<'a>(ctx: &PlanContext<'a>, full_name: &str) -> Option<&'a InstalledMod> {
    if let Some(row) = ctx.installed.iter().find(|r| {
        r.source_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(full_name))
    }) {
        return Some(row);
    }
    catalog_entry_of(ctx, full_name).and_then(|item| tracked_row(ctx, item))
}

/// Tracked mods keep their source, new installs prefer Thunderstore unless the package is
/// deprecated, an explicit source always wins
pub fn choose_source(
    item: &ArchiveModItem,
    tracked: Option<&InstalledMod>,
    explicit: Option<InstallSource>,
) -> Result<InstallSource, String> {
    let has_thunderstore = item.thunderstore.is_some();
    let has_index = item.source != CatalogSource::Thunderstore;
    let available = |s: InstallSource| match s {
        InstallSource::Index => has_index,
        InstallSource::Thunderstore => has_thunderstore,
    };
    if let Some(source) = explicit {
        return if available(source) {
            Ok(source)
        } else {
            Err(format!(
                "{} is not available from {}",
                item.meta.title,
                match source {
                    InstallSource::Index => "the mod index",
                    InstallSource::Thunderstore => "Thunderstore",
                }
            ))
        };
    }
    if let Some(row) = tracked
        && available(InstallSource::of_row(row))
    {
        return Ok(InstallSource::of_row(row));
    }
    let deprecated = item.thunderstore.as_ref().is_some_and(|t| t.deprecated);
    if has_thunderstore && (!deprecated || !has_index) {
        Ok(InstallSource::Thunderstore)
    } else {
        Ok(InstallSource::Index)
    }
}

/// Catalog entry for `id`, Thunderstore packages kept out of the catalog (Multiplayer)
/// resolve from the listing
fn resolve_item(ctx: &PlanContext<'_>, id: &str) -> Result<ArchiveModItem, String> {
    if let Some(item) = catalog::find_in(ctx.catalog, id) {
        return Ok(item.clone());
    }
    if let Some((full_name, _)) = catalog::parse_ts_ref(id) {
        if thunderstore::is_lovely(&full_name) {
            return Err(
                "Lovely is managed by the app and is not installed from Thunderstore".into(),
            );
        }
        if let Some(item) =
            thunderstore::find_package(ctx.packages, &full_name).and_then(catalog::package_item)
        {
            return Ok(item);
        }
    }
    Err(format!(
        "{id} is not in the catalog, refresh it and try again"
    ))
}

pub fn plan_install(
    ctx: &PlanContext<'_>,
    id: &str,
    version: Option<&str>,
    explicit: Option<InstallSource>,
) -> Result<InstallPlan, String> {
    let version = version.map(str::trim).filter(|v| !v.is_empty());
    let item = resolve_item(ctx, id)?;
    if item.dir_name == catalog::STEAMODDED_ID {
        return Ok(plan_steamodded(ctx, &item, version));
    }
    let tracked = tracked_row(ctx, &item);
    match choose_source(&item, tracked, explicit)? {
        InstallSource::Index => Ok(plan_index(ctx, &item, tracked)),
        InstallSource::Thunderstore => plan_thunderstore(ctx, &item, tracked, version),
    }
}

fn steamodded_row<'a>(ctx: &PlanContext<'a>) -> Option<&'a InstalledMod> {
    ctx.installed
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case("steamodded"))
}

fn plan_steamodded(
    ctx: &PlanContext<'_>,
    item: &ArchiveModItem,
    version: Option<&str>,
) -> InstallPlan {
    // A Thunderstore number such as 1.1814.0 becomes its release tag
    let tag = version
        .map(|v| match thunderstore::parse_version(v) {
            Some(_) => thunderstore::steamodded_version_to_tag(v).unwrap_or_else(|| v.to_string()),
            None => v.to_string(),
        })
        .unwrap_or_default();
    let row = steamodded_row(ctx);
    InstallPlan {
        mod_id: item.dir_name.clone(),
        mod_title: item.meta.title.clone(),
        source: InstallSource::Index,
        version: tag.clone(),
        steps: vec![PlanStep {
            source: InstallSource::Index,
            catalog_id: item.dir_name.clone(),
            title: item.meta.title.clone(),
            full_name: None,
            version: tag,
            min_version: None,
            download_url: String::new(),
            folder_name: String::new(),
            requires: Vec::new(),
            requires_steamodded: false,
            requires_talisman: false,
            deprecated: false,
            tracked_name: row.map(|r| r.name.clone()),
            tracked_source: row.map(|r| r.source.clone()),
            installed_version: ctx.steamodded_version.clone(),
            is_root: true,
        }],
        satisfied: Vec::new(),
        lovely: None,
        steamodded: None,
        talisman: None,
        missing: Vec::new(),
        warnings: Vec::new(),
    }
}

fn steamodded_requirement(
    ctx: &PlanContext<'_>,
    min: Option<String>,
    legacy_api: bool,
) -> SteamoddedRequirement {
    let satisfied = ctx.steamodded_installed
        && min.as_deref().is_none_or(|min| {
            ctx.steamodded_version
                .as_deref()
                .is_some_and(|v| bmm_lib::smods_installer::satisfies_min(v, min))
        });
    SteamoddedRequirement {
        min_tag: min
            .as_deref()
            .and_then(thunderstore::steamodded_version_to_tag),
        min_version: min,
        installed: ctx.steamodded_installed,
        installed_version: ctx.steamodded_version.clone(),
        satisfied,
        legacy_api,
    }
}

fn finish_missing(plan: &mut InstallPlan) {
    let mut missing: Vec<String> = plan
        .steps
        .iter()
        .filter(|s| !s.is_root)
        .map(|s| s.title.clone())
        .collect();
    if plan.steamodded.as_ref().is_some_and(|s| !s.satisfied) {
        missing.push("Steamodded".to_string());
    }
    if plan.talisman.as_ref().is_some_and(|t| !t.installed) {
        missing.push("Talisman".to_string());
    }
    if plan.lovely.as_ref().is_some_and(|l| !l.satisfied) {
        missing.push("Lovely".to_string());
    }
    plan.missing = missing;
}

fn plan_index(
    ctx: &PlanContext<'_>,
    item: &ArchiveModItem,
    tracked: Option<&InstalledMod>,
) -> InstallPlan {
    let meta = &item.meta;
    let download_url = meta
        .download_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| format!("index://{}", item.dir_name));
    let folder_name = if meta.folder_name.trim().is_empty() {
        meta.title.split_whitespace().collect::<String>()
    } else {
        meta.folder_name.clone()
    };
    let mut plan = InstallPlan {
        mod_id: item.dir_name.clone(),
        mod_title: meta.title.clone(),
        source: InstallSource::Index,
        version: meta.version.clone(),
        steps: vec![PlanStep {
            source: InstallSource::Index,
            catalog_id: item.dir_name.clone(),
            title: meta.title.clone(),
            full_name: None,
            version: meta.version.clone(),
            min_version: None,
            download_url,
            folder_name,
            requires: Vec::new(),
            requires_steamodded: meta.requires_steamodded,
            requires_talisman: meta.requires_talisman,
            deprecated: false,
            tracked_name: tracked.map(|r| r.name.clone()),
            tracked_source: tracked.map(|r| r.source.clone()),
            installed_version: tracked.and_then(|r| r.current_version.clone()),
            is_root: true,
        }],
        satisfied: Vec::new(),
        lovely: None,
        steamodded: meta
            .requires_steamodded
            .then(|| steamodded_requirement(ctx, None, false)),
        talisman: meta.requires_talisman.then_some(TalismanRequirement {
            installed: ctx.talisman_installed,
        }),
        missing: Vec::new(),
        warnings: Vec::new(),
    };
    finish_missing(&mut plan);
    plan
}

struct Node<'a> {
    pkg: &'a TsPackage,
    version: &'a TsVersion,
    title: String,
    catalog_id: String,
    min: Option<String>,
    tracked: Option<&'a InstalledMod>,
    install: bool,
    expanded: bool,
    edges: Vec<String>,
}

/// Whether a tracked dependency has to be installed or updated to meet `min`. Rows from
/// the index keep their source, a lower index version only warns
fn dependency_needs_install(
    tracked: Option<&InstalledMod>,
    min: &str,
    title: &str,
    warnings: &mut Vec<String>,
) -> bool {
    let Some(row) = tracked else {
        return true;
    };
    let installed = row.current_version.as_deref().unwrap_or("").trim();
    if row.source == SOURCE_THUNDERSTORE {
        return installed.is_empty() || !thunderstore::version_at_least(installed, min);
    }
    if index_version_at_least(installed, min) == Some(false) {
        let warning = format!(
            "{title} {installed} from the mod index is older than the required {min}, update it"
        );
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    false
}

fn node_key(full_name: &str) -> String {
    full_name.to_ascii_lowercase()
}

fn plan_thunderstore(
    ctx: &PlanContext<'_>,
    item: &ArchiveModItem,
    tracked: Option<&InstalledMod>,
    version: Option<&str>,
) -> Result<InstallPlan, String> {
    let info = item
        .thunderstore
        .as_ref()
        .ok_or_else(|| format!("{} is not on Thunderstore", item.meta.title))?;
    let root_pkg = thunderstore::find_package(ctx.packages, &info.full_name).ok_or_else(|| {
        format!(
            "{} is no longer on Thunderstore, refresh the catalog and try again",
            info.full_name
        )
    })?;
    let root_version = match version {
        Some(v) => thunderstore::find_version(root_pkg, v)
            .ok_or_else(|| format!("{} has no active version {v}", root_pkg.full_name))?,
        None => thunderstore::latest_active_version(root_pkg)
            .ok_or_else(|| format!("{} has no active version", root_pkg.full_name))?,
    };

    let mut warnings: Vec<String> = Vec::new();
    let mut lovely_min: Option<String> = None;
    let mut smods_min: Option<String> = None;
    let mut smods_legacy = false;
    let mut smods_required = false;

    let mut nodes: HashMap<String, Node> = HashMap::new();
    let root_key = node_key(&root_pkg.full_name);
    nodes.insert(
        root_key.clone(),
        Node {
            pkg: root_pkg,
            version: root_version,
            title: item.meta.title.clone(),
            catalog_id: item.dir_name.clone(),
            min: None,
            tracked,
            install: true,
            expanded: false,
            edges: Vec::new(),
        },
    );

    let mut queue = vec![root_key.clone()];
    while let Some(key) = queue.pop() {
        let (deps, title) = {
            let node = nodes.get_mut(&key).expect("queued node");
            if node.expanded || !node.install {
                continue;
            }
            node.expanded = true;
            (node.version.dependencies.clone(), node.title.clone())
        };
        let mut edges = Vec::new();
        for raw in &deps {
            let Some(dep) = thunderstore::parse_dependency(raw) else {
                warnings.push(format!(
                    "Ignoring unrecognized dependency '{raw}' of {title}"
                ));
                continue;
            };
            let dep_full = dep.full_name();
            if thunderstore::is_lovely(&dep_full) {
                lovely_min = max_version(lovely_min, &dep.version);
                continue;
            }
            if thunderstore::is_steamodded(&dep_full) {
                smods_required = true;
                if dep_full.eq_ignore_ascii_case(thunderstore::LEGACY_STEAMODDED_PACKAGE) {
                    smods_legacy = true;
                    warnings.push(format!(
                        "{title} depends on {}, the {LEGACY_STEAMODDED_WARNING}",
                        thunderstore::LEGACY_STEAMODDED_PACKAGE
                    ));
                } else {
                    smods_min = max_version(smods_min, &dep.version);
                }
                continue;
            }
            let Some(pkg) = thunderstore::find_package(ctx.packages, &dep_full) else {
                warnings.push(format!(
                    "{title} needs {dep_full} which is not on Thunderstore"
                ));
                continue;
            };
            let dep_key = node_key(&pkg.full_name);
            edges.push(dep_key.clone());
            if let Some(existing) = nodes.get_mut(&dep_key) {
                existing.min = max_version(existing.min.take(), &dep.version);
                if !existing.install {
                    let min = existing.min.clone().unwrap_or_default();
                    if dependency_needs_install(
                        existing.tracked,
                        &min,
                        &existing.title,
                        &mut warnings,
                    ) {
                        existing.install = true;
                        queue.push(dep_key);
                    }
                }
                continue;
            }
            let Some(latest) = thunderstore::latest_active_version(pkg) else {
                warnings.push(format!(
                    "{title} needs {} which has no active version",
                    pkg.full_name
                ));
                edges.pop();
                continue;
            };
            if !thunderstore::version_at_least(&latest.version_number, &dep.version) {
                warnings.push(format!(
                    "{title} asks for {} {} but the newest version is {}",
                    pkg.full_name, dep.version, latest.version_number
                ));
            }
            let catalog_item = catalog_entry_of(ctx, &pkg.full_name);
            let dep_title = catalog_item
                .map(|i| i.meta.title.clone())
                .unwrap_or_else(|| thunderstore::display_title(&pkg.name));
            let dep_tracked = tracked_package(ctx, &pkg.full_name);
            let install =
                dependency_needs_install(dep_tracked, &dep.version, &dep_title, &mut warnings);
            nodes.insert(
                dep_key.clone(),
                Node {
                    pkg,
                    version: latest,
                    title: dep_title,
                    catalog_id: catalog_item
                        .map(|i| i.dir_name.clone())
                        .unwrap_or_else(|| catalog::ts_catalog_id(&pkg.full_name)),
                    min: Some(dep.version.clone()),
                    tracked: dep_tracked,
                    install,
                    expanded: false,
                    edges: Vec::new(),
                },
            );
            if install {
                queue.push(dep_key);
            }
        }
        let mut seen = HashSet::new();
        edges.retain(|k| seen.insert(k.clone()));
        nodes.get_mut(&key).expect("expanded node").edges = edges;
    }

    // Steamodded is never installed as a package, but its own Lovely minimum still applies
    if smods_required
        && let Some(smods) =
            thunderstore::find_package(ctx.packages, thunderstore::STEAMODDED_PACKAGE)
        && let Some(latest) = thunderstore::latest_active_version(smods)
    {
        for dep in latest
            .dependencies
            .iter()
            .filter_map(|d| thunderstore::parse_dependency(d))
        {
            if thunderstore::is_lovely(&dep.full_name()) {
                lovely_min = max_version(lovely_min, &dep.version);
            }
        }
    }

    let order = topological_order(&nodes, &root_key, &mut warnings);

    let mut steps = Vec::new();
    for key in &order {
        let node = &nodes[key];
        let (namespace, name) = thunderstore::split_full_name(&node.pkg.full_name)
            .ok_or_else(|| format!("invalid package name '{}'", node.pkg.full_name))?;
        if node.pkg.is_deprecated {
            warnings.push(format!("{} is deprecated on Thunderstore", node.title));
        }
        let requires_steamodded = node.version.dependencies.iter().any(|d| {
            thunderstore::parse_dependency(d)
                .is_some_and(|d| thunderstore::is_steamodded(&d.full_name()))
        });
        steps.push(PlanStep {
            source: InstallSource::Thunderstore,
            catalog_id: node.catalog_id.clone(),
            title: node.title.clone(),
            full_name: Some(node.pkg.full_name.clone()),
            version: node.version.version_number.clone(),
            min_version: node.min.clone(),
            download_url: thunderstore::download_url(namespace, name, &node.version.version_number),
            folder_name: node.pkg.full_name.clone(),
            requires: node
                .edges
                .iter()
                .map(|k| nodes[k].pkg.full_name.clone())
                .collect(),
            requires_steamodded,
            requires_talisman: node
                .edges
                .iter()
                .any(|k| nodes[k].pkg.name.eq_ignore_ascii_case("Talisman")),
            deprecated: node.pkg.is_deprecated,
            tracked_name: node.tracked.map(|r| r.name.clone()),
            tracked_source: node.tracked.map(|r| r.source.clone()),
            installed_version: node.tracked.and_then(|r| r.current_version.clone()),
            is_root: *key == root_key,
        });
    }

    let mut satisfied: Vec<SatisfiedDependency> = nodes
        .values()
        .filter(|n| !n.install)
        .filter_map(|n| {
            Some(SatisfiedDependency {
                full_name: n.pkg.full_name.clone(),
                title: n.title.clone(),
                tracked_name: n.tracked?.name.clone(),
                installed_version: n.tracked?.current_version.clone(),
                min_version: n.min.clone().unwrap_or_default(),
            })
        })
        .collect();
    satisfied.sort_by(|a, b| a.full_name.cmp(&b.full_name));

    let lovely = lovely_min.map(|min| LovelyRequirement {
        satisfied: bmm_lib::lovely::installed_version_satisfies(
            ctx.lovely_version.as_deref(),
            &min,
        ),
        installed_version: ctx.lovely_version.clone(),
        min_version: min,
    });
    let steamodded = smods_required.then(|| steamodded_requirement(ctx, smods_min, smods_legacy));

    let mut plan = InstallPlan {
        mod_id: item.dir_name.clone(),
        mod_title: item.meta.title.clone(),
        source: InstallSource::Thunderstore,
        version: root_version.version_number.clone(),
        steps,
        satisfied,
        lovely,
        steamodded,
        talisman: None,
        missing: Vec::new(),
        warnings,
    };
    finish_missing(&mut plan);
    Ok(plan)
}

/// Dependencies first, a cycle is reported and its closing edge ignored
fn topological_order(
    nodes: &HashMap<String, Node>,
    root: &str,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    fn visit(
        key: &str,
        nodes: &HashMap<String, Node>,
        done: &mut HashSet<String>,
        stack: &mut Vec<String>,
        order: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) {
        if done.contains(key) {
            return;
        }
        stack.push(key.to_string());
        for dep in &nodes[key].edges {
            if !nodes[dep].install {
                continue;
            }
            if stack.iter().any(|k| k == dep) {
                warnings.push(format!(
                    "Dependency cycle between {} and {}, ignoring it",
                    nodes[key].pkg.full_name, nodes[dep].pkg.full_name
                ));
                continue;
            }
            visit(dep, nodes, done, stack, order, warnings);
        }
        stack.pop();
        done.insert(key.to_string());
        order.push(key.to_string());
    }

    let mut order = Vec::new();
    visit(
        root,
        nodes,
        &mut HashSet::new(),
        &mut Vec::new(),
        &mut order,
        warnings,
    );
    order
}

/// Whether a Thunderstore row has a newer active version, None for rows of other sources
pub fn thunderstore_update_available(
    row: &InstalledMod,
    latest: &HashMap<String, String>,
) -> Option<bool> {
    if row.source != SOURCE_THUNDERSTORE {
        return None;
    }
    let installed = row.current_version.as_deref().unwrap_or("").trim();
    let Some(id) = row.source_id.as_deref() else {
        return Some(false);
    };
    if installed.is_empty() {
        return Some(false);
    }
    Some(latest.get(&id.to_ascii_lowercase()).is_some_and(|remote| {
        thunderstore::compare_versions(remote, installed) == std::cmp::Ordering::Greater
    }))
}

/// Names of installed Thunderstore rows whose package is deprecated
pub fn deprecated_installed(installed: &[InstalledMod], packages: &[TsPackage]) -> Vec<String> {
    let deprecated: HashSet<String> = packages
        .iter()
        .filter(|p| p.is_deprecated)
        .map(|p| p.full_name.to_ascii_lowercase())
        .collect();
    installed
        .iter()
        .filter(|r| r.source == SOURCE_THUNDERSTORE)
        .filter(|r| {
            r.source_id
                .as_deref()
                .is_some_and(|id| deprecated.contains(&id.to_ascii_lowercase()))
        })
        .map(|r| r.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModMeta;
    use crate::thunderstore::tests::fixture_packages;

    fn index_item(dir: &str, title: &str, repo: &str) -> ArchiveModItem {
        ArchiveModItem {
            dir_name: dir.into(),
            meta: ModMeta {
                title: title.into(),
                author: dir.split('@').next().unwrap_or_default().into(),
                repo: repo.into(),
                download_url: Some(format!("{repo}/archive/refs/heads/main.zip")),
                version: "1.0.0".into(),
                ..Default::default()
            },
            description: format!("{title} from the index"),
            ..Default::default()
        }
    }

    fn catalog_items(packages: &[TsPackage]) -> Vec<ArchiveModItem> {
        let mut talisman = index_item(
            "MathIsFun0@Talisman",
            "Talisman",
            "https://github.com/MathIsFun0/Talisman",
        );
        talisman.meta.requires_steamodded = true;
        let mut needs_both = index_item(
            "Someone@BigNumbers",
            "Big Numbers",
            "https://github.com/someone/bignumbers",
        );
        needs_both.meta.requires_steamodded = true;
        needs_both.meta.requires_talisman = true;
        catalog::merge(
            vec![
                index_item(
                    "Steamodded@smods",
                    "Steamodded",
                    "https://github.com/Steamodded/smods",
                ),
                index_item(
                    "InertSteak@Pokermon",
                    "Pokermon",
                    "https://github.com/InertSteak/Pokermon",
                ),
                index_item(
                    "frostice482@Amulet",
                    "Amulet",
                    "https://github.com/frostice482/amulet",
                ),
                talisman,
                needs_both,
            ],
            packages,
        )
    }

    fn row(name: &str, source: &str, source_id: Option<&str>, version: &str) -> InstalledMod {
        InstalledMod {
            name: name.into(),
            path: format!("/mods/{name}"),
            dependencies: Vec::new(),
            current_version: Some(version.into()),
            orphaned: false,
            source: source.into(),
            source_id: source_id.map(str::to_string),
            installed_at: None,
        }
    }

    fn ctx<'a>(
        catalog: &'a [ArchiveModItem],
        packages: &'a [TsPackage],
        installed: &'a [InstalledMod],
    ) -> PlanContext<'a> {
        PlanContext {
            catalog,
            packages,
            installed,
            lovely_version: None,
            steamodded_installed: false,
            steamodded_version: None,
            talisman_installed: false,
        }
    }

    fn step_names(plan: &InstallPlan) -> Vec<&str> {
        plan.steps
            .iter()
            .map(|s| s.full_name.as_deref().unwrap_or(&s.catalog_id))
            .collect()
    }

    fn package(full_name: &str, version: &str, deps: &[&str]) -> TsPackage {
        let (owner, name) = full_name.split_once('-').unwrap();
        TsPackage {
            name: name.into(),
            full_name: full_name.into(),
            owner: owner.into(),
            versions: vec![TsVersion {
                name: name.into(),
                full_name: format!("{full_name}-{version}"),
                version_number: version.into(),
                dependencies: deps.iter().map(|d| d.to_string()).collect(),
                is_active: true,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn speedrun_pulls_the_api_steamodded_and_lovely() {
        let packages = fixture_packages();
        let catalog = catalog_items(&packages);
        let plan = plan_install(
            &ctx(&catalog, &packages, &[]),
            "ts:BalatroMultiplayer-MultiplayerSpeedrun",
            None,
            None,
        )
        .expect("plan");

        assert_eq!(plan.source, InstallSource::Thunderstore);
        assert_eq!(plan.mod_id, "ts:BalatroMultiplayer-MultiplayerSpeedrun");
        assert_eq!(
            step_names(&plan),
            [
                "BalatroMultiplayer-MultiplayerAPI",
                "BalatroMultiplayer-MultiplayerSpeedrun"
            ]
        );
        let api = &plan.steps[0];
        assert_eq!(api.version, "0.1.2");
        assert_eq!(api.min_version.as_deref(), Some("0.1.1"));
        assert!(api.requires_steamodded);
        assert!(!api.is_root);
        let root = &plan.steps[1];
        assert!(root.is_root);
        assert_eq!(root.requires, ["BalatroMultiplayer-MultiplayerAPI"]);
        for step in &plan.steps {
            let (ns, name) =
                thunderstore::split_full_name(step.full_name.as_deref().unwrap()).unwrap();
            bmm_lib::installer::validate_thunderstore_download_url(&step.download_url, ns, name)
                .expect("official download URL");
            assert_eq!(step.folder_name, format!("{ns}-{name}"));
        }

        let smods = plan.steamodded.as_ref().expect("steamodded check");
        assert_eq!(smods.min_version.as_deref(), Some("1.1814.0"));
        assert_eq!(smods.min_tag.as_deref(), Some("1.0.0-beta-1814a"));
        assert!(!smods.satisfied && !smods.legacy_api);
        // Steamodded itself needs Lovely 0.9.0
        let lovely = plan.lovely.as_ref().expect("lovely check");
        assert_eq!(lovely.min_version, "0.9.0");
        assert!(!lovely.satisfied);
        assert_eq!(plan.missing, ["MultiplayerAPI", "Steamodded", "Lovely"]);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    #[test]
    fn installed_steamodded_lovely_and_api_are_skipped() {
        let packages = fixture_packages();
        let catalog = catalog_items(&packages);
        let installed = [row(
            "MultiplayerAPI",
            SOURCE_THUNDERSTORE,
            Some("BalatroMultiplayer-MultiplayerAPI"),
            "0.1.1",
        )];
        let mut context = ctx(&catalog, &packages, &installed);
        context.steamodded_installed = true;
        context.steamodded_version = Some("26.829.0".into());
        context.lovely_version = Some("0.10.0".into());

        let plan = plan_install(
            &context,
            "BalatroMultiplayer-MultiplayerSpeedrun",
            Some("0.1.1"),
            None,
        )
        .expect("plan");
        assert_eq!(
            step_names(&plan),
            ["BalatroMultiplayer-MultiplayerSpeedrun"]
        );
        assert_eq!(plan.satisfied.len(), 1);
        assert_eq!(plan.satisfied[0].tracked_name, "MultiplayerAPI");
        assert_eq!(plan.satisfied[0].min_version, "0.1.1");
        assert!(plan.steamodded.as_ref().unwrap().satisfied);
        assert!(plan.lovely.as_ref().unwrap().satisfied);
        assert!(plan.missing.is_empty());

        // An older tracked dependency is updated in place
        let installed = [row(
            "MultiplayerAPI",
            SOURCE_THUNDERSTORE,
            Some("BalatroMultiplayer-MultiplayerAPI"),
            "0.1.0",
        )];
        let mut context = ctx(&catalog, &packages, &installed);
        context.steamodded_installed = true;
        context.steamodded_version = Some("1.0.0~BETA-1016c".into());
        let plan = plan_install(
            &context,
            "ts://BalatroMultiplayer-MultiplayerSpeedrun",
            None,
            None,
        )
        .expect("plan");
        let api = &plan.steps[0];
        assert_eq!(api.tracked_name.as_deref(), Some("MultiplayerAPI"));
        assert_eq!(api.installed_version.as_deref(), Some("0.1.0"));
        assert_eq!(api.version, "0.1.2");
        // 1016c is older than the 1814a minimum
        assert!(!plan.steamodded.as_ref().unwrap().satisfied);
    }

    #[test]
    fn namespaces_with_underscores_resolve_and_index_rows_keep_their_source() {
        let packages = fixture_packages();
        let catalog = catalog_items(&packages);
        let plan = plan_install(
            &ctx(&catalog, &packages, &[]),
            "ts:spectralpack-Cryptid",
            None,
            None,
        )
        .expect("plan");
        assert_eq!(
            step_names(&plan),
            ["just_frostice482-Amulet", "spectralpack-Cryptid"]
        );
        assert_eq!(plan.steps[0].title, "Amulet");
        assert_eq!(plan.steps[0].catalog_id, "frostice482@Amulet");
        assert_eq!(plan.steps[0].min_version.as_deref(), Some("3.5.4"));
        assert_eq!(plan.steps[0].version, "3.6.1");

        // Amulet installed from the index counts as present, an old version only warns
        let installed = [row("Amulet", SOURCE_INDEX, None, "v3.4.0")];
        let plan = plan_install(
            &ctx(&catalog, &packages, &installed),
            "ts:spectralpack-Cryptid",
            None,
            None,
        )
        .expect("plan");
        assert_eq!(step_names(&plan), ["spectralpack-Cryptid"]);
        assert_eq!(plan.satisfied[0].tracked_name, "Amulet");
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("Amulet v3.4.0 from the mod index is older")),
            "{:?}",
            plan.warnings
        );
        assert_eq!(plan.steps[0].requires, ["just_frostice482-Amulet"]);
    }

    #[test]
    fn deprecated_packages_warn() {
        let packages = fixture_packages();
        let catalog = catalog_items(&packages);
        let plan = plan_install(
            &ctx(&catalog, &packages, &[]),
            "ts:PokermonPlus-Pokermon",
            None,
            None,
        )
        .expect("plan");
        assert_eq!(
            step_names(&plan),
            ["Pokermon-Pokermon", "PokermonPlus-Pokermon"]
        );
        assert!(plan.steps[1].deprecated);
        assert_eq!(plan.steps[0].catalog_id, "InertSteak@Pokermon");
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("deprecated on Thunderstore")),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn cycles_are_reported_and_broken() {
        let packages = vec![
            package("ns-A", "1.0.0", &["ns-B-1.0.0"]),
            package("ns-B", "1.2.0", &["ns-C-1.0.0"]),
            package("ns-C", "1.0.0", &["ns-A-1.0.0", "ns-B-1.0.0"]),
        ];
        let catalog = catalog::merge(Vec::new(), &packages);
        let plan =
            plan_install(&ctx(&catalog, &packages, &[]), "ts:ns-A", None, None).expect("plan");
        assert_eq!(step_names(&plan), ["ns-C", "ns-B", "ns-A"]);
        assert_eq!(
            plan.warnings
                .iter()
                .filter(|w| w.contains("Dependency cycle"))
                .count(),
            2,
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn legacy_steamodded_dependencies_warn() {
        let packages = vec![
            package("old-Mod", "1.0.0", &["Steamopollys-Steamodded-0.9.8"]),
            package(
                "ns-Missing",
                "1.0.0",
                &["ns-Gone-1.0.0", "not a dependency"],
            ),
        ];
        let catalog = catalog::merge(Vec::new(), &packages);
        let mut context = ctx(&catalog, &packages, &[]);
        context.steamodded_installed = true;
        context.steamodded_version = Some("26.829.0".into());
        let plan = plan_install(&context, "ts:old-Mod", None, None).expect("plan");
        let smods = plan.steamodded.as_ref().unwrap();
        assert!(smods.legacy_api);
        assert!(smods.satisfied);
        assert!(plan.warnings[0].contains(LEGACY_STEAMODDED_WARNING));

        let plan = plan_install(&context, "ts:ns-Missing", None, None).expect("plan");
        assert_eq!(step_names(&plan), ["ns-Missing"]);
        assert!(plan.warnings.iter().any(|w| w.contains("ns-Gone")));
        assert!(plan.warnings.iter().any(|w| w.contains("not a dependency")));
    }

    #[test]
    fn source_choice_rules() {
        let packages = fixture_packages();
        let catalog = catalog_items(&packages);
        let pokermon = catalog::find_in(&catalog, "InertSteak@Pokermon").unwrap();
        assert_eq!(pokermon.source, CatalogSource::Merged);
        let index_row = row("Pokermon", SOURCE_INDEX, None, "1.0.0");
        let ts_row = row(
            "Pokermon",
            SOURCE_THUNDERSTORE,
            Some("Pokermon-Pokermon"),
            "3.9.1",
        );

        assert_eq!(
            choose_source(pokermon, None, None),
            Ok(InstallSource::Thunderstore)
        );
        assert_eq!(
            choose_source(pokermon, Some(&index_row), None),
            Ok(InstallSource::Index)
        );
        assert_eq!(
            choose_source(pokermon, Some(&ts_row), None),
            Ok(InstallSource::Thunderstore)
        );
        assert_eq!(
            choose_source(pokermon, Some(&ts_row), Some(InstallSource::Index)),
            Ok(InstallSource::Index)
        );

        let joker = catalog::find_in(&catalog, "ts:nh6574-JokerDisplay").unwrap();
        assert!(choose_source(joker, None, Some(InstallSource::Index)).is_err());
        let talisman = catalog::find_in(&catalog, "MathIsFun0@Talisman").unwrap();
        assert!(choose_source(talisman, None, Some(InstallSource::Thunderstore)).is_err());
        assert_eq!(
            choose_source(talisman, None, None),
            Ok(InstallSource::Index)
        );

        let mut deprecated = pokermon.clone();
        deprecated.thunderstore.as_mut().unwrap().deprecated = true;
        assert_eq!(
            choose_source(&deprecated, None, None),
            Ok(InstallSource::Index)
        );
        let horse = catalog::find_in(&catalog, "ts:SuperDZ555-balatro_horse_mod").unwrap();
        assert_eq!(
            choose_source(horse, None, None),
            Ok(InstallSource::Thunderstore)
        );
    }

    #[test]
    fn tracked_index_rows_keep_their_source_and_switches_are_explicit() {
        let packages = fixture_packages();
        let catalog = catalog_items(&packages);
        let installed = [row("Pokermon", SOURCE_INDEX, None, "1.0.0")];
        let context = ctx(&catalog, &packages, &installed);

        let plan = plan_install(&context, "InertSteak@Pokermon", None, None).expect("plan");
        assert_eq!(plan.source, InstallSource::Index);
        assert_eq!(plan.steps[0].tracked_name.as_deref(), Some("Pokermon"));

        let plan = plan_install(
            &context,
            "InertSteak@Pokermon",
            None,
            Some(InstallSource::Thunderstore),
        )
        .expect("plan");
        assert_eq!(plan.source, InstallSource::Thunderstore);
        let root = plan.steps.last().unwrap();
        assert_eq!(root.tracked_name.as_deref(), Some("Pokermon"));
        assert_eq!(root.tracked_source.as_deref(), Some(SOURCE_INDEX));
        assert_eq!(root.catalog_id, "InertSteak@Pokermon");

        // A Thunderstore only package with the same title is a different mod
        let plan = plan_install(&context, "ts:PokermonPlus-Pokermon", None, None).expect("plan");
        assert_eq!(plan.steps.last().unwrap().tracked_name, None);
        // Its Pokermon dependency is the merged entry the user tracks from the index
        assert_eq!(plan.satisfied[0].tracked_name, "Pokermon");
    }

    #[test]
    fn thunderstore_only_entry_takes_over_an_index_row_the_index_dropped() {
        let packages = fixture_packages();
        let catalog = catalog_items(&packages);
        let mut dropped = row("Joker Display", SOURCE_INDEX, None, "1.0.0");
        dropped.orphaned = true;
        let installed = [dropped];

        let plan = plan_install(
            &ctx(&catalog, &packages, &installed),
            "ts:nh6574-JokerDisplay",
            None,
            None,
        )
        .expect("plan");
        let root = plan.steps.iter().find(|s| s.is_root).unwrap();
        assert_eq!(root.source, InstallSource::Thunderstore);
        assert_eq!(root.tracked_name.as_deref(), Some("Joker Display"));
        assert_eq!(root.tracked_source.as_deref(), Some(SOURCE_INDEX));

        // A row an index entry still lists belongs to that entry
        let mut catalog = catalog;
        catalog.push(index_item("Someone@JokerDisplay", "JokerDisplay", ""));
        let plan = plan_install(
            &ctx(&catalog, &packages, &installed),
            "ts:nh6574-JokerDisplay",
            None,
            None,
        )
        .expect("plan");
        assert!(plan.steps.iter().all(|s| s.tracked_name.is_none()));
    }

    #[test]
    fn index_plans_check_steamodded_and_talisman() {
        let packages = fixture_packages();
        let catalog = catalog_items(&packages);
        let plan = plan_install(
            &ctx(&catalog, &packages, &[]),
            "Someone@BigNumbers",
            None,
            None,
        )
        .expect("plan");
        assert_eq!(plan.source, InstallSource::Index);
        assert_eq!(plan.steps.len(), 1);
        let step = &plan.steps[0];
        assert_eq!(
            step.download_url,
            "https://github.com/someone/bignumbers/archive/refs/heads/main.zip"
        );
        assert_eq!(step.folder_name, "BigNumbers");
        assert_eq!(plan.missing, ["Steamodded", "Talisman"]);
        assert_eq!(plan.steamodded.as_ref().unwrap().min_version, None);

        let mut context = ctx(&catalog, &packages, &[]);
        context.steamodded_installed = true;
        context.talisman_installed = true;
        let plan = plan_install(&context, "Big Numbers", None, None).expect("plan");
        assert!(plan.missing.is_empty());
    }

    #[test]
    fn steamodded_and_lovely_requests() {
        let packages = fixture_packages();
        let catalog = catalog_items(&packages);
        let context = ctx(&catalog, &packages, &[]);
        let plan = plan_install(&context, "ts:Steamodded-Steamodded", Some("1.1814.0"), None)
            .expect("plan");
        assert_eq!(plan.mod_id, catalog::STEAMODDED_ID);
        assert_eq!(plan.steps[0].version, "1.0.0-beta-1814a");
        let plan =
            plan_install(&context, "Steamodded@smods", Some("26.829.0"), None).expect("plan");
        assert_eq!(plan.steps[0].version, "26.829.0");
        let plan = plan_install(&context, "Steamodded@smods", None, None).expect("plan");
        assert_eq!(plan.steps[0].version, "");

        assert!(plan_install(&context, "ts:Thunderstore-lovely", None, None).is_err());
        assert!(plan_install(&context, "ts:nh6574-JokerDisplay", Some("0.0.1"), None).is_err());
        assert!(plan_install(&context, "Nobody@Nothing", None, None).is_err());
    }

    #[test]
    fn thunderstore_rows_check_updates_by_source_id() {
        let latest = thunderstore::latest_versions(&fixture_packages());
        let ts = |version: &str| {
            row(
                "JokerDisplay (nh6574)",
                SOURCE_THUNDERSTORE,
                Some("nh6574-jokerdisplay"),
                version,
            )
        };
        assert_eq!(
            thunderstore_update_available(&ts("2.0.4"), &latest),
            Some(true)
        );
        assert_eq!(
            thunderstore_update_available(&ts("2.0.5"), &latest),
            Some(false)
        );
        assert_eq!(
            thunderstore_update_available(&ts("2.0.10"), &latest),
            Some(false)
        );
        assert_eq!(thunderstore_update_available(&ts(""), &latest), Some(false));
        let gone = row("Gone", SOURCE_THUNDERSTORE, Some("ns-Gone"), "1.0.0");
        assert_eq!(thunderstore_update_available(&gone, &latest), Some(false));
        let index = row("JokerDisplay", SOURCE_INDEX, None, "1.0.0");
        assert_eq!(thunderstore_update_available(&index, &latest), None);
    }

    #[test]
    fn deprecated_rows_are_listed_separately() {
        let installed = [
            row(
                "Horse",
                SOURCE_THUNDERSTORE,
                Some("SuperDZ555-balatro_horse_mod"),
                "1.1.2",
            ),
            row(
                "JokerDisplay",
                SOURCE_THUNDERSTORE,
                Some("nh6574-JokerDisplay"),
                "2.0.5",
            ),
            row("balatro horse mod", SOURCE_INDEX, None, "1.0.0"),
        ];
        assert_eq!(
            deprecated_installed(&installed, &fixture_packages()),
            ["Horse"]
        );
    }

    #[test]
    fn steamodded_version_comes_from_the_install() {
        assert_eq!(
            parse_version_lua("return \"1.0.0~BETA-1016c\"\n").as_deref(),
            Some("1.0.0~BETA-1016c")
        );
        assert_eq!(
            parse_version_lua("return '26.829.0'").as_deref(),
            Some("26.829.0")
        );
        assert_eq!(parse_version_lua("return nil"), None);

        let td = tempfile::tempdir().unwrap();
        let smods = td.path().join("Steamodded");
        std::fs::create_dir_all(&smods).unwrap();
        std::fs::write(smods.join("version.lua"), "return \"26.829.0\"").unwrap();
        let mut tracked = row("Steamodded", SOURCE_INDEX, None, "26.1002.0");
        tracked.path = smods.to_string_lossy().to_string();
        assert_eq!(
            installed_steamodded(std::slice::from_ref(&tracked), None),
            (true, Some("26.1002.0".to_string()))
        );
        tracked.current_version = Some("d3c022d".into());
        assert_eq!(
            installed_steamodded(std::slice::from_ref(&tracked), None),
            (true, Some("26.829.0".to_string()))
        );
        assert_eq!(installed_steamodded(&[], Some(td.path())), (false, None));
    }

    /// Plans against the real Thunderstore listing without installing or caching anything:
    /// `cargo test live_plan -- --ignored --nocapture`
    #[test]
    #[ignore = "network: plans against the live Thunderstore listing"]
    fn live_plan_joker_display_and_speedrun() {
        tauri::async_runtime::block_on(async {
            let mut cache = thunderstore::TsCache::empty();
            thunderstore::refresh(&mut cache, &thunderstore::HttpFetcher, 0)
                .await
                .expect("listing");
            let packages = cache.packages;
            let catalog = catalog::merge(Vec::new(), &packages);
            let context = ctx(&catalog, &packages, &[]);

            for id in [
                "ts:nh6574-JokerDisplay",
                "ts:BalatroMultiplayer-MultiplayerSpeedrun",
            ] {
                let plan = plan_install(&context, id, None, None).expect(id);
                println!(
                    "{id}: steps {:?}, steamodded {:?}, lovely {:?}, missing {:?}, warnings {:?}",
                    plan.steps
                        .iter()
                        .map(|s| format!("{}@{}", s.folder_name, s.version))
                        .collect::<Vec<_>>(),
                    plan.steamodded,
                    plan.lovely,
                    plan.missing,
                    plan.warnings
                );
                for step in &plan.steps {
                    let (ns, name) = thunderstore::split_full_name(&step.folder_name).unwrap();
                    bmm_lib::installer::validate_thunderstore_download_url(
                        &step.download_url,
                        ns,
                        name,
                    )
                    .expect("official download URL");
                }
            }

            let speedrun = plan_install(
                &context,
                "ts:BalatroMultiplayer-MultiplayerSpeedrun",
                None,
                None,
            )
            .unwrap();
            assert!(
                speedrun
                    .steps
                    .iter()
                    .any(|s| s.folder_name == "BalatroMultiplayer-MultiplayerAPI")
            );
            assert!(speedrun.steamodded.is_some());
        });
    }
}
