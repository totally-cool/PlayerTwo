//! Tauri command layer — the API surface the React frontend calls via `invoke`.
//!
//! Each command builds an [`Engine`] for the currently-configured data dir,
//! does its work, and returns plain serializable data / `Result<_, String>`.

use crate::switcher::engine::{Engine, SwitchOutcome};
use crate::switcher::model::{Account, PlatformDef, UniqueId};
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

/// Apply Epic's per-platform "launch silently" setting to the def's launch args.
fn apply_epic_silent(state: &AppState, def: &mut PlatformDef) {
    let dir = state.data_dir.lock().unwrap().clone();
    let silent = Settings::load(&dir)
        .platforms
        .get("epic")
        .and_then(|p| p.silent)
        .unwrap_or(true);
    if !silent {
        def.exe_args = None;
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
pub fn list_platforms(state: tauri::State<AppState>) -> CmdResult<Vec<PlatformSummary>> {
    let engine = engine_for(&state);
    let dir = state.data_dir.lock().unwrap().clone();
    let settings = Settings::load(&dir);
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
pub fn list_accounts(state: tauri::State<AppState>, platform: String) -> CmdResult<Vec<Account>> {
    let engine = engine_for(&state);
    if platform == "steam" {
        // Accounts come live from Steam; overlay any saved name/note/image.
        let overrides = engine.store().list_accounts("steam").unwrap_or_default();
        let merged = engine
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
        return Ok(merged);
    }
    engine.store().list_accounts(&platform).map_err(err)
}

#[tauri::command]
pub fn switch_account(
    state: tauri::State<AppState>,
    platform: String,
    account_id: String,
    auto_start: bool,
) -> CmdResult<SwitchOutcome> {
    tracing::info!(platform = %platform, account = %account_id, "switch requested");
    let mut def = defs::by_id(&platform).ok_or_else(|| format!("unknown platform: {platform}"))?;
    let engine = engine_for(&state);
    if platform == "steam" {
        return engine.switch_steam(&def, &account_id, auto_start).map_err(err);
    }
    if platform == "epic" {
        apply_epic_silent(&state, &mut def);
        return engine.switch_epic(&def, &account_id, auto_start).map_err(err);
    }
    engine.switch(&def, &account_id, auto_start).map_err(err)
}

/// The unique id of the account currently logged in on the system, if detectable.
/// Used to highlight the active profile in the UI.
#[tauri::command]
pub fn current_account_id(
    state: tauri::State<AppState>,
    platform: String,
) -> CmdResult<Option<String>> {
    if platform == "steam" {
        return Ok(engine_for(&state).steam_current());
    }
    if platform == "epic" {
        return Ok(engine_for(&state).epic_current());
    }
    let def = defs::by_id(&platform).ok_or_else(|| format!("unknown platform: {platform}"))?;
    engine_for(&state).current_id(&def).map_err(err)
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
pub fn add_current_account(
    state: tauri::State<AppState>,
    platform: String,
    display_name: String,
) -> CmdResult<AddResult> {
    let def = defs::by_id(&platform).ok_or_else(|| format!("unknown platform: {platform}"))?;
    let engine = engine_for(&state);

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
    };
    engine
        .store()
        .upsert_account(&platform, account.clone())
        .map_err(err)?;
    Ok(AddResult {
        exists: false,
        account,
    })
}

/// Begin a new-account login: clear the current login and launch the platform so
/// the user can sign into a different account, which they then capture via
/// `add_current_account`.
#[tauri::command]
pub fn prepare_new_login(state: tauri::State<AppState>, platform: String) -> CmdResult<bool> {
    let mut def = defs::by_id(&platform).ok_or_else(|| format!("unknown platform: {platform}"))?;
    let engine = engine_for(&state);
    if platform == "steam" {
        // Steam manages its own account list; just open it so the user can use
        // Steam's "Add account" — clearing files would wipe other accounts.
        return engine.launch(&def, true).map_err(err);
    }
    if platform == "epic" {
        apply_epic_silent(&state, &mut def);
        return engine.epic_new_login(&def, true).map_err(err);
    }
    engine.begin_new_login(&def, true).map_err(err)
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
pub fn renew_active_tokens(state: tauri::State<AppState>) -> CmdResult<()> {
    engine_for(&state).epic_renew().map_err(err)
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
pub fn forget_account(
    state: tauri::State<AppState>,
    platform: String,
    account_id: String,
) -> CmdResult<()> {
    tracing::info!(platform = %platform, account = %account_id, "forget account");
    engine_for(&state)
        .store()
        .remove_account(&platform, &account_id)
        .map_err(err)
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
