//! On-disk store of saved accounts.
//!
//! Layout (under the app data dir, e.g. `%AppData%/PlayerTwo`):
//! ```text
//! accounts/<platform>/accounts.json        versioned list of Account metadata
//! accounts/<platform>/<account_id>/...      saved login files for that account
//! accounts/<platform>/<account_id>/registry.json   saved registry values
//! .playertwo.lock                           cross-process mutation lock
//! ```
//! The data dir is configurable, which is what makes "store on a NAS" trivial.
//! Because the store may live on a shared SMB/NAS path used by more than one PC,
//! all writes are atomic (temp-file + rename) and mutations are serialized behind
//! a lockfile with stale-lock detection.

use super::model::Account;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Saved registry values for one account: `"key:value"` -> data.
pub type RegistrySnapshot = BTreeMap<String, String>;

/// Current on-disk schema version for `accounts.json`. Bump when the shape of
/// the persisted metadata changes so a future load can migrate deliberately.
const STORE_VERSION: u32 = 1;

/// A lock held longer than this is assumed abandoned (crashed process) and stolen.
const LOCK_STALE: Duration = Duration::from_secs(30);
/// How long to wait for a contended lock before giving up.
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// Versioned container written to `accounts.json`. The reader also accepts a
/// bare legacy array (pre-versioning), so existing stores keep working.
#[derive(serde::Serialize, serde::Deserialize)]
struct AccountsFile {
    version: u32,
    accounts: Vec<Account>,
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Store { root }
    }

    fn platform_dir(&self, platform: &str) -> PathBuf {
        self.root.join("accounts").join(platform)
    }

    /// Folder holding one account's saved login files.
    pub fn account_dir(&self, platform: &str, account_id: &str) -> PathBuf {
        self.platform_dir(platform).join(sanitize(account_id))
    }

    fn accounts_file(&self, platform: &str) -> PathBuf {
        self.platform_dir(platform).join("accounts.json")
    }

    /// Read the account list for a platform.
    ///
    /// A missing file is an empty list, but a *present but unparseable* file is a
    /// hard error rather than a silent empty list — otherwise a corrupt
    /// `accounts.json` would be treated as "no accounts" and the next save would
    /// wipe all of the user's saved metadata.
    pub fn list_accounts(&self, platform: &str) -> Result<Vec<Account>> {
        let path = self.accounts_file(platform);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        parse_accounts(&text)
            .with_context(|| format!("{} is corrupt — refusing to overwrite it", path.display()))
    }

    /// Write the account list (no lock — internal helper for holders of the lock).
    fn save_accounts_locked(&self, platform: &str, accounts: &[Account]) -> Result<()> {
        let dir = self.platform_dir(platform);
        std::fs::create_dir_all(&dir)?;
        let file = AccountsFile {
            version: STORE_VERSION,
            accounts: accounts.to_vec(),
        };
        let text = serde_json::to_string_pretty(&file)?;
        atomic_write(&self.accounts_file(platform), text.as_bytes())
    }

    /// Insert or update an account's metadata, preserving order.
    pub fn upsert_account(&self, platform: &str, account: Account) -> Result<()> {
        let _lock = self.lock()?;
        let mut accounts = self.list_accounts(platform)?;
        match accounts.iter_mut().find(|a| a.id == account.id) {
            Some(existing) => *existing = account,
            None => accounts.push(account),
        }
        self.save_accounts_locked(platform, &accounts)
    }

    /// Remove an account's metadata and delete its saved files.
    pub fn remove_account(&self, platform: &str, account_id: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut accounts = self.list_accounts(platform)?;
        accounts.retain(|a| a.id != account_id);
        self.save_accounts_locked(platform, &accounts)?;
        let dir = self.account_dir(platform, account_id);
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn load_registry_snapshot(&self, platform: &str, account_id: &str) -> Result<RegistrySnapshot> {
        let path = self.account_dir(platform, account_id).join("registry.json");
        if !path.exists() {
            return Ok(RegistrySnapshot::new());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("{} is corrupt", path.display()))
    }

    pub fn save_registry_snapshot(
        &self,
        platform: &str,
        account_id: &str,
        snapshot: &RegistrySnapshot,
    ) -> Result<()> {
        let _lock = self.lock()?;
        let dir = self.account_dir(platform, account_id);
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(snapshot)?;
        atomic_write(&dir.join("registry.json"), text.as_bytes())
    }

    fn last_used_file(&self, platform: &str) -> PathBuf {
        self.platform_dir(platform).join("last_used.json")
    }

    /// Most-recently-used timestamps (`account_id -> unix secs`) for a platform.
    /// Non-critical metadata: a missing or unreadable file is simply an empty map.
    pub fn load_last_used(&self, platform: &str) -> BTreeMap<String, u64> {
        std::fs::read_to_string(self.last_used_file(platform))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Record that `account_id` was just switched to.
    pub fn record_last_used(&self, platform: &str, account_id: &str, unix_secs: u64) -> Result<()> {
        let _lock = self.lock()?;
        let mut map = self.load_last_used(platform);
        map.insert(account_id.to_string(), unix_secs);
        std::fs::create_dir_all(self.platform_dir(platform))?;
        let text = serde_json::to_string_pretty(&map)?;
        atomic_write(&self.last_used_file(platform), text.as_bytes())
    }

    fn lock_path(&self) -> PathBuf {
        self.root.join(".playertwo.lock")
    }

    /// Take the store-wide mutation lock. Held via RAII until the guard drops.
    fn lock(&self) -> Result<LockGuard> {
        LockGuard::acquire(&self.root, self.lock_path())
    }
}

/// Parse an account list, accepting both the current versioned object form and
/// the legacy bare-array form. Genuine garbage returns an error.
fn parse_accounts(text: &str) -> Result<Vec<Account>> {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        bail!("empty accounts file");
    }
    if trimmed.starts_with('[') {
        // Legacy pre-versioning format: a bare JSON array of accounts.
        return Ok(serde_json::from_str::<Vec<Account>>(text)?);
    }
    Ok(serde_json::from_str::<AccountsFile>(text)?.accounts)
}

/// A best-effort process/host identifier for lock diagnostics.
fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

/// A held store lock; deletes the lockfile on drop.
struct LockGuard {
    path: PathBuf,
}

impl LockGuard {
    fn acquire(root: &Path, path: PathBuf) -> Result<LockGuard> {
        std::fs::create_dir_all(root)?;
        let start = SystemTime::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let _ = writeln!(f, "{} {}", std::process::id(), hostname());
                    return Ok(LockGuard { path });
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path) {
                        tracing::warn!(path = %path.display(), "stealing stale store lock");
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed().unwrap_or_default() > LOCK_TIMEOUT {
                        bail!("store is locked by another PlayerTwo instance; try again shortly");
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
                Err(e) => return Err(e).context("open store lock"),
            }
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Whether a lockfile looks abandoned (older than [`LOCK_STALE`], or unreadable).
fn lock_is_stale(path: &Path) -> bool {
    match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(modified) => modified.elapsed().map(|age| age > LOCK_STALE).unwrap_or(true),
        // Vanished between the failed create and this stat, or clock weirdness:
        // treat as stealable so we don't wedge forever.
        Err(_) => true,
    }
}

/// Monotonic counter making temp-file names unique within this process.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` atomically: write a sibling temp file, then rename it
/// over the destination. On a partial/interrupted write the original file is left
/// intact, which matters on NAS/SMB shares where writes can be interrupted.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(p) => {
            std::fs::create_dir_all(p)?;
            p.to_path_buf()
        }
        None => PathBuf::from("."),
    };
    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmp".into());
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{stem}.{}.{seq}.tmp", std::process::id()));

    std::fs::write(&tmp, bytes).with_context(|| format!("write temp {}", tmp.display()))?;
    // `fs::rename` replaces an existing destination atomically on the same volume.
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e).with_context(|| format!("atomic rename to {}", path.display()))
        }
    }
}

/// Make an arbitrary id safe to use as a folder name.
fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect()
}
