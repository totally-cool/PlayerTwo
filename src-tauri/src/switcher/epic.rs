//! Epic Games-specific account switching.
//!
//! Epic stores the auto-login token in `GameUserSettings.ini` under
//! `[RememberMe] Enable=True Data=<token>`, and rotates that token every launcher
//! session. Rather than swapping the whole file (which would destroy the file's
//! other settings), we surgically edit just the `[RememberMe]` section and keep a
//! per-account copy of the token.
//!
//! The active account is identified by Epic's own `AccountId` registry value
//! (the same value the platform def uses as its unique id) — a stable GUID —
//! rather than by guessing from file modification times, which could misattribute
//! a captured token to the wrong account.
//!
//! (Approach inspired by symonxdd/epic-switcher, reimplemented for this schema.)

use crate::os::Host;
use crate::switcher::store::{atomic_write, Store};
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

const PLATFORM: &str = "epic";

/// Epic's own identifier for the signed-in account (a stable GUID). This is the
/// same key/value the platform def uses as its `unique_id`.
const IDENTIFIERS_KEY: &str = "HKCU\\Software\\Epic Games\\Unreal Engine\\Identifiers";
const ACCOUNT_ID_VALUE: &str = "AccountId";

/// Minimum plausible length of a real RememberMe `Data=` token. Epic's token is a
/// long encrypted blob; anything shorter is a placeholder/blank/flag and must not
/// be captured as a login. Used consistently for both live capture and reading a
/// migrated whole-file snapshot. Erring high is safe: a rejected token aborts a
/// capture (recoverable), whereas accepting junk would store a broken login.
const MIN_TOKEN_LEN: usize = 500;

fn login_ini(host: &dyn Host) -> PathBuf {
    PathBuf::from(host.expand_vars(
        "%LocalAppData%\\EpicGamesLauncher\\Saved\\Config\\WindowsEditor\\GameUserSettings.ini",
    ))
}
fn logs_dir(host: &dyn Host) -> PathBuf {
    PathBuf::from(host.expand_vars("%LocalAppData%\\EpicGamesLauncher\\Saved\\Logs"))
}

/// Extract a `Data=<token>` value from INI text, if it looks like a real token.
fn extract_token(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("Data=") {
            let token = rest.trim().to_string();
            if token.len() >= MIN_TOKEN_LEN {
                return Some(token);
            }
        }
    }
    None
}

/// The current live RememberMe token, if a valid one is present.
pub fn current_token(host: &dyn Host) -> Option<String> {
    let text = std::fs::read_to_string(login_ini(host)).ok()?;
    extract_token(&text)
}

/// The current account's id: Epic's `AccountId` registry GUID. Returns `None`
/// (rather than a guess) if it can't be read confidently, so we never misattribute
/// a captured token to the wrong account.
pub fn current_id(host: &dyn Host) -> Option<String> {
    host.read_registry(IDENTIFIERS_KEY, ACCOUNT_ID_VALUE)
        .ok()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Best-effort human-readable account name from recent Epic logs. Returns None
/// if nothing recognizable is found (caller should fall back to a given name).
pub fn username_from_logs(host: &dyn Host) -> Option<String> {
    let re = regex::Regex::new(r#"(?:DisplayName|epicUserName|AccountName)["']?\s*[:=]\s*["']?([^"'\r\n,}]+)"#).ok()?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(logs_dir(host))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "log").unwrap_or(false))
        .collect();
    // newest first
    files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    files.reverse();
    for path in files.into_iter().take(3) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(c) = re.captures(&text) {
                let name = c.get(1)?.as_str().trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

fn token_file(store: &Store, account_id: &str) -> PathBuf {
    store.account_dir(PLATFORM, account_id).join("epic_token.txt")
}

/// Save the currently-active account's token under `account_id`.
pub fn capture(host: &dyn Host, store: &Store, account_id: &str) -> Result<()> {
    let token = current_token(host).ok_or_else(|| anyhow!("no valid Epic login token found"))?;
    let dir = store.account_dir(PLATFORM, account_id);
    std::fs::create_dir_all(&dir)?;
    atomic_write(&token_file(store, account_id), token.as_bytes()).context("write Epic token")?;
    tracing::debug!(account = account_id, "captured Epic RememberMe token");
    Ok(())
}

/// When the saved token for `account_id` was last written (unix seconds), if any.
/// Used by the UI to show login-freshness and offer a refresh.
pub fn token_saved_at(store: &Store, account_id: &str) -> Option<u64> {
    let meta = std::fs::metadata(token_file(store, account_id)).ok()?;
    let modified = meta.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// The saved token for an account: prefer `epic_token.txt`, else extract from a
/// migrated whole-file `GameUserSettings.ini` snapshot.
fn saved_token(store: &Store, account_id: &str) -> Option<String> {
    if let Ok(t) = std::fs::read_to_string(token_file(store, account_id)) {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    let migrated = store
        .account_dir(PLATFORM, account_id)
        .join("GameUserSettings.ini");
    std::fs::read_to_string(migrated).ok().and_then(|t| extract_token(&t))
}

/// Surgically set the `[RememberMe]` section of an INI, preserving every other
/// section, key, comment, and the file's existing line-ending style. Only the
/// `Enable=` and `Data=` lines inside `[RememberMe]` are rewritten; the section
/// is appended if it's absent.
fn edit_remember_me(existing: &str, enable: bool, token: &str) -> String {
    let newline = if existing.contains("\r\n") { "\r\n" } else { "\n" };
    let enable_line = format!("Enable={}", if enable { "True" } else { "False" });
    let data_line = format!("Data={token}");

    if existing.trim().is_empty() {
        return format!("[RememberMe]{newline}{enable_line}{newline}{data_line}{newline}");
    }

    let is_header = |l: &str| {
        let t = l.trim();
        t.starts_with('[') && t.ends_with(']')
    };

    // Strip a single trailing newline so `split('\n')` doesn't yield a spurious
    // empty final element; we restore the trailing newline on join.
    let ended_nl = existing.ends_with('\n');
    let body = if ended_nl { &existing[..existing.len() - 1] } else { existing };

    let mut out: Vec<String> = Vec::new();
    let mut in_section = false;
    let mut seen_section = false;
    let mut have_enable = false;
    let mut have_data = false;

    for raw in body.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if is_header(line) {
            // Leaving [RememberMe]: append any keys it was missing.
            if in_section {
                if !have_enable {
                    out.push(enable_line.clone());
                }
                if !have_data {
                    out.push(data_line.clone());
                }
            }
            in_section = line.trim().eq_ignore_ascii_case("[RememberMe]");
            if in_section {
                seen_section = true;
                have_enable = false;
                have_data = false;
            }
            out.push(line.to_string());
            continue;
        }
        if in_section {
            let t = line.trim_start();
            if t.len() >= 7 && t[..7].eq_ignore_ascii_case("Enable=") {
                out.push(enable_line.clone());
                have_enable = true;
                continue;
            }
            if t.len() >= 5 && t[..5].eq_ignore_ascii_case("Data=") {
                out.push(data_line.clone());
                have_data = true;
                continue;
            }
        }
        out.push(line.to_string());
    }
    // File ended while still inside [RememberMe]: flush missing keys.
    if in_section {
        if !have_enable {
            out.push(enable_line.clone());
        }
        if !have_data {
            out.push(data_line.clone());
        }
    }
    if !seen_section {
        out.push("[RememberMe]".to_string());
        out.push(enable_line);
        out.push(data_line);
    }

    let mut joined = out.join(newline);
    if ended_nl {
        joined.push_str(newline);
    }
    joined
}

/// Write the RememberMe token for `account_id` into Epic's live session file,
/// preserving the rest of `GameUserSettings.ini`.
pub fn switch(host: &dyn Host, store: &Store, account_id: &str) -> Result<()> {
    let token = saved_token(store, account_id)
        .ok_or_else(|| anyhow!("no saved token for this account — re-import it"))?;
    let ini = login_ini(host);
    let existing = std::fs::read_to_string(&ini).unwrap_or_default();
    let updated = edit_remember_me(&existing, true, &token);
    atomic_write(&ini, updated.as_bytes()).context("write GameUserSettings.ini")?;
    tracing::info!(account = account_id, "wrote Epic login token");
    Ok(())
}

/// Clear the live login so the launcher shows a fresh sign-in — surgically, so
/// only the RememberMe token is removed and other settings are preserved.
pub fn clear(host: &dyn Host) -> Result<()> {
    let ini = login_ini(host);
    if !ini.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(&ini).unwrap_or_default();
    let cleared = edit_remember_me(&existing, false, "");
    atomic_write(&ini, cleared.as_bytes()).context("clear GameUserSettings.ini")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::edit_remember_me;

    #[test]
    fn preserves_other_sections_when_setting_token() {
        let existing = "[/Script/Foo.Bar]\nWindowMode=1\nResolution=1920x1080\n\n[RememberMe]\nEnable=True\nData=oldtoken\n";
        let out = edit_remember_me(existing, true, "NEWTOKEN");
        assert!(out.contains("[/Script/Foo.Bar]"));
        assert!(out.contains("WindowMode=1"));
        assert!(out.contains("Resolution=1920x1080"));
        assert!(out.contains("Data=NEWTOKEN"));
        assert!(!out.contains("oldtoken"));
        // exactly one RememberMe section, one Data line
        assert_eq!(out.matches("[RememberMe]").count(), 1);
        assert_eq!(out.matches("Data=").count(), 1);
    }

    #[test]
    fn creates_section_when_absent() {
        let existing = "[Other]\nKey=Val\n";
        let out = edit_remember_me(existing, true, "TT");
        assert!(out.contains("[Other]"));
        assert!(out.contains("Key=Val"));
        assert!(out.contains("[RememberMe]"));
        assert!(out.contains("Enable=True"));
        assert!(out.contains("Data=TT"));
    }

    #[test]
    fn clear_blanks_only_the_token() {
        let existing = "[Other]\nKey=Val\n[RememberMe]\nEnable=True\nData=secret\n";
        let out = edit_remember_me(existing, false, "");
        assert!(out.contains("[Other]"));
        assert!(out.contains("Key=Val"));
        assert!(out.contains("Enable=False"));
        assert!(out.contains("Data=\n") || out.ends_with("Data="));
        assert!(!out.contains("secret"));
    }

    #[test]
    fn preserves_crlf_line_endings() {
        let existing = "[RememberMe]\r\nEnable=True\r\nData=old\r\n";
        let out = edit_remember_me(existing, true, "NEW");
        assert!(out.contains("\r\n"));
        assert!(out.contains("Data=NEW"));
        assert!(!out.contains("Data=old"));
    }

    #[test]
    fn empty_input_produces_clean_section() {
        let out = edit_remember_me("", true, "TT");
        assert_eq!(out, "[RememberMe]\nEnable=True\nData=TT\n");
    }
}
