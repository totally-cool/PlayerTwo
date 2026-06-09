//! Data model for PlayerTwo.
//!
//! These types are a clean-room design describing *what* a platform login is,
//! independent of any operating system. The OS-specific behaviour lives behind
//! the [`crate::os::Host`] trait.

use serde::{Deserialize, Serialize};

/// How the *currently* logged-in account is uniquely identified for a platform.
///
/// During a switch we read this to know "who is logged in right now" so we can
/// save their files before swapping someone else in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UniqueId {
    /// Read a value from a registry key. `key` like `HKCU\\Software\\...`.
    Registry { key: String, value: String },
    /// Match a regex (one capture group) against a file's text contents.
    FileRegex { file: String, regex: String },
    /// Read a string field from a JSON file via a JSON pointer (RFC 6901).
    JsonField { file: String, pointer: String },
    /// No natural ID exists; we drop a generated marker file to fingerprint a login.
    GeneratedFile { file: String },
}

/// A single artifact that makes up a login: a file/glob on disk, or a registry value.
///
/// `saved` is the relative path/name the artifact is stored under inside an
/// account's saved folder, so it can be restored to `live` later.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginArtifact {
    /// A file or wildcard path. `live` may contain `%VARS%` and `*` globs.
    File { live: String, saved: String },
    /// A registry value copied verbatim.
    Registry {
        key: String,
        value: String,
        saved: String,
    },
}

/// Strategies for locating a platform's launcher executable, tried in order.
///
/// Registry / protocol lookups are resolved via the `Host`, so this stays
/// OS-neutral. The first strategy that resolves to an existing file wins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "via", rename_all = "snake_case")]
pub enum ExeLocator {
    /// A literal path (may contain `%VARS%`); used if the file exists.
    Path { path: String },
    /// Read the exe path (or an install dir) from a registry value.
    Registry {
        key: String,
        value: String,
        /// Appended to the value when it points at a dir, not the exe itself.
        #[serde(default)]
        suffix: Option<String>,
    },
    /// Windows "App Paths" lookup by exe file name, e.g. `"Discord.exe"`.
    AppPaths { exe: String },
    /// Parse a URL-protocol handler's open command (e.g. `"com.epicgames.launcher"`).
    UrlProtocol { scheme: String },
}

/// Declarative description of one switchable platform (Steam, Discord, ...).
///
/// This is the schema for the JSON files in `src/defs/`. It is intentionally
/// OS-neutral: `live` paths use `%VAR%` placeholders resolved by the `Host`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformDef {
    /// Stable slug, e.g. `"discord"`. Used as a folder name in the store.
    pub id: String,
    /// Human-readable name shown in the UI.
    pub name: String,
    /// Default executable to (re)launch after switching, if known. Used as a
    /// last resort if no `exe_locators` resolve.
    #[serde(default)]
    pub exe_default: Option<String>,
    /// Ordered strategies for locating the launcher exe (tried before `exe_default`).
    #[serde(default)]
    pub exe_locators: Vec<ExeLocator>,
    /// Extra args appended when launching.
    #[serde(default)]
    pub exe_args: Option<String>,
    /// Process image names to terminate before swapping files.
    #[serde(default)]
    pub exes_to_end: Vec<String>,
    /// The set of files/registry values that constitute a login.
    pub login: Vec<LoginArtifact>,
    /// Additional live paths to delete on logout (besides the login files above).
    #[serde(default)]
    pub clear: Vec<String>,
    /// How to read the currently logged-in account's identity.
    pub unique_id: UniqueId,
}

/// A saved account belonging to a platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// The unique-id value captured at save time.
    pub id: String,
    /// Editable display name.
    pub display_name: String,
    /// Optional user note shown under the name.
    #[serde(default)]
    pub note: Option<String>,
    /// Relative path to a profile image inside the account folder, if any.
    #[serde(default)]
    pub image: Option<String>,
}
