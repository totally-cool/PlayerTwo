//! The account-switch engine.
//!
//! This is the OS-independent core. It expresses the switch as a sequence of
//! steps and delegates every privileged/OS-specific action to a [`Host`].
//!
//! Switch algorithm:
//! 1. Kill the platform's running processes.
//! 2. Detect who is logged in now; if identifiable and not the target, save them.
//! 3. Clear the live login (delete login files + unique-id marker).
//! 4. Restore the target account's saved login files + registry values.
//! 5. Optionally relaunch the platform.

use super::model::{Account, ExeLocator, LoginArtifact, PlatformDef, UniqueId};
use super::store::{RegistrySnapshot, Store};
use crate::os::Host;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct Engine {
    host: Box<dyn Host>,
    store: Store,
}

/// Outcome of a switch, surfaced to the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SwitchOutcome {
    pub switched: bool,
    pub already_active: bool,
    pub launched: bool,
    pub message: String,
}

impl Engine {
    pub fn new(host: Box<dyn Host>, store: Store) -> Self {
        Engine { host, store }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Expand `%VAR%` placeholders using the active host (for callers that need
    /// a concrete path, e.g. minting a generated-id marker file).
    pub fn expand_vars(&self, input: &str) -> String {
        self.host.expand_vars(input)
    }

    /// Read the currently logged-in account's unique id, if one can be determined.
    pub fn current_id(&self, plat: &PlatformDef) -> Result<Option<String>> {
        match &plat.unique_id {
            UniqueId::Registry { key, value } => self.host.read_registry(key, value),
            UniqueId::FileRegex { file, regex: pattern } => {
                let path = self.host.expand_vars(file);
                let Ok(text) = std::fs::read_to_string(&path) else {
                    return Ok(None);
                };
                let re = regex::Regex::new(pattern).context("invalid unique_id regex")?;
                Ok(re
                    .captures(&text)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_string()))
            }
            UniqueId::JsonField { file, pointer } => {
                let path = self.host.expand_vars(file);
                let Ok(text) = std::fs::read_to_string(&path) else {
                    return Ok(None);
                };
                let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                Ok(json
                    .pointer(pointer)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()))
            }
            UniqueId::GeneratedFile { file } => {
                let path = self.host.expand_vars(file);
                Ok(std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string()))
            }
        }
    }

    /// Perform the switch. `account_id` is the saved account to make active.
    pub fn switch(
        &self,
        plat: &PlatformDef,
        account_id: &str,
        auto_start: bool,
    ) -> Result<SwitchOutcome> {
        tracing::info!(platform = %plat.id, account = account_id, "switch start");
        // 1. Stop the platform and wait for it to fully exit — avoids file locks
        //    and the relaunch tripping the app's single-instance lock.
        self.host.kill_processes(&plat.exes_to_end)?;
        self.wait_for_exit(&plat.exes_to_end);

        // 2. Save whoever is logged in now (if we can tell, and it isn't the target).
        if let Some(current) = self.current_id(plat)? {
            if current == account_id {
                let launched = self.maybe_launch(plat, auto_start)?;
                return Ok(SwitchOutcome {
                    switched: false,
                    already_active: true,
                    launched,
                    message: "Account is already active".into(),
                });
            }
            // Only auto-save if we already track this account (avoids capturing
            // a stranger). New accounts are added explicitly via `add_current`.
            if self.store.list_accounts(&plat.id)?.iter().any(|a| a.id == current) {
                self.capture_login(plat, &current)?;
            }
        }

        // 3. Clear the live login.
        self.clear_login(plat)?;

        // 4. Restore the target.
        self.restore_login(plat, account_id)?;

        // 5. Relaunch.
        let launched = self.maybe_launch(plat, auto_start)?;
        Ok(SwitchOutcome {
            switched: true,
            already_active: false,
            launched,
            message: "Switched".into(),
        })
    }

    /// Clear the current login and launch the platform so the user can sign into
    /// a NEW account. The currently-active account is saved first if it's tracked,
    /// so nothing is lost.
    pub fn begin_new_login(&self, plat: &PlatformDef, auto_start: bool) -> Result<bool> {
        self.host.kill_processes(&plat.exes_to_end)?;
        self.wait_for_exit(&plat.exes_to_end);
        if let Some(current) = self.current_id(plat)? {
            if self.store.list_accounts(&plat.id)?.iter().any(|a| a.id == current) {
                self.capture_login(plat, &current)?;
            }
        }
        self.clear_login(plat)?;
        self.maybe_launch(plat, auto_start)
    }

    /// Launch the platform's app (public wrapper for the Steam "add account" flow).
    pub fn launch(&self, plat: &PlatformDef, auto_start: bool) -> Result<bool> {
        self.maybe_launch(plat, auto_start)
    }

    // ---- Steam-specific path (see switcher::steam) ----

    pub fn steam_accounts(&self) -> Vec<Account> {
        crate::switcher::steam::list_accounts(&*self.host)
    }

    pub fn steam_current(&self) -> Option<String> {
        crate::switcher::steam::current(&*self.host)
    }

    pub fn switch_steam(
        &self,
        plat: &PlatformDef,
        steamid: &str,
        auto_start: bool,
    ) -> Result<SwitchOutcome> {
        self.host.kill_processes(&plat.exes_to_end)?;
        self.wait_for_exit(&plat.exes_to_end);
        crate::switcher::steam::switch(&*self.host, steamid)?;
        let launched = self.maybe_launch(plat, auto_start)?;
        Ok(SwitchOutcome {
            switched: true,
            already_active: false,
            launched,
            message: "Switched".into(),
        })
    }

    // ---- Epic-specific path (see switcher::epic) ----

    pub fn epic_current(&self) -> Option<String> {
        crate::switcher::epic::current_id(&*self.host)
    }

    pub fn epic_username(&self) -> Option<String> {
        crate::switcher::epic::username_from_logs(&*self.host)
    }

    pub fn capture_epic(&self, account_id: &str) -> Result<()> {
        crate::switcher::epic::capture(&*self.host, &self.store, account_id)
    }

    pub fn switch_epic(
        &self,
        plat: &PlatformDef,
        account_id: &str,
        auto_start: bool,
    ) -> Result<SwitchOutcome> {
        self.host.kill_processes(&plat.exes_to_end)?;
        self.wait_for_exit(&plat.exes_to_end);
        crate::switcher::epic::switch(&*self.host, &self.store, account_id)?;
        let launched = self.maybe_launch(plat, auto_start)?;
        Ok(SwitchOutcome {
            switched: true,
            already_active: false,
            launched,
            message: "Switched".into(),
        })
    }

    /// Clear Epic's live login and open the launcher for a fresh sign-in.
    pub fn epic_new_login(&self, plat: &PlatformDef, auto_start: bool) -> Result<bool> {
        self.host.kill_processes(&plat.exes_to_end)?;
        self.wait_for_exit(&plat.exes_to_end);
        // Save the current account first if it's tracked, so its token isn't lost.
        if let Some(id) = self.epic_current() {
            if self.store.list_accounts("epic")?.iter().any(|a| a.id == id) {
                let _ = self.capture_epic(&id);
            }
        }
        crate::switcher::epic::clear(&*self.host)?;
        self.maybe_launch(plat, auto_start)
    }

    /// Refresh the saved token for whichever Epic account is currently active
    /// (Epic rotates RememberMe tokens, so a stored one can go stale).
    pub fn epic_renew(&self) -> Result<()> {
        if let Some(id) = self.epic_current() {
            if self.store.list_accounts("epic")?.iter().any(|a| a.id == id) {
                let _ = self.capture_epic(&id);
            }
        }
        Ok(())
    }

    /// Poll until no process in `exe_names` remains, up to ~5 seconds.
    fn wait_for_exit(&self, exe_names: &[String]) {
        if exe_names.is_empty() {
            return;
        }
        for _ in 0..50 {
            if !self.host.are_running(exe_names) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Capture the *current* live login into the store under `account_id`.
    /// Used both during a switch and by the explicit "add current account" action.
    pub fn capture_login(&self, plat: &PlatformDef, account_id: &str) -> Result<()> {
        let dir = self.store.account_dir(&plat.id, account_id);
        std::fs::create_dir_all(&dir)?;
        let mut registry = RegistrySnapshot::new();

        for artifact in &plat.login {
            match artifact {
                LoginArtifact::File { live, saved } => {
                    let live = self.host.expand_vars(live);
                    copy_into_saved(Path::new(&live), &dir.join(saved))?;
                }
                LoginArtifact::Registry { key, value, saved } => {
                    if let Some(data) = self.host.read_registry(key, value)? {
                        registry.insert(saved.clone(), data);
                    }
                }
            }
        }
        self.store.save_registry_snapshot(&plat.id, account_id, &registry)?;
        Ok(())
    }

    /// Restore a saved account's files + registry values to the live locations.
    fn restore_login(&self, plat: &PlatformDef, account_id: &str) -> Result<()> {
        let dir = self.store.account_dir(&plat.id, account_id);
        for artifact in &plat.login {
            match artifact {
                LoginArtifact::File { live, saved } => {
                    let live = self.host.expand_vars(live);
                    copy_from_saved(&dir.join(saved), Path::new(&live))?;
                }
                LoginArtifact::Registry { key, value, saved } => {
                    let snapshot = self.store.load_registry_snapshot(&plat.id, account_id)?;
                    if let Some(data) = snapshot.get(saved) {
                        self.host.write_registry(key, value, data)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Delete the live login so the platform sees "logged out".
    fn clear_login(&self, plat: &PlatformDef) -> Result<()> {
        for artifact in &plat.login {
            match artifact {
                LoginArtifact::File { live, .. } => {
                    delete_live(&self.host.expand_vars(live))?;
                }
                LoginArtifact::Registry { key, value, .. } => {
                    self.host.delete_registry_value(key, value)?;
                }
            }
        }
        for extra in &plat.clear {
            delete_live(&self.host.expand_vars(extra))?;
        }
        // A generated-id marker must be removed so the next login is treated as fresh.
        if let UniqueId::GeneratedFile { file } = &plat.unique_id {
            let _ = std::fs::remove_file(self.host.expand_vars(file));
        }
        Ok(())
    }

    fn maybe_launch(&self, plat: &PlatformDef, auto_start: bool) -> Result<bool> {
        if !auto_start {
            return Ok(false);
        }
        let Some(exe) = self.resolve_exe(plat) else {
            tracing::debug!(platform = %plat.id, "no launcher resolved; not launching");
            return Ok(false);
        };
        let args = plat.exe_args.clone().unwrap_or_default();
        tracing::debug!(platform = %plat.id, exe = %exe, args = %args, "launching");
        self.host.launch(&exe, &args, false)?;
        Ok(true)
    }

    /// Heuristic: is this platform installed / has it been used on this machine?
    /// True if its launcher exe resolves, or any login file's folder / registry
    /// value already exists.
    pub fn is_installed(&self, plat: &PlatformDef) -> bool {
        if self
            .resolve_exe(plat)
            .map(|p| Path::new(&p).exists())
            .unwrap_or(false)
        {
            return true;
        }
        // A registered URL-protocol scheme implies the app is installed — covers
        // UWP/Store apps (e.g. Xbox) whose exe lives in ACL-locked WindowsApps and
        // whose protocol uses package activation rather than shell\open\command.
        for loc in &plat.exe_locators {
            if let ExeLocator::UrlProtocol { scheme } = loc {
                if self
                    .host
                    .read_registry(&format!("HKCR\\{scheme}"), "")
                    .ok()
                    .flatten()
                    .is_some()
                {
                    return true;
                }
            }
        }
        plat.login.iter().any(|artifact| match artifact {
            LoginArtifact::File { live, .. } => {
                let p = self.host.expand_vars(live);
                Path::new(&p)
                    .parent()
                    .map(|d| d.exists())
                    .unwrap_or(false)
            }
            LoginArtifact::Registry { key, value, .. } => {
                self.host.read_registry(key, value).ok().flatten().is_some()
            }
        })
    }

    /// Resolve the launcher path: try each locator in order (preferring one that
    /// resolves to an existing file), then fall back to `exe_default`.
    fn resolve_exe(&self, plat: &PlatformDef) -> Option<String> {
        for loc in &plat.exe_locators {
            if let Some(path) = self.locate(loc) {
                if Path::new(&path).exists() {
                    return Some(path);
                }
            }
        }
        // Last resort: the literal default (returned even if missing, so a
        // failed launch names a concrete path in its error).
        plat.exe_default.as_ref().map(|d| self.host.expand_vars(d))
    }

    fn locate(&self, loc: &ExeLocator) -> Option<String> {
        match loc {
            ExeLocator::Path { path } => Some(self.host.expand_vars(path)),
            ExeLocator::Registry { key, value, suffix } => {
                let base = self.host.read_registry(key, value).ok().flatten()?;
                Some(match suffix {
                    Some(s) => format!("{base}{s}"),
                    None => base,
                })
            }
            ExeLocator::AppPaths { exe } => {
                const ROOTS: [&str; 2] = [
                    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\",
                    "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths\\",
                ];
                for root in ROOTS {
                    if let Ok(Some(v)) = self.host.read_registry(&format!("{root}{exe}"), "") {
                        return Some(strip_quotes(&v));
                    }
                }
                None
            }
            ExeLocator::UrlProtocol { scheme } => {
                let key = format!("HKCR\\{scheme}\\shell\\open\\command");
                let cmd = self.host.read_registry(&key, "").ok().flatten()?;
                Some(parse_command_exe(&cmd))
            }
        }
    }
}

/// Strip surrounding double-quotes from a registry path string.
fn strip_quotes(s: &str) -> String {
    s.trim().trim_matches('"').to_string()
}

/// Extract the exe path from a shell open command like `"C:\..\X.exe" %1`.
fn parse_command_exe(cmd: &str) -> String {
    let c = cmd.trim();
    if let Some(rest) = c.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return rest[..end].to_string();
        }
    }
    // Unquoted: drop a trailing " %1" / " %*" argument token.
    c.split(" %").next().unwrap_or(c).trim().to_string()
}

// ---- file helpers -------------------------------------------------------

/// Copy a live path (possibly a glob or directory) into the saved location.
fn copy_into_saved(live: &Path, saved: &Path) -> Result<()> {
    let live_str = live.to_string_lossy();
    if live_str.contains('*') {
        // Wildcard: copy each match into `saved/` keyed by file name.
        std::fs::create_dir_all(saved)?;
        for entry in glob::glob(&live_str)?.flatten() {
            if entry.is_file() {
                if let Some(name) = entry.file_name() {
                    std::fs::copy(&entry, saved.join(name))?;
                }
            }
        }
    } else if live.is_dir() {
        copy_dir_recursive(live, saved)?;
    } else if live.is_file() {
        if let Some(parent) = saved.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(live, saved)?;
    }
    Ok(())
}

/// Copy a saved path back to its live location.
fn copy_from_saved(saved: &Path, live: &Path) -> Result<()> {
    if !saved.exists() {
        return Ok(()); // nothing was captured for this artifact
    }
    if saved.is_dir() {
        copy_dir_recursive(saved, live)?;
    } else {
        if let Some(parent) = live.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(saved, live)?;
    }
    Ok(())
}

fn delete_live(path: &str) -> Result<()> {
    if path.contains('*') {
        for entry in glob::glob(path)?.flatten() {
            let _ = if entry.is_dir() {
                std::fs::remove_dir_all(&entry)
            } else {
                std::fs::remove_file(&entry)
            };
        }
        return Ok(());
    }
    let p = PathBuf::from(path);
    if p.is_dir() {
        let _ = std::fs::remove_dir_all(&p);
    } else if p.is_file() {
        let _ = std::fs::remove_file(&p);
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
