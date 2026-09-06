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
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// How long to wait for a platform's processes to exit before aborting a switch,
/// when the platform definition doesn't specify its own `exit_timeout_secs`.
const DEFAULT_EXIT_TIMEOUT_SECS: u64 = 10;
/// How often to re-check whether the processes have exited.
const EXIT_POLL_INTERVAL_MS: u64 = 100;

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
        // 1. Stop the platform and wait for it to fully exit — avoids file locks,
        //    racing the launcher's shutdown writes, and the relaunch tripping the
        //    app's single-instance lock. Abort if it won't close.
        self.ensure_stopped(plat)?;

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
            // A failed capture here aborts the switch *before* we clear anything,
            // so a login is never lost to a silently-ignored capture error.
            if self.store.list_accounts(&plat.id)?.iter().any(|a| a.id == current) {
                self.capture_login(plat, &current)?;
            }
        }

        // 3. Snapshot the live login so a failed restore can be rolled back.
        let backup = self.backup_live(plat)?;

        // 4. Clear the live login.
        self.clear_login(plat)?;

        // 5. Restore the target. If this fails the user would otherwise be left
        //    logged out, so roll the previous login back into place.
        if let Err(e) = self.restore_login(plat, account_id) {
            tracing::error!(error = %e, "restore failed; rolling back to the previous login");
            if let Err(re) = self.restore_backup(plat, &backup) {
                return Err(e.context(format!(
                    "restore failed and rollback ALSO failed ({re}); the login may be inconsistent — re-import the account"
                )));
            }
            return Err(e.context("restore failed; rolled back to the previous login"));
        }
        // Success: `backup` is dropped here and its temp copy removed.

        // 6. Relaunch.
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
        self.ensure_stopped(plat)?;
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
        self.ensure_stopped(plat)?;
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

    /// The Epic account PlayerTwo last switched to whose sign-in the launcher
    /// never confirmed — i.e. its saved token was rejected. Drives the UI warning.
    pub fn epic_unconfirmed(&self) -> Option<String> {
        crate::switcher::epic::unconfirmed_switch(&*self.host)
    }

    /// Save `account_id`'s live token, unless the live login is an unconfirmed
    /// token PlayerTwo planted for somebody else — in which case there is nothing
    /// of this account's to save and capturing would corrupt its saved login.
    /// Returns whether a capture actually happened.
    fn capture_epic_if_trustworthy(&self, account_id: &str) -> Result<bool> {
        if let Some(planted) = self.epic_unconfirmed() {
            if planted != account_id {
                tracing::warn!(
                    account = %account_id,
                    planted = %planted,
                    "skipping Epic capture: the live login is an unconfirmed token written for \
                     another account (its saved token was most likely rejected as expired)"
                );
                return Ok(false);
            }
        }
        self.capture_epic(account_id)?;
        Ok(true)
    }

    /// When the Epic token for `account_id` was last saved (unix seconds).
    pub fn epic_token_saved_at(&self, account_id: &str) -> Option<u64> {
        crate::switcher::epic::token_saved_at(&self.store, account_id)
    }

    pub fn switch_epic(
        &self,
        plat: &PlatformDef,
        account_id: &str,
        auto_start: bool,
    ) -> Result<SwitchOutcome> {
        self.ensure_stopped(plat)?;
        let current = self.epic_current();

        // "Already active" needs a live token as well as a matching `AccountId`:
        // the registry keeps naming an account after the launcher has signed out
        // of it, and short-circuiting there would leave the user stuck at a
        // sign-in screen with no way to re-apply their saved login.
        if current.as_deref() == Some(account_id) && crate::switcher::epic::has_live_token(&*self.host) {
            let launched = self.maybe_launch(plat, auto_start)?;
            return Ok(SwitchOutcome {
                switched: false,
                already_active: true,
                launched,
                message: "Account is already active".into(),
            });
        }

        // Epic rotates the RememberMe token every launcher session. Save the
        // outgoing account's *fresh* token before overwriting the INI with the
        // target's, otherwise the stored token goes stale and switching back to
        // that account later fails. Aborting on capture failure is intentional:
        // we must not overwrite a login we failed to save.
        if let Some(current) = current.filter(|c| c != account_id) {
            if self.store.list_accounts("epic")?.iter().any(|a| a.id == current) {
                self.capture_epic_if_trustworthy(&current)?;
            }
        }
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
        self.ensure_stopped(plat)?;
        // Save the current account first if it's tracked, so its token isn't lost.
        // A failed capture aborts before we clear — clearing an un-saved login
        // would permanently lose it.
        if let Some(id) = self.epic_current() {
            if self.store.list_accounts("epic")?.iter().any(|a| a.id == id) {
                self.capture_epic(&id)?;
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
                // Best-effort: renewal is a safe-to-skip refresh (no clear follows),
                // so a failure here is logged, not fatal. The trustworthiness check
                // matters most here — this runs on every app start, including the
                // restart right after a rejected switch.
                if let Err(e) = self.capture_epic_if_trustworthy(&id) {
                    tracing::debug!(error = %e, account = %id, "epic token renew skipped");
                }
            }
        }
        Ok(())
    }

    /// Stop the platform and wait for it to fully exit, returning an error if it
    /// won't. Racing a still-running launcher's shutdown writes can corrupt its
    /// login files, so callers abort the switch rather than proceed.
    fn ensure_stopped(&self, plat: &PlatformDef) -> Result<()> {
        self.host.kill_processes(&plat.exes_to_end)?;
        let timeout_secs = plat.exit_timeout_secs.unwrap_or(DEFAULT_EXIT_TIMEOUT_SECS);
        if !self.wait_for_exit(&plat.exes_to_end, timeout_secs) {
            bail!(
                "{} is still running ~{}s after being asked to close; aborting the switch to avoid \
                 corrupting its files. If this launcher is just slow to close, raise its \
                 `exit_timeout_secs` in the platform definition.",
                plat.name,
                timeout_secs
            );
        }
        Ok(())
    }

    /// Poll until no process in `exe_names` remains, up to `timeout_secs`. Returns
    /// `true` if everything exited (or there was nothing to wait for), `false` if
    /// a process is still running when the timeout elapses.
    #[must_use]
    fn wait_for_exit(&self, exe_names: &[String], timeout_secs: u64) -> bool {
        if exe_names.is_empty() {
            return true;
        }
        let attempts = (timeout_secs * 1000 / EXIT_POLL_INTERVAL_MS).max(1);
        for _ in 0..attempts {
            if !self.host.are_running(exe_names) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(EXIT_POLL_INTERVAL_MS));
        }
        !self.host.are_running(exe_names)
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
                    let dest = dir.join(saved);
                    // Clear any previous capture first, so a re-snapshot replaces
                    // the saved login cleanly instead of merging new files over
                    // stale ones (e.g. obsolete leveldb segments would otherwise
                    // pile up and corrupt the profile over repeated switches).
                    remove_saved(&dest);
                    copy_into_saved(Path::new(&live), &dest)?;
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
        let snapshot = self.store.load_registry_snapshot(&plat.id, account_id)?;
        for artifact in &plat.login {
            match artifact {
                LoginArtifact::File { live, saved } => {
                    let live = self.host.expand_vars(live);
                    copy_from_saved(&dir.join(saved), Path::new(&live))?;
                }
                LoginArtifact::Registry { key, value, saved } => {
                    if let Some(data) = snapshot.get(saved) {
                        self.host.write_registry(key, value, data)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Snapshot the live login (files + registry values + clear-extras + any
    /// generated-id marker) into a throwaway temp directory so a failed restore
    /// can be undone. The returned [`LiveBackup`] cleans its temp copy up on drop.
    fn backup_live(&self, plat: &PlatformDef) -> Result<LiveBackup> {
        let seq = BACKUP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("playertwo-rollback-{}-{}", std::process::id(), seq));
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create rollback dir {}", dir.display()))?;
        let mut items = Vec::new();
        let mut n = 0usize;

        for artifact in &plat.login {
            match artifact {
                LoginArtifact::File { live, .. } => {
                    let live = self.host.expand_vars(live);
                    let rel = format!("f{n}");
                    n += 1;
                    copy_into_saved(Path::new(&live), &dir.join(&rel))?;
                    items.push(BackupItem::File { live, rel });
                }
                LoginArtifact::Registry { key, value, .. } => {
                    let data = self.host.read_registry(key, value)?;
                    items.push(BackupItem::Registry {
                        key: key.clone(),
                        value: value.clone(),
                        data,
                    });
                }
            }
        }
        for extra in &plat.clear {
            let live = self.host.expand_vars(extra);
            let rel = format!("f{n}");
            n += 1;
            copy_into_saved(Path::new(&live), &dir.join(&rel))?;
            items.push(BackupItem::File { live, rel });
        }
        if let UniqueId::GeneratedFile { file } = &plat.unique_id {
            let live = self.host.expand_vars(file);
            let rel = format!("f{n}");
            copy_into_saved(Path::new(&live), &dir.join(&rel))?;
            items.push(BackupItem::File { live, rel });
        }
        Ok(LiveBackup { dir, items })
    }

    /// Roll a [`LiveBackup`] back into the live locations after a failed restore.
    /// First clears any partially-restored target artifacts, then writes the
    /// snapshot back; artifacts that were absent at backup time stay absent.
    fn restore_backup(&self, plat: &PlatformDef, backup: &LiveBackup) -> Result<()> {
        self.clear_login(plat)?;
        for item in &backup.items {
            match item {
                BackupItem::File { live, rel } => {
                    copy_from_saved(&backup.dir.join(rel), Path::new(live))?;
                }
                BackupItem::Registry { key, value, data } => {
                    if let Some(data) = data {
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

// ---- rollback backup ----------------------------------------------------

/// Monotonic counter making rollback temp-dir names unique within this process.
static BACKUP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A single backed-up live artifact, enough to restore the prior state.
enum BackupItem {
    /// A file/dir/glob copied under `rel` in the backup dir (`live` is expanded).
    /// A missing `rel` copy means the artifact was absent and should stay absent.
    File { live: String, rel: String },
    /// A registry value; `data == None` means it was absent at backup time.
    Registry {
        key: String,
        value: String,
        data: Option<String>,
    },
}

/// A throwaway snapshot of the live login used to roll back a failed restore.
/// Removes its temp directory when dropped (on success or after rollback).
struct LiveBackup {
    dir: PathBuf,
    items: Vec<BackupItem>,
}

impl Drop for LiveBackup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
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
    // Wildcard artifact: files were captured by name into `saved/` (see
    // `copy_into_saved`). The live path is a glob, not a real destination, so
    // restore each saved file into the directory the glob points at.
    let live_str = live.to_string_lossy();
    if live_str.contains('*') {
        if let Some(parent) = live.parent() {
            std::fs::create_dir_all(parent)?;
            if saved.is_dir() {
                for entry in std::fs::read_dir(saved)? {
                    let entry = entry?;
                    if entry.path().is_file() {
                        std::fs::copy(entry.path(), parent.join(entry.file_name()))?;
                    }
                }
            }
        }
        return Ok(());
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

/// Remove a previously-saved artifact (file or directory), ignoring errors.
/// Used before a re-capture so stale files don't linger in the saved profile.
fn remove_saved(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else if path.is_file() {
        let _ = std::fs::remove_file(path);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::stub::StubHost;

    const IDS_KEY: &str = "HKCU\\Test\\Ids";
    const EPIC_IDS_KEY: &str = "HKCU\\Software\\Epic Games\\Unreal Engine\\Identifiers";

    /// A unique temp directory that cleans itself up on drop.
    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new(tag: &str) -> Self {
            let seq = BACKUP_SEQ.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("p2-test-{}-{}-{}", tag, std::process::id(), seq));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn acct(id: &str) -> Account {
        Account {
            id: id.into(),
            display_name: id.into(),
            note: None,
            image: None,
            last_used: None,
        }
    }

    /// A generic file+registry platform whose account id is its `AccountId` value.
    fn generic_plat(live_root: &Path) -> PlatformDef {
        let live = live_root.join("login.dat").to_string_lossy().to_string();
        PlatformDef {
            id: "testplat".into(),
            name: "Test".into(),
            exe_default: None,
            exe_locators: vec![],
            exe_args: None,
            exes_to_end: vec![],
            exit_timeout_secs: None,
            login: vec![
                LoginArtifact::File {
                    live,
                    saved: "login.dat".into(),
                },
                LoginArtifact::Registry {
                    key: IDS_KEY.into(),
                    value: "AccountId".into(),
                    saved: "accountid".into(),
                },
                LoginArtifact::Registry {
                    key: IDS_KEY.into(),
                    value: "Token".into(),
                    saved: "token".into(),
                },
            ],
            clear: vec![],
            unique_id: UniqueId::Registry {
                key: IDS_KEY.into(),
                value: "AccountId".into(),
            },
        }
    }

    /// Make `account_id` the live login: write its file data and registry values.
    fn set_live(host: &StubHost, plat: &PlatformDef, data: &str, account_id: &str) {
        for artifact in &plat.login {
            if let LoginArtifact::File { live, .. } = artifact {
                let p = PathBuf::from(live);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, data).unwrap();
            }
        }
        host.set_registry(IDS_KEY, "AccountId", account_id);
        host.set_registry(IDS_KEY, "Token", &format!("tok-{account_id}"));
    }

    fn engine_at(host: &StubHost, root: &Path) -> Engine {
        Engine::new(Box::new(host.clone()), Store::new(root.to_path_buf()))
    }

    #[test]
    fn full_switch_roundtrip() {
        let tmp = TempDir::new("rt");
        let live = tmp.path.join("live");
        let root = tmp.path.join("store");
        let host = StubHost::new();
        let plat = generic_plat(&live);
        let engine = engine_at(&host, &root);

        engine.store().upsert_account("testplat", acct("idA")).unwrap();
        engine.store().upsert_account("testplat", acct("idB")).unwrap();

        set_live(&host, &plat, "A-data", "idA");
        engine.capture_login(&plat, "idA").unwrap();
        set_live(&host, &plat, "B-data", "idB");
        engine.capture_login(&plat, "idB").unwrap();

        // Live is B; switch to A.
        let out = engine.switch(&plat, "idA", false).unwrap();
        assert!(out.switched && !out.already_active);
        assert_eq!(
            std::fs::read_to_string(live.join("login.dat")).unwrap(),
            "A-data"
        );
        assert_eq!(host.get_registry(IDS_KEY, "AccountId").as_deref(), Some("idA"));
        assert_eq!(engine.current_id(&plat).unwrap().as_deref(), Some("idA"));
    }

    #[test]
    fn already_active_short_circuits() {
        let tmp = TempDir::new("active");
        let live = tmp.path.join("live");
        let root = tmp.path.join("store");
        let host = StubHost::new();
        let plat = generic_plat(&live);
        let engine = engine_at(&host, &root);

        engine.store().upsert_account("testplat", acct("idA")).unwrap();
        set_live(&host, &plat, "A-data", "idA");
        engine.capture_login(&plat, "idA").unwrap();

        let out = engine.switch(&plat, "idA", false).unwrap();
        assert!(out.already_active && !out.switched);
        // Nothing was disturbed.
        assert_eq!(
            std::fs::read_to_string(live.join("login.dat")).unwrap(),
            "A-data"
        );
    }

    #[test]
    fn failed_restore_rolls_back() {
        let tmp = TempDir::new("rollback");
        let live = tmp.path.join("live");
        let root = tmp.path.join("store");
        let host = StubHost::new();
        let plat = generic_plat(&live);
        let engine = engine_at(&host, &root);

        engine.store().upsert_account("testplat", acct("idA")).unwrap();
        engine.store().upsert_account("testplat", acct("idB")).unwrap();
        set_live(&host, &plat, "A-data", "idA");
        engine.capture_login(&plat, "idA").unwrap();
        set_live(&host, &plat, "B-data", "idB");
        engine.capture_login(&plat, "idB").unwrap();

        // Restoring A writes AccountId=idA — make that write fail mid-restore.
        host.fail_registry_write_data("idA");
        let err = engine.switch(&plat, "idA", false);
        assert!(err.is_err(), "switch should fail when restore fails");

        // Rolled back to B: live file and registry are B's again.
        assert_eq!(
            std::fs::read_to_string(live.join("login.dat")).unwrap(),
            "B-data"
        );
        assert_eq!(host.get_registry(IDS_KEY, "AccountId").as_deref(), Some("idB"));
    }

    #[test]
    fn capture_failure_aborts_before_clear() {
        let tmp = TempDir::new("capfail");
        let live = tmp.path.join("live");
        let root = tmp.path.join("store");
        let host = StubHost::new();
        let plat = generic_plat(&live);
        let engine = engine_at(&host, &root);

        engine.store().upsert_account("testplat", acct("idA")).unwrap();
        engine.store().upsert_account("testplat", acct("idB")).unwrap();
        set_live(&host, &plat, "B-data", "idB");
        engine.capture_login(&plat, "idB").unwrap();

        // Capturing the outgoing account reads the Token value; make it fail.
        host.fail_registry_read("Token");
        let err = engine.switch(&plat, "idA", false);
        assert!(err.is_err(), "switch should abort when capture fails");

        // Live login for B is untouched — clear never ran.
        assert_eq!(
            std::fs::read_to_string(live.join("login.dat")).unwrap(),
            "B-data"
        );
        assert_eq!(host.get_registry(IDS_KEY, "AccountId").as_deref(), Some("idB"));
    }

    #[test]
    fn corrupt_accounts_json_errors_not_empty() {
        let tmp = TempDir::new("corrupt");
        let root = tmp.path.join("store");
        let store = Store::new(root.clone());
        let dir = root.join("accounts").join("testplat");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("accounts.json"), "{ not valid json").unwrap();
        assert!(store.list_accounts("testplat").is_err());
    }

    #[test]
    fn legacy_array_reads_and_migrates_to_versioned() {
        let tmp = TempDir::new("legacy");
        let root = tmp.path.join("store");
        let store = Store::new(root.clone());
        let dir = root.join("accounts").join("testplat");
        std::fs::create_dir_all(&dir).unwrap();
        // Legacy pre-versioning format: a bare array.
        std::fs::write(
            dir.join("accounts.json"),
            r#"[{"id":"x","display_name":"X"}]"#,
        )
        .unwrap();
        let a = store.list_accounts("testplat").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].id, "x");

        // A save rewrites it in the versioned format, still round-tripping.
        store.upsert_account("testplat", acct("y")).unwrap();
        let a2 = store.list_accounts("testplat").unwrap();
        assert_eq!(a2.len(), 2);
        let txt = std::fs::read_to_string(dir.join("accounts.json")).unwrap();
        assert!(txt.contains("\"version\""));
    }

    // ---- Epic-specific ----

    fn epic_ini_path(local_app_data: &Path) -> PathBuf {
        local_app_data
            .join("EpicGamesLauncher")
            .join("Saved")
            .join("Config")
            .join("WindowsEditor")
            .join("GameUserSettings.ini")
    }

    fn epic_plat() -> PlatformDef {
        PlatformDef {
            id: "epic".into(),
            name: "Epic".into(),
            exe_default: None,
            exe_locators: vec![],
            exe_args: None,
            exes_to_end: vec![],
            exit_timeout_secs: None,
            login: vec![],
            clear: vec![],
            unique_id: UniqueId::Registry {
                key: EPIC_IDS_KEY.into(),
                value: "AccountId".into(),
            },
        }
    }

    #[test]
    fn epic_switch_preserves_unrelated_ini_sections() {
        let tmp = TempDir::new("epicini");
        let lad = tmp.path.join("lad");
        let host = StubHost::new().with_var("LocalAppData", &lad.to_string_lossy());
        let store = Store::new(tmp.path.join("store"));

        let ini = epic_ini_path(&lad);
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        std::fs::write(
            &ini,
            "[/Script/Foo]\nBar=1\n\n[RememberMe]\nEnable=True\nData=old\n",
        )
        .unwrap();

        let token = "z".repeat(600);
        let adir = store.account_dir("epic", "acct1");
        std::fs::create_dir_all(&adir).unwrap();
        std::fs::write(adir.join("epic_token.txt"), &token).unwrap();

        crate::switcher::epic::switch(&host, &store, "acct1").unwrap();

        let out = std::fs::read_to_string(&ini).unwrap();
        assert!(out.contains("[/Script/Foo]"), "unrelated section preserved");
        assert!(out.contains("Bar=1"));
        assert!(out.contains(&format!("Data={token}")));
        assert!(!out.contains("Data=old"));
    }

    #[test]
    fn epic_switch_captures_rotated_token_before_switching_away() {
        let tmp = TempDir::new("epicrot");
        let lad = tmp.path.join("lad");
        let root = tmp.path.join("store");
        let host = StubHost::new().with_var("LocalAppData", &lad.to_string_lossy());

        // Outgoing account is live with a freshly-rotated token.
        let fresh = "f".repeat(600);
        let ini = epic_ini_path(&lad);
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        std::fs::write(&ini, format!("[RememberMe]\nEnable=True\nData={fresh}\n")).unwrap();
        host.set_registry(EPIC_IDS_KEY, "AccountId", "idOut");

        let engine = engine_at(&host, &root);
        engine.store().upsert_account("epic", acct("idOut")).unwrap();
        engine.store().upsert_account("epic", acct("idIn")).unwrap();

        // Incoming account already has a saved token so the switch can complete.
        let store = Store::new(root.clone());
        let indir = store.account_dir("epic", "idIn");
        std::fs::create_dir_all(&indir).unwrap();
        let in_token = "i".repeat(600);
        std::fs::write(indir.join("epic_token.txt"), &in_token).unwrap();

        let out = engine.switch_epic(&epic_plat(), "idIn", false).unwrap();
        assert!(out.switched);

        // The outgoing account's rotated token was saved before being overwritten.
        let saved_out =
            std::fs::read_to_string(store.account_dir("epic", "idOut").join("epic_token.txt"))
                .unwrap();
        assert_eq!(saved_out, fresh);

        // The live INI now carries the incoming account's token.
        let ini_txt = std::fs::read_to_string(&ini).unwrap();
        assert!(ini_txt.contains(&format!("Data={in_token}")));
    }

    /// Build an Epic scenario: three saved accounts, `live` signed in with a
    /// token of its own. Returns (tempdir, host, store root, ini path).
    fn epic_fixture(tag: &str, live: &str, live_token: &str) -> (TempDir, StubHost, PathBuf, PathBuf) {
        let tmp = TempDir::new(tag);
        let lad = tmp.path.join("lad");
        let root = tmp.path.join("store");
        let host = StubHost::new().with_var("LocalAppData", &lad.to_string_lossy());
        let ini = epic_ini_path(&lad);
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        std::fs::write(&ini, format!("[RememberMe]\nEnable=True\nData={live_token}\n")).unwrap();
        host.set_registry(EPIC_IDS_KEY, "AccountId", live);
        (tmp, host, root, ini)
    }

    /// Give `account_id` a saved token so a switch to it can complete.
    fn seed_epic_token(root: &Path, account_id: &str, token: &str) {
        let dir = Store::new(root.to_path_buf()).account_dir("epic", account_id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("epic_token.txt"), token).unwrap();
    }

    fn saved_epic_token(root: &Path, account_id: &str) -> String {
        let dir = Store::new(root.to_path_buf()).account_dir("epic", account_id);
        std::fs::read_to_string(dir.join("epic_token.txt")).unwrap()
    }

    /// The corruption this guard exists for: switching to B writes B's token while
    /// the registry still says A (the launcher rejected it and never signed in).
    /// A later switch to C must not then save B's token as A's.
    #[test]
    fn epic_rejected_switch_does_not_misfile_the_planted_token() {
        let a_token = "a".repeat(600);
        let b_token = "b".repeat(700);
        let c_token = "c".repeat(800);
        let (_tmp, host, root, _ini) = epic_fixture("epicmisfile", "idA", &a_token);

        let engine = engine_at(&host, &root);
        for id in ["idA", "idB", "idC"] {
            engine.store().upsert_account("epic", acct(id)).unwrap();
        }
        seed_epic_token(&root, "idA", &a_token);
        seed_epic_token(&root, "idB", &b_token);
        seed_epic_token(&root, "idC", &c_token);

        // Switch to B. B's token lands in the INI; the launcher never signs in, so
        // `AccountId` stays on A.
        engine.switch_epic(&epic_plat(), "idB", false).unwrap();
        assert_eq!(engine.epic_unconfirmed().as_deref(), Some("idB"));

        // Now switch to C. The outgoing "current" account is still A by the
        // registry, but the live token is B's — A must be left alone.
        engine.switch_epic(&epic_plat(), "idC", false).unwrap();
        assert_eq!(
            saved_epic_token(&root, "idA"),
            a_token,
            "A's saved token must survive an unconfirmed switch to another account"
        );
        assert_eq!(saved_epic_token(&root, "idB"), b_token, "B's token untouched");
    }

    /// The same guard must not block the normal path: once the launcher confirms
    /// the sign-in, captures work again.
    #[test]
    fn epic_confirmed_switch_clears_the_pending_record() {
        let a_token = "a".repeat(600);
        let b_token = "b".repeat(700);
        let (_tmp, host, root, ini) = epic_fixture("epicconfirm", "idA", &a_token);

        let engine = engine_at(&host, &root);
        engine.store().upsert_account("epic", acct("idA")).unwrap();
        engine.store().upsert_account("epic", acct("idB")).unwrap();
        seed_epic_token(&root, "idB", &b_token);

        engine.switch_epic(&epic_plat(), "idB", false).unwrap();
        assert_eq!(engine.epic_unconfirmed().as_deref(), Some("idB"));

        // The launcher signs in and rotates the token, as it does every session.
        host.set_registry(EPIC_IDS_KEY, "AccountId", "idB");
        let rotated = "r".repeat(900);
        std::fs::write(&ini, format!("[RememberMe]\nEnable=True\nData={rotated}\n")).unwrap();

        assert!(engine.epic_unconfirmed().is_none(), "confirmed switch clears the record");
        // And switching away now saves B's *rotated* token, as it always has.
        seed_epic_token(&root, "idA", &a_token);
        engine.switch_epic(&epic_plat(), "idA", false).unwrap();
        assert_eq!(saved_epic_token(&root, "idB"), rotated);
    }

    /// `AccountId` outliving a sign-out must not short-circuit as "already
    /// active" — that left the user stuck at Epic's login screen with the switch
    /// button doing nothing.
    #[test]
    fn epic_reapplies_token_when_account_id_is_stale() {
        let a_token = "a".repeat(600);
        // Signed out: the launcher leaves a short placeholder behind.
        let (_tmp, host, root, ini) = epic_fixture("epicstale", "idA", "shortstub");

        let engine = engine_at(&host, &root);
        engine.store().upsert_account("epic", acct("idA")).unwrap();
        seed_epic_token(&root, "idA", &a_token);

        let out = engine.switch_epic(&epic_plat(), "idA", false).unwrap();
        assert!(out.switched, "should re-apply the saved login, not report already-active");
        assert!(!out.already_active);
        let ini_txt = std::fs::read_to_string(&ini).unwrap();
        assert!(ini_txt.contains(&format!("Data={a_token}")));
    }
}
