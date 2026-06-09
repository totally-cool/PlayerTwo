//! Built-in platform definitions.
//!
//! All platforms live in `builtin.json` (a JSON array of `PlatformDef`),
//! embedded at compile time. Adding a platform = adding an entry there.
//!
//! These definitions are ported from the GPL-3.0 TcNo Account Switcher
//! (https://github.com/TCNOco/TcNo-Acc-Switcher) and adapted to this schema.
//! The example paths are from general public knowledge and should be verified
//! against a real install before relying on them.

use crate::switcher::model::PlatformDef;

/// Parse all embedded platform definitions.
pub fn builtin() -> Vec<PlatformDef> {
    match serde_json::from_str::<Vec<PlatformDef>>(include_str!("builtin.json")) {
        Ok(defs) => defs,
        Err(e) => {
            eprintln!("malformed builtin.json: {e}");
            Vec::new()
        }
    }
}

/// Look up one platform definition by id.
pub fn by_id(id: &str) -> Option<PlatformDef> {
    builtin().into_iter().find(|p| p.id == id)
}
