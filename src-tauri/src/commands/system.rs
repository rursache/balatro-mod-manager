use bmm_lib::finder::{is_balatro_running, is_steam_running};

#[tauri::command]
pub async fn check_steam_running() -> bool {
    is_steam_running()
}

#[tauri::command]
pub async fn check_balatro_running() -> bool {
    is_balatro_running()
}

#[tauri::command]
pub async fn get_app_version() -> String {
    crate::APP_VERSION.to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_version_is_resolved_at_build_time() {
        let version = crate::APP_VERSION;
        assert!(!version.trim().is_empty());
        assert_eq!(version, version.trim());
        assert!(
            !version.starts_with(['v', 'V'])
                || !version[1..].starts_with(|c: char| c.is_ascii_digit()),
            "tag prefix should be stripped: {version}"
        );
    }

    #[test]
    fn bundle_version_stays_static_semver() {
        let version = env!("CARGO_PKG_VERSION");
        let core = version.split(['-', '+']).next().unwrap_or_default();
        let parts: Vec<&str> = core.split('.').collect();
        assert_eq!(parts.len(), 3, "bundle version must be major.minor.patch");
        assert!(
            parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        );
    }
}
