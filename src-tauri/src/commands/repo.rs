use crate::assets::{THUMB_CACHE_TTL_SECS, ensure_assets_dirs_async, safe_slug};
use crate::catalog::{self, CatalogStatus};
use crate::models::{ArchiveModItem, CatalogSource, ThunderstoreInfo};
use crate::state::AppState;
use crate::thunderstore;
use bmm_lib::errors::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDetails {
    pub description: String,
    pub requires_steamodded: bool,
    pub requires_talisman: bool,
    pub repo_url: Option<String>,
    #[serde(default)]
    pub source: CatalogSource,
    #[serde(default)]
    pub thunderstore: Option<ThunderstoreInfo>,
    #[serde(default)]
    pub downloads: Option<u64>,
}

async fn lookup(dir_name: &str, title: &str) -> Option<ArchiveModItem> {
    match catalog::find_item(dir_name).await {
        Some(item) => Some(item),
        None => catalog::find_item(title).await,
    }
}

/// Thumbnail cache key, the catalog id when the caller has one so sources cannot collide
pub(crate) fn thumb_key(title: &str, id: Option<&str>) -> String {
    id.map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(title)
        .to_string()
}

/// The index `description.md` text, None when the entry only has Thunderstore text
fn index_description(item: &ArchiveModItem) -> Option<&str> {
    if item.source == CatalogSource::Thunderstore || item.description.trim().is_empty() {
        return None;
    }
    let from_thunderstore = item
        .thunderstore
        .as_ref()
        .is_some_and(|t| t.description == item.description);
    (!from_thunderstore).then_some(item.description.as_str())
}

/// README of the latest version, cached on disk per version. Only the detail view asks for
/// it, cards use the short description so the catalog never bursts README requests
async fn thunderstore_readme(info: &ThunderstoreInfo) -> Option<String> {
    let (_, descs_dir) = ensure_assets_dirs_async().await.ok()?;
    let path = descs_dir.join(format!(
        "{}@{}.md",
        safe_slug(&info.full_name),
        safe_slug(&info.latest_version)
    ));
    match thunderstore::readme_cached(&info.namespace, &info.name, &info.latest_version, &path)
        .await
    {
        Ok(text) if !text.trim().is_empty() => Some(text),
        Ok(_) => None,
        Err(e) => {
            log::warn!("Failed to fetch README of {}: {e}", info.full_name);
            None
        }
    }
}

/// Short text for a card, never touches the network
fn card_description(item: &ArchiveModItem) -> String {
    if let Some(text) = index_description(item) {
        return text.to_string();
    }
    item.thunderstore
        .as_ref()
        .map(|t| t.description.trim())
        .filter(|d| !d.is_empty())
        .map_or_else(|| item.description.clone(), str::to_string)
}

/// Index description, then the Thunderstore README, then the short description
async fn describe(item: &ArchiveModItem) -> String {
    if let Some(text) = index_description(item) {
        return text.to_string();
    }
    if let Some(info) = &item.thunderstore {
        if let Some(readme) = thunderstore_readme(info).await {
            return readme;
        }
        if !info.description.trim().is_empty() {
            return info.description.clone();
        }
    }
    item.description.clone()
}

fn repo_url_of(item: &ArchiveModItem) -> Option<String> {
    let repo = item.meta.repo.trim();
    (!repo.is_empty()).then(|| repo.to_string())
}

/// Description, requirements and repo URL of one mod, served from the local catalog cache
/// (Thunderstore READMEs are fetched once per version)
#[tauri::command]
pub async fn get_mod_details(title: String, dir_name: String) -> Result<ModDetails, String> {
    let item = lookup(&dir_name, &title)
        .await
        .ok_or_else(|| format!("'{title}' was not found in the mod catalog"))?;
    Ok(ModDetails {
        description: describe(&item).await,
        requires_steamodded: item.meta.requires_steamodded,
        requires_talisman: item.meta.requires_talisman,
        repo_url: repo_url_of(&item),
        source: item.source,
        downloads: item.downloads,
        thunderstore: item.thunderstore,
    })
}

/// Merged GitHub index and Thunderstore catalog. `thunderstore` carries the UI setting so
/// the first sync after launch already honors it
#[tauri::command]
pub async fn fetch_repo_mods(
    state: tauri::State<'_, AppState>,
    force: Option<bool>,
    thunderstore: Option<bool>,
) -> Result<Vec<ArchiveModItem>, String> {
    if let Some(enabled) = thunderstore
        && enabled != thunderstore::is_enabled()
    {
        thunderstore::set_enabled(enabled);
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = db.set_thunderstore_enabled(enabled) {
            log::warn!("Failed to persist the Thunderstore setting: {e}");
        }
    }
    catalog::sync_all(force.unwrap_or(false)).await
}

/// Per source sync state of the catalog, never touches the network
#[tauri::command]
pub async fn catalog_status() -> Result<CatalogStatus, String> {
    Ok(catalog::status().await)
}

async fn is_thumb_fresh_async(path: &std::path::Path) -> bool {
    let modified = match tokio::fs::metadata(path).await.and_then(|m| m.modified()) {
        Ok(ts) => ts,
        Err(_) => return false,
    };
    let age = match std::time::SystemTime::now().duration_since(modified) {
        Ok(d) => d,
        Err(_) => return false,
    };
    age.as_secs() < THUMB_CACHE_TTL_SECS
}

/// Cached thumbnail path, keyed by `id` (the catalog id) when given, else by `title`
#[tauri::command]
pub async fn get_cached_thumbnail_by_title(
    title: String,
    id: Option<String>,
) -> Result<Option<String>, String> {
    let (thumbs_dir, _) = ensure_assets_dirs_async().await?;
    let slug = safe_slug(&thumb_key(&title, id.as_deref()));
    let path = thumbs_dir.join(format!("{slug}.jpg"));
    if tokio::fs::metadata(&path).await.is_err() {
        return Ok(None);
    }
    if !is_thumb_fresh_async(&path).await {
        let _ = tokio::fs::remove_file(&path).await;
        return Ok(None);
    }
    Ok(Some(
        path.to_str()
            .ok_or_else(|| format!("Failed to convert thumbnail path: {}", path.display()))?
            .to_string(),
    ))
}

/// Map of key -> cached thumbnail path for the given keys (catalog ids, or titles for
/// thumbnails cached before ids were used)
#[tauri::command]
pub async fn get_cached_thumbnails_map(
    titles: Vec<String>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let (thumbs_dir, _) = ensure_assets_dirs_async().await?;
    let mut out = std::collections::HashMap::new();
    for title in titles {
        let slug = safe_slug(&title);
        let path = thumbs_dir.join(format!("{slug}.jpg"));
        let exists = tokio::fs::metadata(&path).await.is_ok();
        if exists && is_thumb_fresh_async(&path).await {
            if let Some(s) = path.to_str() {
                out.insert(title, s.to_string());
            }
        } else if exists {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
    Ok(out)
}

/// Card description from the catalog, Thunderstore READMEs are left to `get_mod_details`
#[tauri::command]
pub async fn get_description_cached_or_remote(
    title: String,
    dir_name: String,
) -> Result<String, String> {
    let (_, descs_dir) = ensure_assets_dirs_async().await?;
    let path = descs_dir.join(format!("{}.md", safe_slug(&title)));
    let item = lookup(&dir_name, &title).await;

    if let Some(item) = &item
        && index_description(item).is_none()
        && item.thunderstore.is_some()
    {
        return Ok(card_description(item));
    }

    if let Some(item) = item
        && !item.description.trim().is_empty()
    {
        let cached = tokio::fs::read_to_string(&path).await.ok();
        if cached.as_deref() != Some(item.description.as_str())
            && let Err(e) = tokio::fs::write(&path, &item.description).await
        {
            log::warn!("Failed to cache description for {}: {}", title, e);
        }
        return Ok(item.description);
    }

    Ok(tokio::fs::read_to_string(&path).await.unwrap_or_default())
}

#[tauri::command]
pub async fn get_mod_requirements(dir_name: String) -> Result<(bool, bool), String> {
    let item = catalog::find_item(&dir_name)
        .await
        .ok_or_else(|| format!("'{dir_name}' was not found in the mod catalog"))?;
    Ok((item.meta.requires_steamodded, item.meta.requires_talisman))
}

#[tauri::command]
pub async fn get_mod_repo_url(dir_name: String) -> Result<Option<String>, String> {
    Ok(catalog::find_item(&dir_name)
        .await
        .and_then(|item| repo_url_of(&item)))
}

#[tauri::command]
pub async fn get_cached_description_by_title(title: String) -> Result<Option<String>, String> {
    let (_, descs_dir) = ensure_assets_dirs_async().await?;
    let slug = safe_slug(&title);
    let path = descs_dir.join(format!("{slug}.md"));
    if tokio::fs::metadata(&path).await.is_err() {
        return Ok(None);
    }
    let text = tokio::fs::read_to_string(&path).await.map_err(|e| {
        AppError::FileRead {
            path: path.clone(),
            source: e.to_string(),
        }
        .to_string()
    })?;
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModMeta;

    #[test]
    fn repo_url_of_ignores_blank_repo() {
        let mut item = ArchiveModItem {
            dir_name: "a@b".into(),
            meta: ModMeta {
                repo: "  ".into(),
                ..Default::default()
            },
            description: String::new(),
            image_url: String::new(),
            has_thumbnail: false,
            ..Default::default()
        };
        assert_eq!(repo_url_of(&item), None);
        item.meta.repo = " https://github.com/a/b ".into();
        assert_eq!(
            repo_url_of(&item).as_deref(),
            Some("https://github.com/a/b")
        );
    }

    #[test]
    fn thumb_key_prefers_the_catalog_id() {
        let ts = thumb_key("Pokermon", Some("ts:PokermonPlus-Pokermon"));
        assert_eq!(ts, "ts:PokermonPlus-Pokermon");
        assert_eq!(thumb_key("Pokermon", Some("  ")), "Pokermon");
        assert_eq!(thumb_key("Pokermon", None), "Pokermon");
        assert_ne!(
            safe_slug(&ts),
            safe_slug(&thumb_key("Pokermon", Some("InertSteak@Pokermon")))
        );
    }

    #[test]
    fn index_description_skips_thunderstore_text() {
        let info = ThunderstoreInfo {
            description: "short".into(),
            ..Default::default()
        };
        let mut item = ArchiveModItem {
            description: "from index".into(),
            source: CatalogSource::Merged,
            thunderstore: Some(info),
            ..Default::default()
        };
        assert_eq!(index_description(&item), Some("from index"));
        item.description = "short".into();
        assert_eq!(index_description(&item), None);
        item.description = "anything".into();
        item.source = CatalogSource::Thunderstore;
        assert_eq!(index_description(&item), None);
        item.source = CatalogSource::Index;
        item.description = " ".into();
        assert_eq!(index_description(&item), None);
    }

    #[test]
    fn card_description_never_needs_the_readme() {
        let info = ThunderstoreInfo {
            description: " short ".into(),
            ..Default::default()
        };
        let mut item = ArchiveModItem {
            description: "short".into(),
            source: CatalogSource::Thunderstore,
            thunderstore: Some(info),
            ..Default::default()
        };
        assert_eq!(card_description(&item), "short");
        item.source = CatalogSource::Merged;
        item.description = "from index".into();
        assert_eq!(card_description(&item), "from index");
        item.description = String::new();
        item.thunderstore = Some(ThunderstoreInfo::default());
        assert_eq!(card_description(&item), "");
    }
}
