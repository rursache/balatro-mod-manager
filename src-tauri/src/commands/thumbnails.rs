use crate::commands::repo::get_cached_thumbnail_by_title;

fn is_http_url(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("https://") || url.starts_with("http://")
}

/// Cached thumbnail path for `title`, downloaded first when it is missing or expired
///
/// Cards call this as they come into view, None means the fetch failed or was cancelled
#[tauri::command]
pub async fn fetch_thumbnail(
    title: String,
    url: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Option<String>, String> {
    if !is_http_url(&url) {
        return get_cached_thumbnail_by_title(title).await;
    }
    if !state.thumbs.fetch(title.clone(), url).await {
        return Ok(None);
    }
    get_cached_thumbnail_by_title(title).await
}

/// Called when a card scrolls out of view before its thumbnail arrived
#[tauri::command]
pub fn cancel_thumbnail(title: String, state: tauri::State<'_, crate::state::AppState>) {
    state.thumbs.cancel(&title);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_urls_are_fetched() {
        assert!(is_http_url("https://raw.githubusercontent.com/a/b/c.jpg"));
        assert!(is_http_url(" http://example.com/a.jpg "));
        assert!(!is_http_url(""));
        assert!(!is_http_url("file:///etc/passwd"));
        assert!(!is_http_url("/images/cover.jpg"));
    }
}
