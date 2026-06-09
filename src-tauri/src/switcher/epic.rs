//! Epic Games-specific account switching.
//!
//! Epic stores the auto-login token in `GameUserSettings.ini` under
//! `[RememberMe] Enable=True Data=<token>`. Rather than swapping the whole file,
//! we save/write just that token per account. The current account is identified
//! by the newest file in Epic's `Saved\Data` folder, and a human-readable name
//! is best-effort parsed from Epic's logs.
//!
//! (Approach inspired by symonxdd/epic-switcher, reimplemented for this schema.)

use crate::os::Host;
use crate::switcher::store::Store;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

const REMEMBER_BLOCK: &str = "[RememberMe]\nEnable=True\nData=";
const PLATFORM: &str = "epic";

fn login_ini(host: &dyn Host) -> PathBuf {
    PathBuf::from(host.expand_vars(
        "%LocalAppData%\\EpicGamesLauncher\\Saved\\Config\\WindowsEditor\\GameUserSettings.ini",
    ))
}
fn data_dir(host: &dyn Host) -> PathBuf {
    PathBuf::from(host.expand_vars("%LocalAppData%\\EpicGamesLauncher\\Saved\\Data"))
}
fn logs_dir(host: &dyn Host) -> PathBuf {
    PathBuf::from(host.expand_vars("%LocalAppData%\\EpicGamesLauncher\\Saved\\Logs"))
}

/// Extract a `Data=<token>` value from INI text, if it looks like a real token.
fn extract_token(text: &str, min_len: usize) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("Data=") {
            let token = rest.trim().to_string();
            if token.len() >= min_len {
                return Some(token);
            }
        }
    }
    None
}

/// The current live RememberMe token, if a valid one is present.
pub fn current_token(host: &dyn Host) -> Option<String> {
    let text = std::fs::read_to_string(login_ini(host)).ok()?;
    extract_token(&text, 1000)
}

/// The current account's id: the newest file in Epic's Data folder (name without
/// extension, minus an optional `OC_` prefix).
pub fn current_id(host: &dyn Host) -> Option<String> {
    let mut newest: Option<(std::time::SystemTime, String)> = None;
    for entry in std::fs::read_dir(data_dir(host)).ok()?.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            continue;
        }
        let Ok(modified) = meta.modified() else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
            newest = Some((modified, name));
        }
    }
    let (_, fname) = newest?;
    let stem = Path::new(&fname)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or(fname);
    Some(stem.strip_prefix("OC_").unwrap_or(&stem).to_string())
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
    std::fs::write(dir.join("epic_token.txt"), token).context("write Epic token")?;
    tracing::debug!(account = account_id, "captured Epic RememberMe token");
    Ok(())
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
    std::fs::read_to_string(migrated).ok().and_then(|t| extract_token(&t, 100))
}

/// Write the RememberMe token for `account_id` into Epic's live session file.
pub fn switch(host: &dyn Host, store: &Store, account_id: &str) -> Result<()> {
    let token = saved_token(store, account_id)
        .ok_or_else(|| anyhow!("no saved token for this account — re-import it"))?;
    let ini = login_ini(host);
    if let Some(parent) = ini.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&ini, format!("{REMEMBER_BLOCK}{token}\n")).context("write GameUserSettings.ini")?;
    tracing::info!(account = account_id, "wrote Epic login token");
    Ok(())
}

/// Clear the live login so the launcher shows a fresh sign-in.
pub fn clear(host: &dyn Host) -> Result<()> {
    let ini = login_ini(host);
    if ini.exists() {
        std::fs::write(&ini, "").context("clear GameUserSettings.ini")?;
    }
    Ok(())
}
