use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use reqwest::StatusCode;

use crate::assets::{THUMB_CACHE_TTL_SECS, ensure_assets_dirs, safe_slug};
use tokio::sync::{Semaphore, mpsc};
use tokio::time::{Duration, sleep};

/// Background thumbnail fetch request
#[derive(Clone, Debug)]
struct ThumbReq {
    title: String,
    url: String,
    attempts: u32,
    priority: bool, // High priority requests (visible thumbnails) are processed first
}

/// Manager that rate-limits and retries thumbnail downloads in the background.
/// It honors 429 Retry-After when present, and uses exponential backoff for 5xx/network errors.
/// Supports priority queueing for visible thumbnails.
#[derive(Clone)]
pub struct ThumbnailManager {
    tx_high: mpsc::Sender<ThumbReq>, // High priority channel (visible thumbnails)
    tx_low: mpsc::Sender<ThumbReq>,  // Low priority channel (background prefetch)
    // Prevent duplicate queueing per title within a session
    enqueued: Arc<Mutex<HashSet<String>>>,
}

/// Upper bound on the dedupe set. Exceeding this triggers a trim down to
/// `ENQUEUED_TRIM_TO`, so memory stays O(1) even if a session enqueues
/// many distinct titles.
const ENQUEUED_CAP: usize = 4096;
const ENQUEUED_TRIM_TO: usize = 2048;

impl ThumbnailManager {
    pub fn new() -> Self {
        // Smaller bounded queues to avoid memory spikes - high priority gets processed first
        let (tx_high, mut rx_high) = mpsc::channel::<ThumbReq>(128);
        let (tx_low, mut rx_low) = mpsc::channel::<ThumbReq>(256);
        let enqueued: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        // Index thumbnails are full size originals, keep the load on raw.githubusercontent.com modest
        let semaphore = Arc::new(Semaphore::new(6));
        let client = crate::mod_index::http_client().clone();

        // Spawn dispatcher task that prioritizes high-priority requests
        let enq_for_task = enqueued.clone();
        let tx_high_for_dispatch = tx_high.clone();
        let tx_low_for_dispatch = tx_low.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                // Try high priority first (with timeout), then fall back to low priority
                let req = tokio::select! {
                    biased;
                    Some(req) = rx_high.recv() => Some(req),
                    Some(req) = rx_low.recv() => Some(req),
                    else => None,
                };

                let Some(mut req) = req else {
                    // Both channels closed
                    break;
                };

                // Skip if file already exists or has been de-duped
                if file_exists_for_title_async(&req.title).await {
                    // Remove from enqueued set so future explicit requests are allowed
                    if let Ok(mut set) = enq_for_task.lock() {
                        set.remove(&req.title);
                    }
                    continue;
                }

                let semaphore = semaphore.clone();
                let client = client.clone();
                let enq_set = enq_for_task.clone();
                let tx_retry = if req.priority {
                    tx_high_for_dispatch.clone()
                } else {
                    tx_low_for_dispatch.clone()
                };
                tauri::async_runtime::spawn(async move {
                    // Acquire permit inside the spawned task to avoid blocking the dispatcher
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => return, // Semaphore closed
                    };
                    match fetch_and_store(&client, &req.title, &req.url).await {
                        Ok(true) => {
                            if let Ok(mut set) = enq_set.lock() {
                                set.remove(&req.title);
                            }
                        }
                        Ok(false) => {
                            // Non-retryable (e.g., 404/unsupported), drop and clear
                            if let Ok(mut set) = enq_set.lock() {
                                set.remove(&req.title);
                            }
                        }
                        Err(backoff) => {
                            let delay = match backoff {
                                Backoff::RetryAfter(delay) => delay,
                                Backoff::RetryWith(delay, url) => {
                                    req.url = url;
                                    delay
                                }
                            };
                            req.attempts = req.attempts.saturating_add(1);
                            if req.attempts > 3 {
                                if let Ok(mut set) = enq_set.lock() {
                                    set.remove(&req.title);
                                }
                                return;
                            }
                            let title = req.title.clone();
                            tauri::async_runtime::spawn(async move {
                                sleep(delay).await;
                                // Put back into queue, keep enqueued flag as-is
                                let _ = tx_retry.send(req).await;
                                // If send fails, allow future enqueue by clearing mark
                                if let Ok(mut set) = enq_set.lock() {
                                    set.remove(&title);
                                }
                            });
                        }
                    }
                });
            }
        });

        Self {
            tx_high,
            tx_low,
            enqueued,
        }
    }

    /// Enqueue a single thumbnail request if not already present and not already cached.
    /// Use `priority = true` for visible thumbnails to download them first.
    pub fn enqueue(&self, title: String, url: String) {
        self.enqueue_with_priority(title, url, false);
    }

    /// Enqueue a single thumbnail with explicit priority.
    /// High priority thumbnails (visible on screen) are processed before low priority ones.
    pub fn enqueue_with_priority(&self, title: String, url: String, priority: bool) {
        // Use sync file check here since we're in a sync context
        if file_exists_for_title_sync(&title) {
            return;
        }
        // Insert into dedupe set. If it would exceed the cap, drop the oldest
        // half (FIFO-ish — HashSet iteration order is arbitrary but stable
        // enough here) so the set cannot grow without bound when the queue
        // sees thousands of distinct titles per session.
        if let Ok(mut set) = self.enqueued.lock() {
            if !set.insert(title.clone()) {
                return; // already queued
            }
            if set.len() > ENQUEUED_CAP {
                let drop_n = set.len() - ENQUEUED_TRIM_TO;
                let stale: Vec<String> = set.iter().take(drop_n).cloned().collect();
                for k in stale {
                    set.remove(&k);
                }
            }
        }
        let req = ThumbReq {
            title: title.clone(),
            url,
            attempts: 0,
            priority,
        };
        let send_result = if priority {
            self.tx_high.try_send(req)
        } else {
            self.tx_low.try_send(req)
        };
        if send_result.is_err() {
            // Channel full or closed. Clear the dedupe entry so a later call
            // can retry instead of leaving the title pinned forever.
            if let Ok(mut set) = self.enqueued.lock() {
                set.remove(&title);
            }
        }
    }

    /// Enqueue multiple thumbnail requests.
    pub fn enqueue_many(&self, items: impl IntoIterator<Item = (String, String)>) {
        for (title, url) in items {
            self.enqueue(title, url);
        }
    }

    /// Enqueue multiple thumbnail requests with priority support.
    /// Items are tuples of (title, url, priority).
    pub fn enqueue_many_with_priority(
        &self,
        items: impl IntoIterator<Item = (String, String, bool)>,
    ) {
        for (title, url, priority) in items {
            self.enqueue_with_priority(title, url, priority);
        }
    }
}

impl Default for ThumbnailManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that indicate we should retry later with a delay
enum Backoff {
    RetryAfter(Duration),
    /// Retry against another URL, used to move from raw.githubusercontent.com to the mirror
    RetryWith(Duration, String),
}

/// The mirror URL to use when raw.githubusercontent.com throttles or refuses a thumbnail
fn mirror_on_throttle(status: StatusCode, url: &str) -> Option<String> {
    if status != StatusCode::TOO_MANY_REQUESTS && status != StatusCode::FORBIDDEN {
        return None;
    }
    crate::mod_index::mirror_for(url)
}

async fn fetch_and_store(
    client: &reqwest::Client,
    title: &str,
    url: &str,
) -> Result<bool, Backoff> {
    log::info!("Thumbnail fetch start: title='{}' url='{}'", title, url);
    // Don't waste network if already cached
    if file_exists_for_title_async(title).await {
        return Ok(false);
    }

    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(_) => return Err(Backoff::RetryAfter(jitter(Duration::from_secs(3)))),
    };

    if let Some(mirror) = mirror_on_throttle(resp.status(), url) {
        log::info!(
            "Thumbnail throttled, retrying via mirror: title='{}'",
            title
        );
        return Err(Backoff::RetryWith(jitter(Duration::from_secs(1)), mirror));
    }

    match resp.status() {
        StatusCode::OK => {
            let bytes = match resp.bytes().await {
                Ok(b) if !b.is_empty() => b,
                _ => return Ok(false),
            };
            if write_thumbnail_async(title, &bytes).await.is_err() {
                // Disk error; drop silently
                return Ok(false);
            }
            log::info!("Thumbnail saved: title='{}'", title);
            Ok(true)
        }
        StatusCode::TOO_MANY_REQUESTS => {
            let delay =
                retry_after_delay(resp.headers()).unwrap_or_else(|| jitter(Duration::from_secs(5)));
            Err(Backoff::RetryAfter(delay))
        }
        s if s.is_server_error() => Err(Backoff::RetryAfter(jitter(Duration::from_secs(4)))),
        StatusCode::NOT_FOUND | StatusCode::GONE => Ok(false),
        _ => Ok(false),
    }
}

fn jitter(base: Duration) -> Duration {
    // Small jitter based on current time millis; avoids extra deps
    use std::time::{SystemTime, UNIX_EPOCH};
    let base_ms = base.as_millis() as u64;
    let wiggle_base = (base_ms / 3).max(1);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let wiggle = now_ms % wiggle_base;
    Duration::from_millis(base_ms + wiggle)
}

fn retry_after_delay(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    use reqwest::header::RETRY_AFTER;
    if let Some(val) = headers.get(RETRY_AFTER)
        && let Ok(s) = val.to_str()
    {
        // Either seconds or HTTP-date
        if let Ok(secs) = s.trim().parse::<u64>() {
            return Some(Duration::from_secs(secs));
        }
        if let Ok(target) = httpdate::parse_http_date(s) {
            // Convert to duration from now; guard against past
            if let Ok(diff) = target.duration_since(std::time::SystemTime::now()) {
                return Some(diff);
            }
        }
    }
    None
}

fn file_exists_for_title_sync(title: &str) -> bool {
    let slug = safe_slug(title);
    if let Ok((thumbs, _)) = ensure_assets_dirs() {
        let p = thumbs.join(format!("{slug}.jpg"));
        if !p.exists() {
            return false;
        }
        if !is_thumb_fresh(&p) {
            let _ = std::fs::remove_file(&p);
            return false;
        }
        return true;
    }
    false
}

async fn file_exists_for_title_async(title: &str) -> bool {
    let slug = safe_slug(title);
    if let Ok((thumbs, _)) = ensure_assets_dirs() {
        let p = thumbs.join(format!("{slug}.jpg"));
        match tokio::fs::metadata(&p).await {
            Ok(meta) => {
                if !is_thumb_fresh_from_metadata(&meta) {
                    let _ = tokio::fs::remove_file(&p).await;
                    return false;
                }
                true
            }
            Err(_) => false,
        }
    } else {
        false
    }
}

fn is_thumb_fresh(path: &std::path::Path) -> bool {
    let modified = match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(ts) => ts,
        Err(_) => return false,
    };
    let age = match std::time::SystemTime::now().duration_since(modified) {
        Ok(d) => d,
        Err(_) => return false,
    };
    age.as_secs() < THUMB_CACHE_TTL_SECS
}

fn is_thumb_fresh_from_metadata(meta: &std::fs::Metadata) -> bool {
    let modified = match meta.modified() {
        Ok(ts) => ts,
        Err(_) => return false,
    };
    let age = match std::time::SystemTime::now().duration_since(modified) {
        Ok(d) => d,
        Err(_) => return false,
    };
    age.as_secs() < THUMB_CACHE_TTL_SECS
}

async fn write_thumbnail_async(title: &str, bytes: &[u8]) -> Result<(), String> {
    let slug = safe_slug(title);
    let (thumbs, _) = ensure_assets_dirs()?;
    let path = thumbs.join(format!("{slug}.jpg"));
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    // ==================== jitter tests ====================

    #[test]
    fn test_jitter_returns_at_least_base() {
        let base = Duration::from_secs(3);
        let result = jitter(base);
        assert!(result >= base, "jitter should be >= base duration");
    }

    #[test]
    fn test_jitter_bounded_by_base_plus_third() {
        let base = Duration::from_secs(3);
        let max_expected = base + Duration::from_secs(1); // base + base/3
        let result = jitter(base);
        assert!(
            result <= max_expected,
            "jitter should be <= base + base/3, got {:?}",
            result
        );
    }

    #[test]
    fn test_jitter_small_duration() {
        let base = Duration::from_millis(100);
        let result = jitter(base);
        assert!(result >= base);
        // With base=100ms, wiggle_base = 33ms, so max jitter is 100+32 = 132ms
        assert!(result <= Duration::from_millis(133));
    }

    #[test]
    fn test_jitter_zero_duration() {
        let base = Duration::from_millis(0);
        let result = jitter(base);
        // With base=0, wiggle_base = max(0/3, 1) = 1, so jitter is 0 + (now % 1) = 0
        assert_eq!(result, Duration::from_millis(0));
    }

    #[test]
    fn test_jitter_very_small_duration() {
        let base = Duration::from_millis(2);
        let result = jitter(base);
        // With base=2ms, wiggle_base = max(2/3, 1) = 1, so jitter is 2 + (now % 1) = 2
        assert!(result >= base);
    }

    // ==================== retry_after_delay tests ====================

    #[test]
    fn test_retry_after_delay_with_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("120"),
        );
        let result = retry_after_delay(&headers);
        assert_eq!(result, Some(Duration::from_secs(120)));
    }

    #[test]
    fn test_retry_after_delay_with_seconds_whitespace() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("  60  "),
        );
        let result = retry_after_delay(&headers);
        assert_eq!(result, Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_retry_after_delay_missing_header() {
        let headers = reqwest::header::HeaderMap::new();
        let result = retry_after_delay(&headers);
        assert_eq!(result, None);
    }

    #[test]
    fn test_retry_after_delay_invalid_value() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("not-a-number"),
        );
        let result = retry_after_delay(&headers);
        // Not a number and not a valid HTTP date
        assert_eq!(result, None);
    }

    #[test]
    fn test_retry_after_delay_zero_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("0"),
        );
        let result = retry_after_delay(&headers);
        assert_eq!(result, Some(Duration::from_secs(0)));
    }

    #[test]
    fn test_retry_after_delay_large_value() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("3600"),
        );
        let result = retry_after_delay(&headers);
        assert_eq!(result, Some(Duration::from_secs(3600)));
    }

    // ==================== is_thumb_fresh tests ====================

    #[test]
    fn test_is_thumb_fresh_new_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.jpg");
        std::fs::write(&path, b"test data").unwrap();

        // File just created should be fresh
        assert!(is_thumb_fresh(&path));
    }

    #[test]
    fn test_is_thumb_fresh_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.jpg");

        assert!(!is_thumb_fresh(&path));
    }

    // ==================== is_thumb_fresh_from_metadata tests ====================

    #[test]
    fn test_is_thumb_fresh_from_metadata_new_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.jpg");
        std::fs::write(&path, b"test data").unwrap();

        let meta = std::fs::metadata(&path).unwrap();
        assert!(is_thumb_fresh_from_metadata(&meta));
    }

    // ==================== THUMB_CACHE_TTL_SECS constant test ====================

    #[test]
    fn test_thumb_cache_ttl_is_30_days() {
        assert_eq!(THUMB_CACHE_TTL_SECS, 60 * 60 * 24 * 30);
    }

    #[test]
    fn test_mirror_on_throttle_only_for_raw_index_urls() {
        let raw = crate::mod_index::raw_url("abc", "mods/A@B/thumbnail.jpg");
        assert_eq!(
            mirror_on_throttle(StatusCode::TOO_MANY_REQUESTS, &raw),
            crate::mod_index::mirror_for(&raw)
        );
        assert!(mirror_on_throttle(StatusCode::FORBIDDEN, &raw).is_some());
        assert_eq!(mirror_on_throttle(StatusCode::NOT_FOUND, &raw), None);
        assert_eq!(
            mirror_on_throttle(StatusCode::TOO_MANY_REQUESTS, "https://example.com/a.jpg"),
            None
        );
    }

    // ==================== ThumbReq tests ====================

    #[test]
    fn test_thumb_req_clone() {
        let req = ThumbReq {
            title: "Test Mod".to_string(),
            url: "http://example.com/thumb.jpg".to_string(),
            attempts: 2,
            priority: true,
        };
        let cloned = req.clone();
        assert_eq!(cloned.title, req.title);
        assert_eq!(cloned.url, req.url);
        assert_eq!(cloned.attempts, req.attempts);
        assert_eq!(cloned.priority, req.priority);
    }

    #[test]
    fn test_thumb_req_debug() {
        let req = ThumbReq {
            title: "Test".to_string(),
            url: "http://example.com".to_string(),
            attempts: 0,
            priority: false,
        };
        let debug_str = format!("{:?}", req);
        assert!(debug_str.contains("ThumbReq"));
        assert!(debug_str.contains("Test"));
    }

    // ==================== Backoff enum tests ====================

    #[test]
    fn test_backoff_retry_after() {
        let backoff = Backoff::RetryAfter(Duration::from_secs(5));
        match backoff {
            Backoff::RetryAfter(d) => assert_eq!(d, Duration::from_secs(5)),
            Backoff::RetryWith(..) => unreachable!(),
        }
    }
}
