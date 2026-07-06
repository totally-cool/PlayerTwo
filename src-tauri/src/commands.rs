//! Tauri command layer — the API surface the React frontend calls via `invoke`.
//!
//! Each command builds an [`Engine`] for the currently-configured data dir,
//! does its work, and returns plain serializable data / `Result<_, String>`.

use crate::switcher::engine::{Engine, SwitchOutcome};
use crate::switcher::model::{Account, UniqueId};
use crate::switcher::settings::Settings;
use crate::switcher::store::Store;
use crate::{defs, os};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Process-wide state managed by Tauri.
pub struct AppState {
    /// Where saved accounts live. Changeable at runtime (e.g. point at a NAS).
    pub data_dir: Mutex<PathBuf>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            data_dir: Mutex::new(default_data_dir()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    let s = e.to_string();
    tracing::error!(error = %s, "command failed");
    s
}

fn engine_for(state: &AppState) -> Engine {
    let dir = state.data_dir.lock().unwrap().clone();
    Engine::new(os::host(), Store::new(dir))
}

/// Run blocking engine work off the UI thread. Switching kills processes, waits
/// for them to exit (up to ~5s), and copies login files (possibly to a NAS) —
/// doing that on Tauri's main thread freezes the window, so we hand it to the
/// blocking pool and `await` the result.
async fn run_engine<T, F>(dir: PathBuf, f: F) -> CmdResult<T>
where
    F: FnOnce(&Engine) -> CmdResult<T> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new(os::host(), Store::new(dir));
        f(&engine)
    })
    .await
    .map_err(|e| err(format!("background task failed: {e}")))?
}

/// Whether Epic should launch silently after switching (per-platform setting).
fn epic_silent(dir: &std::path::Path) -> bool {
    Settings::load(dir)
        .platforms
        .get("epic")
        .and_then(|p| p.silent)
        .unwrap_or(true)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Overlay the platform's most-recently-used timestamps onto `accounts` and sort
/// most-recently-used first, keeping the stored (manual) order as the tiebreaker.
fn apply_mru(store: &Store, platform: &str, accounts: Vec<Account>) -> Vec<Account> {
    let mru = store.load_last_used(platform);
    let mut indexed: Vec<(usize, Account)> = accounts
        .into_iter()
        .enumerate()
        .map(|(i, mut a)| {
            a.last_used = mru.get(&a.id).copied();
            (i, a)
        })
        .collect();
    // Higher timestamp first; `None` sorts last; equal/none keep original order.
    indexed.sort_by(|(ia, a), (ib, b)| b.last_used.cmp(&a.last_used).then(ia.cmp(ib)));
    indexed.into_iter().map(|(_, a)| a).collect()
}

// ---- data dir persistence ----------------------------------------------

pub fn default_data_dir() -> PathBuf {
    if let Some(saved) = load_saved_data_dir() {
        return saved;
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PlayerTwo")
}

/// Path to the log file — next to the executable.
pub fn log_file_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("playertwo.log")
}

#[tauri::command]
pub fn get_log_path() -> String {
    log_file_path().display().to_string()
}

/// Open the log file in Notepad (Windows).
#[tauri::command]
pub fn open_log() -> CmdResult<()> {
    let path = log_file_path();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("notepad.exe")
            .arg(&path)
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(err)?;
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

/// Small marker file recording a user-chosen data dir (mirrors the "custom
/// user data location" idea — this is what lets the store live on a NAS).
fn data_dir_marker() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PlayerTwo")
        .join("data_path.txt")
}

fn load_saved_data_dir() -> Option<PathBuf> {
    std::fs::read_to_string(data_dir_marker())
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty())
}

// ---- commands -----------------------------------------------------------

#[derive(Serialize)]
pub struct PlatformSummary {
    pub id: String,
    pub name: String,
    pub account_count: usize,
    /// Auto-detected as installed/used on this machine.
    pub detected: bool,
    /// Whether it should be shown in the main UI (explicit setting or detection).
    pub enabled: bool,
}

#[tauri::command]
pub async fn list_platforms(
    state: tauri::State<'_, AppState>,
) -> CmdResult<Vec<PlatformSummary>> {
    let dir = state.data_dir.lock().unwrap().clone();
    let settings_dir = dir.clone();
    run_engine(dir, move |engine| {
        let settings = Settings::load(&settings_dir);
        let mut out = Vec::new();
        for def in defs::builtin() {
            let count = engine.store().list_accounts(&def.id).map_err(err)?.len();
            let detected = engine.is_installed(&def);
            let enabled = settings.platform_enabled(&def.id, detected);
            out.push(PlatformSummary {
                id: def.id,
                name: def.name,
                account_count: count,
                detected,
                enabled,
            });
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub fn get_settings(state: tauri::State<AppState>) -> Settings {
    let dir = state.data_dir.lock().unwrap().clone();
    Settings::load(&dir)
}

#[tauri::command]
pub fn update_settings(state: tauri::State<AppState>, settings: Settings) -> CmdResult<()> {
    let dir = state.data_dir.lock().unwrap().clone();
    settings.save(&dir).map_err(err)
}

/// Manually enable or disable a platform (overrides auto-detection).
#[tauri::command]
pub fn set_platform_enabled(
    state: tauri::State<AppState>,
    platform: String,
    enabled: bool,
) -> CmdResult<()> {
    let dir = state.data_dir.lock().unwrap().clone();
    let mut settings = Settings::load(&dir);
    settings.platforms.entry(platform).or_default().enabled = Some(enabled);
    settings.save(&dir).map_err(err)
}

#[tauri::command]
pub async fn list_accounts(
    state: tauri::State<'_, AppState>,
    platform: String,
) -> CmdResult<Vec<Account>> {
    let dir = state.data_dir.lock().unwrap().clone();
    run_engine(dir, move |engine| {
        if platform == "steam" {
            // Accounts come live from Steam; overlay any saved name/note/image.
            let overrides = engine.store().list_accounts("steam").unwrap_or_default();
            let merged: Vec<Account> = engine
                .steam_accounts()
                .into_iter()
                .map(|mut a| {
                    if let Some(o) = overrides.iter().find(|o| o.id == a.id) {
                        a.display_name = o.display_name.clone();
                        a.note = o.note.clone().or(a.note);
                        a.image = o.image.clone();
                    }
                    a
                })
                .collect();
            return Ok(apply_mru(engine.store(), "steam", merged));
        }
        let accounts = engine.store().list_accounts(&platform).map_err(err)?;
        Ok(apply_mru(engine.store(), &platform, accounts))
    })
    .await
}

#[tauri::command]
pub async fn switch_account(
    state: tauri::State<'_, AppState>,
    platform: String,
    account_id: String,
    auto_start: bool,
) -> CmdResult<SwitchOutcome> {
    tracing::info!(platform = %platform, account = %account_id, "switch requested");
    let mut def = defs::by_id(&platform).ok_or_else(|| format!("unknown platform: {platform}"))?;
    let dir = state.data_dir.lock().unwrap().clone();
    if platform == "epic" && !epic_silent(&dir) {
        def.exe_args = None;
    }
    run_engine(dir, move |engine| {
        let outcome = if platform == "steam" {
            engine.switch_steam(&def, &account_id, auto_start).map_err(err)?
        } else if platform == "epic" {
            engine.switch_epic(&def, &account_id, auto_start).map_err(err)?
        } else {
            engine.switch(&def, &account_id, auto_start).map_err(err)?
        };
        // Remember this as the most recent account for MRU ordering. Non-critical:
        // a failure here shouldn't fail the (already-completed) switch.
        if let Err(e) = engine.store().record_last_used(&platform, &account_id, now()) {
            tracing::debug!(error = %e, "failed to record last-used timestamp");
        }
        Ok(outcome)
    })
    .await
}

/// When the saved Epic login token for an account was last written (unix seconds),
/// if any. Drives the "login saved <relative time>" hint on Epic cards.
#[tauri::command]
pub async fn epic_token_saved_at(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> CmdResult<Option<u64>> {
    let dir = state.data_dir.lock().unwrap().clone();
    run_engine(dir, move |engine| Ok(engine.epic_token_saved_at(&account_id))).await
}

/// The unique id of the account currently logged in on the system, if detectable.
/// Used to highlight the active profile in the UI.
#[tauri::command]
pub async fn current_account_id(
    state: tauri::State<'_, AppState>,
    platform: String,
) -> CmdResult<Option<String>> {
    let dir = state.data_dir.lock().unwrap().clone();
    run_engine(dir, move |engine| {
        if platform == "steam" {
            return Ok(engine.steam_current());
        }
        if platform == "epic" {
            return Ok(engine.epic_current());
        }
        let def =
            defs::by_id(&platform).ok_or_else(|| format!("unknown platform: {platform}"))?;
        engine.current_id(&def).map_err(err)
    })
    .await
}

/// Result of capturing the current login: either a freshly added account, or a
/// flag that the detected account already exists (so the UI can offer a rename).
#[derive(Serialize)]
pub struct AddResult {
    pub exists: bool,
    pub account: Account,
}

/// Capture the *currently* logged-in account and save it under `display_name`.
/// If the detected account already exists, returns it with `exists: true` and
/// does not overwrite — the caller should offer to rename instead.
#[tauri::command]
pub async fn add_current_account(
    state: tauri::State<'_, AppState>,
    platform: String,
    display_name: String,
) -> CmdResult<AddResult> {
    let dir = state.data_dir.lock().unwrap().clone();
    run_engine(dir, move |engine| {
    let def = defs::by_id(&platform).ok_or_else(|| format!("unknown platform: {platform}"))?;

    // Steam: accounts live in Steam itself; "import" just saves a name override
    // for the currently signed-in account.
    if platform == "steam" {
        let id = engine
            .steam_current()
            .ok_or_else(|| "No Steam account is currently signed in".to_string())?;
        if let Some(existing) = engine
            .store()
            .list_accounts("steam")
            .map_err(err)?
            .into_iter()
            .find(|a| a.id == id)
        {
            return Ok(AddResult {
                exists: true,
                account: existing,
            });
        }
        let account = Account {
            id,
            display_name,
            note: None,
            image: None,
            last_used: None,
        };
        engine
            .store()
            .upsert_account("steam", account.clone())
            .map_err(err)?;
        return Ok(AddResult {
            exists: false,
            account,
        });
    }

    // Epic: capture just the RememberMe token; name from logs if not provided.
    if platform == "epic" {
        let id = engine
            .epic_current()
            .ok_or_else(|| "No Epic account is currently signed in".to_string())?;
        if let Some(existing) = engine
            .store()
            .list_accounts("epic")
            .map_err(err)?
            .into_iter()
            .find(|a| a.id == id)
        {
            return Ok(AddResult {
                exists: true,
                account: existing,
            });
        }
        engine.capture_epic(&id).map_err(err)?;
        let name = if display_name.trim().is_empty() {
            engine.epic_username().unwrap_or_else(|| id.clone())
        } else {
            display_name
        };
        let account = Account {
            id,
            display_name: name,
            note: None,
            image: None,
            last_used: None,
        };
        engine
            .store()
            .upsert_account("epic", account.clone())
            .map_err(err)?;
        return Ok(AddResult {
            exists: false,
            account,
        });
    }

    let id = match engine.current_id(&def).map_err(err)? {
        Some(id) => id,
        None => {
            // Platforms without a natural id get a freshly minted marker.
            if let UniqueId::GeneratedFile { file } = &def.unique_id {
                let id = format!("acct-{}", now());
                std::fs::write(engine.expand_vars(file), &id).map_err(err)?;
                id
            } else {
                return Err("Could not detect a logged-in account to add".into());
            }
        }
    };

    // Already saved? Report it rather than creating a duplicate.
    if let Some(existing) = engine
        .store()
        .list_accounts(&platform)
        .map_err(err)?
        .into_iter()
        .find(|a| a.id == id)
    {
        return Ok(AddResult {
            exists: true,
            account: existing,
        });
    }

    engine.capture_login(&def, &id).map_err(err)?;
    let account = Account {
        id,
        display_name,
        note: None,
        image: None,
        last_used: None,
    };
    engine
        .store()
        .upsert_account(&platform, account.clone())
        .map_err(err)?;
    Ok(AddResult {
        exists: false,
        account,
    })
    })
    .await
}

/// Begin a new-account login: clear the current login and launch the platform so
/// the user can sign into a different account, which they then capture via
/// `add_current_account`.
#[tauri::command]
pub async fn prepare_new_login(
    state: tauri::State<'_, AppState>,
    platform: String,
) -> CmdResult<bool> {
    let mut def = defs::by_id(&platform).ok_or_else(|| format!("unknown platform: {platform}"))?;
    let dir = state.data_dir.lock().unwrap().clone();
    if platform == "epic" && !epic_silent(&dir) {
        def.exe_args = None;
    }
    run_engine(dir, move |engine| {
        if platform == "steam" {
            // Steam manages its own account list; just open it so the user can use
            // Steam's "Add account" — clearing files would wipe other accounts.
            return engine.launch(&def, true).map_err(err);
        }
        if platform == "epic" {
            return engine.epic_new_login(&def, true).map_err(err);
        }
        engine.begin_new_login(&def, true).map_err(err)
    })
    .await
}

/// Set a platform's "launch silently after switching" preference.
#[tauri::command]
pub fn set_platform_silent(
    state: tauri::State<AppState>,
    platform: String,
    silent: bool,
) -> CmdResult<()> {
    let dir = state.data_dir.lock().unwrap().clone();
    let mut settings = Settings::load(&dir);
    settings.platforms.entry(platform).or_default().silent = Some(silent);
    settings.save(&dir).map_err(err)
}

/// Refresh the saved login token for the currently-active account where the
/// platform rotates tokens (currently Epic). Safe to call on startup.
#[tauri::command]
pub async fn renew_active_tokens(state: tauri::State<'_, AppState>) -> CmdResult<()> {
    let dir = state.data_dir.lock().unwrap().clone();
    run_engine(dir, move |engine| engine.epic_renew().map_err(err)).await
}

/// Edit an account's display name / note (id is immutable).
#[tauri::command]
pub fn update_account(
    state: tauri::State<AppState>,
    platform: String,
    account: Account,
) -> CmdResult<()> {
    engine_for(&state)
        .store()
        .upsert_account(&platform, account)
        .map_err(err)
}

#[tauri::command]
pub async fn forget_account(
    state: tauri::State<'_, AppState>,
    platform: String,
    account_id: String,
) -> CmdResult<()> {
    tracing::info!(platform = %platform, account = %account_id, "forget account");
    let dir = state.data_dir.lock().unwrap().clone();
    run_engine(dir, move |engine| {
        engine
            .store()
            .remove_account(&platform, &account_id)
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub fn get_data_dir(state: tauri::State<AppState>) -> String {
    state.data_dir.lock().unwrap().display().to_string()
}

/// Point the store at a new location (local path, mapped drive, or UNC share).
#[tauri::command]
pub fn set_data_dir(state: tauri::State<AppState>, path: String) -> CmdResult<()> {
    let new_dir = PathBuf::from(path.trim());
    if new_dir.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    std::fs::create_dir_all(&new_dir).map_err(err)?;

    let marker = data_dir_marker();
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    std::fs::write(&marker, new_dir.display().to_string()).map_err(err)?;

    tracing::info!(path = %new_dir.display(), "data location changed");
    *state.data_dir.lock().unwrap() = new_dir;
    Ok(())
}
