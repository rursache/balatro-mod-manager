// Version resolution shared by build.rs (via #[path]) and the crate tests
// Precedence: BMM_VERSION override > exact git tag > short commit hash > "dev"

pub const FALLBACK_VERSION: &str = "dev";

/// Trims the value and drops a leading "v" so tags like "v1.2.3" read "1.2.3"
pub fn normalize(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(trimmed);
    (!stripped.is_empty()).then(|| stripped.to_string())
}

pub fn resolve(
    env_override: Option<&str>,
    exact_tag: Option<&str>,
    short_hash: Option<&str>,
) -> String {
    env_override
        .and_then(normalize)
        .or_else(|| exact_tag.and_then(normalize))
        .or_else(|| {
            short_hash
                .map(str::trim)
                .filter(|h| !h.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| FALLBACK_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins_over_everything() {
        assert_eq!(
            resolve(Some("v2.0.0"), Some("v1.0.0"), Some("abc1234")),
            "2.0.0"
        );
    }

    #[test]
    fn blank_override_is_ignored() {
        assert_eq!(
            resolve(Some("  "), Some("v1.4.0"), Some("abc1234")),
            "1.4.0"
        );
    }

    #[test]
    fn tag_beats_hash_and_loses_its_v_prefix() {
        assert_eq!(resolve(None, Some("v1.4.0\n"), Some("abc1234")), "1.4.0");
        assert_eq!(resolve(None, Some("1.4.0-beta.1"), None), "1.4.0-beta.1");
    }

    #[test]
    fn hash_is_used_when_head_is_not_tagged() {
        assert_eq!(resolve(None, None, Some("abc1234\n")), "abc1234");
    }

    #[test]
    fn falls_back_to_dev_outside_git() {
        assert_eq!(resolve(None, None, None), "dev");
        assert_eq!(resolve(None, Some(""), Some("")), "dev");
    }

    #[test]
    fn only_a_v_before_a_digit_is_stripped() {
        assert_eq!(normalize("v1.0.0").as_deref(), Some("1.0.0"));
        assert_eq!(normalize("V1.0.0").as_deref(), Some("1.0.0"));
        assert_eq!(normalize("vnext").as_deref(), Some("vnext"));
        assert_eq!(normalize("v").as_deref(), Some("v"));
        assert_eq!(normalize(""), None);
    }
}
