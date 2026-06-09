//! User settings, persisted to `<data>/settings.json`.
//!
//! Three scopes:
//! - program-wide (`auto_start`, `minimize_after_switch`)
//! - per-platform (`platforms[id]` — currently just enable/disable)
//! - per-account settings live on the `Account` itself (name, note)

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlatformSettings {
    /// Explicit enable/disable. `None` means "follow auto-detection".
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Launch the app silently after switching (Epic `-silent`). `None` = default.
    #[serde(default)]
    pub silent: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Launch the platform's app after switching.
    #[serde(default = "default_true")]
    pub auto_start: bool,
    /// Hide the switcher window after a successful switch.
    #[serde(default)]
    pub minimize_after_switch: bool,
    /// Verbose (debug-level) logging. Applied at startup.
    #[serde(default)]
    pub debug_logging: bool,
    /// Per-platform overrides, keyed by platform id.
    #[serde(default)]
    pub platforms: BTreeMap<String, PlatformSettings>,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_start: true,
            minimize_after_switch: false,
            debug_logging: false,
            platforms: BTreeMap::new(),
        }
    }
}

impl Settings {
    pub fn load(data_dir: &Path) -> Settings {
        std::fs::read_to_string(data_dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        std::fs::write(
            data_dir.join("settings.json"),
            serde_json::to_string_pretty(self)?,
        )?;
        Ok(())
    }

    /// Whether a platform should be shown/active: explicit setting if present,
    /// otherwise fall back to auto-detection.
    pub fn platform_enabled(&self, id: &str, detected: bool) -> bool {
        self.platforms
            .get(id)
            .and_then(|p| p.enabled)
            .unwrap_or(detected)
    }
}
