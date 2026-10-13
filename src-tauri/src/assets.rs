//! Shared utilities for mod asset management (thumbnails, descriptions).
//!
//! This module consolidates helper functions that were previously duplicated
//! across repo.rs, cache.rs, and thumb_queue.rs.

use std::path::PathBuf;

/// Cached thumbnails are refetched after this long, or sooner when the index blob changes
pub const THUMB_CACHE_TTL_SECS: u64 = 60 * 60 * 24 * 30;

/// Characters considered legal for slug generation.
/// Includes alphanumeric and common punctuation that's safe for filenames.
fn is_legal_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '('
                | ')'
                | '+'
                | ','
                | '-'
                | '='
                | ';'
                | '@'
                | '['
                | ']'
                | '^'
                | '_'
                | '`'
                | '{'
                | '}'
                | '~'
        )
}

/// Convert a mod title to a filesystem-safe slug for caching.
///
/// - Lowercases the input
/// - Replaces non-legal characters with hyphens
/// - Collapses multiple hyphens into one
/// - Trims leading/trailing hyphens
pub fn safe_slug(input: &str) -> String {
    let mut s = input.trim().to_lowercase();
    s = s
        .chars()
        .map(|c| if is_legal_char(c) { c } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s.trim_matches('-').to_string()
}

/// Returns paths to the thumbnails and descriptions cache directories.
/// Creates the directories if they don't exist (synchronous version).
pub fn ensure_assets_dirs() -> Result<(PathBuf, PathBuf), String> {
    let config_dir = dirs::config_dir().ok_or_else(|| "config dir not found".to_string())?;
    let base = config_dir.join("Balatro").join("mod_assets");
    let thumbs = base.join("thumbnails");
    let descs = base.join("descriptions");
    std::fs::create_dir_all(&thumbs).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&descs).map_err(|e| e.to_string())?;
    Ok((thumbs, descs))
}

/// Returns paths to the thumbnails and descriptions cache directories.
/// Creates the directories if they don't exist (async version).
pub async fn ensure_assets_dirs_async() -> Result<(PathBuf, PathBuf), String> {
    let config_dir = dirs::config_dir().ok_or_else(|| "config dir not found".to_string())?;
    let base = config_dir.join("Balatro").join("mod_assets");
    let thumbs = base.join("thumbnails");
    let descs = base.join("descriptions");
    tokio::fs::create_dir_all(&thumbs)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(&descs)
        .await
        .map_err(|e| e.to_string())?;
    Ok((thumbs, descs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_slug_basic() {
        assert_eq!(safe_slug("Hello World"), "hello-world");
        assert_eq!(safe_slug("Steamodded"), "steamodded");
        assert_eq!(safe_slug("Talisman"), "talisman");
    }

    #[test]
    fn test_safe_slug_special_chars() {
        // Parentheses and ampersand are in the legal char list
        assert_eq!(safe_slug("Mod (v1.0)"), "mod-(v1-0)"); // Period is NOT legal, becomes hyphen
        assert_eq!(safe_slug("Test & More"), "test-&-more");
        assert_eq!(safe_slug("Name's Mod"), "name's-mod");
    }

    #[test]
    fn test_safe_slug_unicode() {
        // Unicode gets replaced with hyphens
        assert_eq!(safe_slug("日本語Mod"), "mod");
        assert_eq!(safe_slug("Émoji 🎮 Test"), "moji-test");
    }

    #[test]
    fn test_safe_slug_multiple_hyphens() {
        assert_eq!(
            safe_slug("Test---Multiple----Hyphens"),
            "test-multiple-hyphens"
        );
        assert_eq!(safe_slug("  Trimmed  "), "trimmed");
    }

    #[test]
    fn test_safe_slug_edge_cases() {
        assert_eq!(safe_slug(""), "");
        assert_eq!(safe_slug("   "), "");
        assert_eq!(safe_slug("---"), "");
    }

    #[test]
    fn test_is_legal_char() {
        // Alphanumeric
        assert!(is_legal_char('a'));
        assert!(is_legal_char('Z'));
        assert!(is_legal_char('5'));

        // Allowed punctuation
        assert!(is_legal_char('!'));
        assert!(is_legal_char('-'));
        assert!(is_legal_char('_'));
        assert!(is_legal_char('('));
        assert!(is_legal_char(')'));

        // Not allowed
        assert!(!is_legal_char(' '));
        assert!(!is_legal_char('/'));
        assert!(!is_legal_char('\\'));
        assert!(!is_legal_char(':'));
        assert!(!is_legal_char('日'));
    }
}
