use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Serialize)]
pub struct Payload {
    pub args: Vec<String>,
    pub cwd: String,
}

/// Event payload for installed-mods-changed event.
/// Contains delta information about what changed.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ModsChangedEvent {
    /// Mod names that were added
    pub added: Vec<String>,
    /// Mod names that were removed
    pub removed: Vec<String>,
    /// Whether this is a full refresh (added/removed may be incomplete)
    pub full_refresh: bool,
}

/// A mod's `meta.json` from the mod index
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ModMeta {
    #[serde(rename = "requires-steamodded", alias = "requires_steamodded", default)]
    pub requires_steamodded: bool,
    #[serde(rename = "requires-talisman", alias = "requires_talisman", default)]
    pub requires_talisman: bool,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "downloadURL")]
    pub download_url: Option<String>,
    #[serde(rename = "folderName", default)]
    pub folder_name: String,
    #[serde(default, deserialize_with = "string_or_number")]
    pub version: String,
    #[serde(rename = "automatic-version-check", default)]
    pub automatic_version_check: bool,
    #[serde(rename = "last-updated", default)]
    pub last_updated: u64,
}

/// One catalog entry as consumed by the frontend
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchiveModItem {
    pub dir_name: String,
    pub meta: ModMeta,
    pub description: String,
    pub image_url: String,
    #[serde(default)]
    pub has_thumbnail: bool,
}

fn string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mod_meta_accepts_numeric_version_and_ignores_unknown_keys() {
        let meta: ModMeta = serde_json::from_value(json!({
            "title": "X",
            "version": 2,
            "id": "ignored",
            "downloadURL": "https://example.com/x.zip"
        }))
        .expect("parse");
        assert_eq!(meta.version, "2");
        assert_eq!(
            meta.download_url.as_deref(),
            Some("https://example.com/x.zip")
        );
        assert!(!meta.requires_steamodded);
    }

    #[test]
    fn mod_meta_serializes_with_index_field_names() {
        let meta = ModMeta {
            title: "X".into(),
            last_updated: 5,
            ..Default::default()
        };
        let value = serde_json::to_value(&meta).expect("serialize");
        assert_eq!(value["last-updated"], 5);
        assert!(value.get("requires-steamodded").is_some());
        assert!(value.get("downloadURL").is_some());
    }
}
