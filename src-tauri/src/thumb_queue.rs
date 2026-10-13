use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex, MutexGuard};

use reqwest::StatusCode;

use crate::assets::{THUMB_CACHE_TTL_SECS, ensure_assets_dirs, safe_slug};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::time::{Duration, sleep};

#[derive(Clone, Debug)]
struct ThumbReq {
    title: String,
    url: String,
    attempts: u32,
}

/// One queued download plus the callers waiting on it, `interest` drops when a card
/// scrolls away and a job nobody wants anymore is skipped before it hits the network
#[derive(Default)]
struct Job {
    interest: usize,
    waiters: Vec<oneshot::Sender<bool>>,
}

type JobMap = Mutex<HashMap<String, Job>>;

fn lock_jobs(jobs: &JobMap) -> MutexGuard<'_, HashMap<String, Job>> {
    jobs.lock().unwrap_or_else(|e| e.into_inner())
}

/// Adds a waiter for `title`, returns true when a new download has to be queued
fn register(jobs: &JobMap, title: &str, waiter: oneshot::Sender<bool>) -> bool {
    let mut jobs = lock_jobs(jobs);
    let (job, is_new) = match jobs.entry(title.to_string()) {
        Entry::Occupied(e) => (e.into_mut(), false),
        Entry::Vacant(e) => (e.insert(Job::default()), true),
    };
    job.interest += 1;
    job.waiters.push(waiter);
    is_new
}

fn release(jobs: &JobMap, title: &str) {
    if let Some(job) = lock_jobs(jobs).get_mut(title) {
        job.interest = job.interest.saturating_sub(1);
    }
}

fn has_interest(jobs: &JobMap, title: &str) -> bool {
    lock_jobs(jobs).get(title).is_some_and(|j| j.interest > 0)
}

fn finish(jobs: &JobMap, title: &str, cached: bool) {
    let job = lock_jobs(jobs).remove(title);
    for waiter in job.into_iter().flat_map(|j| j.waiters) {
        let _ = waiter.send(cached);
    }
}

/// Downloads thumbnails on demand for cards that are on screen, honors 429 Retry-After,
/// backs off on 5xx/network errors and moves to the jsDelivr mirror when throttled
#[derive(Clone)]
pub struct ThumbnailManager {
    tx: mpsc::Sender<ThumbReq>,
    jobs: Arc<JobMap>,
}

impl ThumbnailManager {
    pub fn new() -> Self {
        let (tx, mut rx) = mpsc::channel::<ThumbReq>(256);
        let jobs: Arc<JobMap> = Arc::default();

        // Index thumbnails are full size originals, keep the load on raw.githubusercontent.com modest
        let semaphore = Arc::new(Semaphore::new(6));
        let client = crate::mod_index::http_client().clone();

        let jobs_for_task = jobs.clone();
        let tx_retry = tx.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(mut req) = rx.recv().await {
                let semaphore = semaphore.clone();
                let client = client.clone();
                let jobs = jobs_for_task.clone();
                let tx_retry = tx_retry.clone();
                tauri::async_runtime::spawn(async move {
                    let Ok(permit) = semaphore.acquire_owned().await else {
                        finish(&jobs, &req.title, false);
                        return;
                    };
                    if !has_interest(&jobs, &req.title) {
                        log::debug!("Thumbnail skipped, no longer visible: '{}'", req.title);
                        finish(&jobs, &req.title, false);
                        return;
                    }
                    let delay = match fetch_and_store(&client, &req.title, &req.url).await {
                        Ok(cached) => {
                            finish(&jobs, &req.title, cached);
                            return;
                        }
                        Err(Backoff::RetryAfter(delay)) => delay,
                        Err(Backoff::RetryWith(delay, url)) => {
                            req.url = url;
                            delay
                        }
                    };
                    drop(permit);
                    req.attempts = req.attempts.saturating_add(1);
                    if req.attempts > 3 {
                        finish(&jobs, &req.title, false);
                        return;
                    }
                    sleep(delay).await;
                    let title = req.title.clone();
                    if tx_retry.send(req).await.is_err() {
                        finish(&jobs, &title, false);
                    }
                });
            }
        });

        Self { tx, jobs }
    }

    /// Resolves once the thumbnail for `title` is on disk (true) or could not be fetched
    pub async fn fetch(&self, title: String, url: String) -> bool {
        if file_exists_for_title_async(&title).await {
            return true;
        }
        let (waiter, done) = oneshot::channel();
        if register(&self.jobs, &title, waiter) {
            let req = ThumbReq {
                title: title.clone(),
                url,
                attempts: 0,
            };
            if self.tx.try_send(req).is_err() {
                finish(&self.jobs, &title, false);
            }
        }
        done.await.unwrap_or(false)
    }

    /// Drops one caller's interest so a download that has not started yet gets skipped
    pub fn cancel(&self, title: &str) {
        release(&self.jobs, title);
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

/// `Ok(true)` when the thumbnail is on disk afterwards, `Ok(false)` when it cannot be fetched
async fn fetch_and_store(
    client: &reqwest::Client,
    title: &str,
    url: &str,
) -> Result<bool, Backoff> {
    // Another caller may have cached it while this request waited for a permit
    if file_exists_for_title_async(title).await {
        return Ok(true);
    }
    log::info!("Thumbnail fetch start: title='{}' url='{}'", title, url);

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
        };
        let cloned = req.clone();
        assert_eq!(cloned.title, req.title);
        assert_eq!(cloned.url, req.url);
        assert_eq!(cloned.attempts, req.attempts);
    }

    #[test]
    fn test_thumb_req_debug() {
        let req = ThumbReq {
            title: "Test".to_string(),
            url: "http://example.com".to_string(),
            attempts: 0,
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

    // ==================== job bookkeeping tests ====================

    #[test]
    fn register_queues_once_per_title() {
        let jobs = JobMap::default();
        let (a, _ra) = oneshot::channel();
        let (b, _rb) = oneshot::channel();
        let (c, _rc) = oneshot::channel();
        assert!(register(&jobs, "A", a));
        assert!(!register(&jobs, "A", b));
        assert!(register(&jobs, "B", c));
        assert_eq!(lock_jobs(&jobs)["A"].interest, 2);
    }

    #[test]
    fn cancel_skips_only_when_every_caller_left() {
        let jobs = JobMap::default();
        let (a, _ra) = oneshot::channel();
        let (b, _rb) = oneshot::channel();
        register(&jobs, "A", a);
        register(&jobs, "A", b);
        release(&jobs, "A");
        assert!(has_interest(&jobs, "A"));
        release(&jobs, "A");
        assert!(!has_interest(&jobs, "A"));
        // Scrolling back before the job starts revives it
        let (c, _rc) = oneshot::channel();
        assert!(!register(&jobs, "A", c));
        assert!(has_interest(&jobs, "A"));
    }

    #[test]
    fn release_unknown_title_is_a_noop() {
        let jobs = JobMap::default();
        release(&jobs, "missing");
        assert!(!has_interest(&jobs, "missing"));
        assert!(lock_jobs(&jobs).is_empty());
    }

    #[test]
    fn finish_notifies_all_waiters_and_clears_job() {
        let jobs = JobMap::default();
        let (a, mut ra) = oneshot::channel();
        let (b, mut rb) = oneshot::channel();
        register(&jobs, "A", a);
        register(&jobs, "A", b);
        finish(&jobs, "A", true);
        assert_eq!(ra.try_recv(), Ok(true));
        assert_eq!(rb.try_recv(), Ok(true));
        assert!(lock_jobs(&jobs).is_empty());

        let (c, mut rc) = oneshot::channel();
        assert!(register(&jobs, "A", c));
        finish(&jobs, "A", false);
        assert_eq!(rc.try_recv(), Ok(false));
    }
}
