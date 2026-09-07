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
//! Those two sources — the token in the INI and the id in the registry — are
//! written by the launcher at *different* times, so they disagree whenever a
//! switch is applied but the launcher never actually signs in (a rejected/expired
//! token, or a launcher that was killed first). Capturing in that state files the
//! planted token under the previous account and silently corrupts both saved
//! logins. [`unconfirmed_switch`] closes that hole: every write records what was
//! planted, and a capture is only trusted once the launcher has confirmed the
//! sign-in.
//!
//! (Approach inspired by symonxdd/epic-switcher, reimplemented for this schema.)

use crate::os::Host;
use crate::switcher::store::{atomic_write, Store};
use anyhow::{anyhow, bail, Context, Result};
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

/// Where the pending-switch record lives. Deliberately machine-local rather than
/// in the store: the store may sit on a NAS shared by several PCs, but "which
/// token is currently planted in the INI" is a fact about *this* machine.
fn pending_path(host: &dyn Host) -> PathBuf {
    PathBuf::from(host.expand_vars("%LocalAppData%\\PlayerTwo\\epic_pending.json"))
}

/// A token PlayerTwo wrote into the live INI whose sign-in the launcher has not
/// confirmed yet.
#[derive(serde::Serialize, serde::Deserialize)]
struct PendingSwitch {
    /// The account the token was written for.
    account_id: String,
    /// Fingerprint of the exact token written, so we can tell our own planted
    /// token from a fresh one the launcher minted after a real sign-in.
    token_fp: String,
}

/// Stable fingerprint of a token (FNV-1a 64 plus length). Hand-rolled so it stays
/// identical across builds — `DefaultHasher` is explicitly not stable across
/// releases, and this value is persisted between runs. It only ever answers "is
/// this the same string?", so collision resistance is irrelevant; the worst a
/// collision could do is skip one capture.
fn fingerprint(token: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in token.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}.{}", token.len())
}

fn load_pending(host: &dyn Host) -> Option<PendingSwitch> {
    let text = std::fs::read_to_string(pending_path(host)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_pending(host: &dyn Host, pending: &PendingSwitch) -> Result<()> {
    let text = serde_json::to_string_pretty(pending)?;
    atomic_write(&pending_path(host), text.as_bytes())
}

fn clear_pending(host: &dyn Host) {
    let _ = std::fs::remove_file(pending_path(host));
}

/// The account whose token PlayerTwo planted in the live INI but which the
/// launcher never confirmed signing in as — in practice, an expired token Epic
/// rejected. `None` means the live login can be trusted to belong to whoever
/// [`current_id`] reports.
///
/// Reconciles as a side effect: once the launcher confirms the switch, the record
/// is dropped so later captures work normally again.
pub fn unconfirmed_switch(host: &dyn Host) -> Option<String> {
    let pending = load_pending(host)?;
    // The launcher flipped `AccountId` to our target: the sign-in took.
    if current_id(host).as_deref() == Some(pending.account_id.as_str()) {
        clear_pending(host);
        return None;
    }
    match current_token(host) {
        // A different token is live, so somebody signed in for real (by hand, or
        // the launcher rotated ours). The INI is trustworthy again.
        Some(live) if fingerprint(&live) != pending.token_fp => {
            clear_pending(host);
            None
        }
        // Our planted token is still sitting there untouched, or the launcher
        // wiped it back to a stub — either way it was never signed in with, and
        // `AccountId` still names somebody else.
        _ => Some(pending.account_id),
    }
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

/// Whether a real login token is present in the live INI. `AccountId` can name an
/// account the launcher has since signed out of, so "is someone actually logged
/// in" needs this as well as [`current_id`].
pub fn has_live_token(host: &dyn Host) -> bool {
    current_token(host).is_some()
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
    // Never file a token under an account the launcher hasn't actually signed in
    // as. Without this a rejected switch leaves account B's token in the INI while
    // the registry still says A, and the next capture saves B's token as A's.
    if let Some(planted) = unconfirmed_switch(host) {
        if planted != account_id {
            bail!(
                "the live Epic login is a token PlayerTwo wrote for another account and the \
                 launcher never signed in with it — sign in to Epic first, then save"
            );
        }
    }
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
    // Record what we planted. Until the launcher confirms this sign-in, the INI
    // and the registry describe different accounts and must not be paired up
    // (see `unconfirmed_switch`). Non-fatal: the write itself already succeeded.
    if let Err(e) = save_pending(
        host,
        &PendingSwitch {
            account_id: account_id.to_string(),
            token_fp: fingerprint(&token),
        },
    ) {
        tracing::warn!(error = %e, "could not record the pending Epic switch");
    }
    tracing::info!(account = account_id, "wrote Epic login token");
    Ok(())
}

/// Clear the live login so the launcher shows a fresh sign-in — surgically, so
/// only the RememberMe token is removed and other settings are preserved.
pub fn clear(host: &dyn Host) -> Result<()> {
    // Nothing is planted any more, so drop any pending record — the next sign-in
    // is a deliberate fresh one and should capture normally.
    clear_pending(host);
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
