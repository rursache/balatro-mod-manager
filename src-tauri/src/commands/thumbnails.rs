use crate::commands::repo::{get_cached_thumbnail_by_title, thumb_key};

fn is_http_url(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("https://") || url.starts_with("http://")
}

/// Cached thumbnail path for a mod, downloaded first when it is missing or expired. It is
/// keyed by `id` (the catalog id) when given so index and Thunderstore entries never share
/// a file, else by `title`
///
/// Cards call this as they come into view, None means the fetch failed or was cancelled
#[tauri::command]
pub async fn fetch_thumbnail(
    title: String,
    url: String,
    id: Option<String>,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Option<String>, String> {
    let key = thumb_key(&title, id.as_deref());
    if !is_http_url(&url) {
        return get_cached_thumbnail_by_title(key, None).await;
    }
    if !state.thumbs.fetch(key.clone(), url).await {
        return Ok(None);
    }
    get_cached_thumbnail_by_title(key, None).await
}

/// Called when a card scrolls out of view before its thumbnail arrived
#[tauri::command]
pub fn cancel_thumbnail(
    title: String,
    id: Option<String>,
    state: tauri::State<'_, crate::state::AppState>,
) {
    state.thumbs.cancel(&thumb_key(&title, id.as_deref()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_urls_are_fetched() {
        assert!(is_http_url("https://raw.githubusercontent.com/a/b/c.jpg"));
        assert!(is_http_url(" http://example.com/a.jpg "));
        assert!(is_http_url(
            "https://gcdn.thunderstore.io/live/repository/icons/nh6574-JokerDisplay-2.0.5.png"
        ));
        assert!(!is_http_url(""));
        assert!(!is_http_url("file:///etc/passwd"));
        assert!(!is_http_url("/images/cover.jpg"));
    }
}
