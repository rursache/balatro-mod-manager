use crate::assets::{THUMB_CACHE_TTL_SECS, ensure_assets_dirs_async, safe_slug};
use crate::mod_index;
use crate::models::ArchiveModItem;
use bmm_lib::errors::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDetails {
    pub description: String,
    pub requires_steamodded: bool,
    pub requires_talisman: bool,
    pub repo_url: Option<String>,
}

async fn lookup(dir_name: &str, title: &str) -> Option<ArchiveModItem> {
    match mod_index::find_item(dir_name).await {
        Some(item) => Some(item),
        None => mod_index::find_item(title).await,
    }
}

fn repo_url_of(item: &ArchiveModItem) -> Option<String> {
    let repo = item.meta.repo.trim();
    (!repo.is_empty()).then(|| repo.to_string())
}

/// Description, requirements and repo URL of one mod, served from the local index cache
#[tauri::command]
pub async fn get_mod_details(title: String, dir_name: String) -> Result<ModDetails, String> {
    let item = lookup(&dir_name, &title)
        .await
        .ok_or_else(|| format!("'{title}' was not found in the mod index"))?;
    Ok(ModDetails {
        description: item.description.clone(),
        requires_steamodded: item.meta.requires_steamodded,
        requires_talisman: item.meta.requires_talisman,
        repo_url: repo_url_of(&item),
    })
}

#[tauri::command]
pub async fn fetch_repo_mods(force: Option<bool>) -> Result<Vec<ArchiveModItem>, String> {
    mod_index::sync_index(force.unwrap_or(false)).await
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

#[tauri::command]
pub async fn get_cached_thumbnail_by_title(title: String) -> Result<Option<String>, String> {
    let (thumbs_dir, _) = ensure_assets_dirs_async().await?;
    let slug = safe_slug(&title);
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

/// Return a map of title -> cached thumbnail path for the given titles (if present).
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

#[tauri::command]
pub async fn cache_thumbnail_from_url(
    title: String,
    url: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<bool, String> {
    // If present, no-op quickly
    let (thumbs_dir, _) = ensure_assets_dirs_async().await?;
    let slug = safe_slug(&title);
    let path = thumbs_dir.join(format!("{slug}.jpg"));
    if tokio::fs::metadata(&path).await.is_ok() {
        if is_thumb_fresh_async(&path).await {
            return Ok(false);
        }
        let _ = tokio::fs::remove_file(&path).await;
    }

    // Enqueue background fetch with 429-aware backoff; return immediately
    log::info!("Thumbnail enqueue: title='{}' url='{}'", title, url);
    state.thumbs.enqueue(title, url);
    Ok(false)
}

#[tauri::command]
pub async fn get_cached_installed_thumbnail(
    title: String,
    dir_name: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Option<String>, String> {
    let installed = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        db.get_installed_mods()
            .map_err(|e| e.to_string())?
            .into_iter()
            .any(|m| m.name.eq_ignore_ascii_case(&title))
    };
    if !installed {
        return Ok(None);
    }

    let (thumbs_dir, _) = ensure_assets_dirs_async().await?;
    let slug = safe_slug(&title);
    let path = thumbs_dir.join(format!("{slug}.jpg"));
    let path_string = || {
        path.to_str()
            .map(str::to_string)
            .ok_or_else(|| "Failed to convert thumbnail path".to_string())
    };
    if tokio::fs::metadata(&path).await.is_ok() {
        if is_thumb_fresh_async(&path).await {
            return path_string().map(Some);
        }
        let _ = tokio::fs::remove_file(&path).await;
    }

    let Some(item) = lookup(&dir_name, &title).await else {
        return Ok(None);
    };
    let url = item.image_url;
    if url.is_empty() {
        return Ok(None);
    }

    let fetched = match mod_index::http_client().get(&url).send().await {
        Ok(resp) if resp.status().is_success() => resp.bytes().await.ok(),
        _ => None,
    };
    match fetched {
        Some(bytes) if !bytes.is_empty() => {
            tokio::fs::write(&path, &bytes).await.map_err(|e| {
                AppError::FileWrite {
                    path: path.clone(),
                    source: e.to_string(),
                }
                .to_string()
            })?;
            path_string().map(Some)
        }
        _ => {
            // Defer retry (and the mirror fallback) to the background queue
            state.thumbs.enqueue(title, url);
            Ok(None)
        }
    }
}

#[tauri::command]
pub async fn get_description_cached_or_remote(
    title: String,
    dir_name: String,
) -> Result<String, String> {
    let (_, descs_dir) = ensure_assets_dirs_async().await?;
    let path = descs_dir.join(format!("{}.md", safe_slug(&title)));

    if let Some(item) = lookup(&dir_name, &title).await
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
    let item = mod_index::find_item(&dir_name)
        .await
        .ok_or_else(|| format!("'{dir_name}' was not found in the mod index"))?;
    Ok((item.meta.requires_steamodded, item.meta.requires_talisman))
}

#[tauri::command]
pub async fn get_mod_repo_url(dir_name: String) -> Result<Option<String>, String> {
    Ok(mod_index::find_item(&dir_name)
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
        };
        assert_eq!(repo_url_of(&item), None);
        item.meta.repo = " https://github.com/a/b ".into();
        assert_eq!(
            repo_url_of(&item).as_deref(),
            Some("https://github.com/a/b")
        );
    }
}
