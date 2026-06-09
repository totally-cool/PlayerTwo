//! On-disk store of saved accounts.
//!
//! Layout (under the app data dir, e.g. `%AppData%/PlayerTwo`):
//! ```text
//! accounts/<platform>/accounts.json        ordered list of Account metadata
//! accounts/<platform>/<account_id>/...      saved login files for that account
//! accounts/<platform>/<account_id>/registry.json   saved registry values
//! ```
//! The data dir is configurable, which is what makes "store on a NAS" trivial.

use super::model::Account;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Saved registry values for one account: `"key:value"` -> data.
pub type RegistrySnapshot = BTreeMap<String, String>;

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

    pub fn list_accounts(&self, platform: &str) -> Result<Vec<Account>> {
        let path = self.accounts_file(platform);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        Ok(serde_json::from_str(&text).unwrap_or_default())
    }

    pub fn save_accounts(&self, platform: &str, accounts: &[Account]) -> Result<()> {
        let dir = self.platform_dir(platform);
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(accounts)?;
        std::fs::write(self.accounts_file(platform), text)?;
        Ok(())
    }

    /// Insert or update an account's metadata, preserving order.
    pub fn upsert_account(&self, platform: &str, account: Account) -> Result<()> {
        let mut accounts = self.list_accounts(platform)?;
        match accounts.iter_mut().find(|a| a.id == account.id) {
            Some(existing) => *existing = account,
            None => accounts.push(account),
        }
        self.save_accounts(platform, &accounts)
    }

    /// Remove an account's metadata and delete its saved files.
    pub fn remove_account(&self, platform: &str, account_id: &str) -> Result<()> {
        let mut accounts = self.list_accounts(platform)?;
        accounts.retain(|a| a.id != account_id);
        self.save_accounts(platform, &accounts)?;
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
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text).unwrap_or_default())
    }

    pub fn save_registry_snapshot(
        &self,
        platform: &str,
        account_id: &str,
        snapshot: &RegistrySnapshot,
    ) -> Result<()> {
        let dir = self.account_dir(platform, account_id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join("registry.json"),
            serde_json::to_string_pretty(snapshot)?,
        )?;
        Ok(())
    }
}

/// Make an arbitrary id safe to use as a folder name.
fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect()
}
