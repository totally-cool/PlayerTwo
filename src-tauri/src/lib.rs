//! PlayerTwo — Tauri backend entry point.
//!
//! Architecture:
//! - `switcher` — OS-independent model + switch engine + saved-account store
//! - `os`       — the portability seam (Windows today; Linux/macOS later)
//! - `defs`     — embedded platform definitions (JSON)
//! - `commands` — the API surface exposed to the React/MUI frontend

mod commands;
mod defs;
mod os;
mod switcher;

use commands::AppState;
use switcher::settings::Settings;

/// Initialize file logging next to the executable. Returns the appender guard,
/// which must be kept alive for the duration of the program.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let path = commands::log_file_path();
    let dir = path.parent()?.to_path_buf();
    let name = path.file_name()?.to_string_lossy().to_string();

    let verbose = Settings::load(&commands::default_data_dir()).debug_logging;
    let level = if verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    let (writer, guard) = tracing_appender::non_blocking(tracing_appender::rolling::never(dir, name));
    let _ = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_max_level(level)
        .try_init();

    tracing::info!(debug_logging = verbose, "PlayerTwo starting");
    Some(guard)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Held until the app exits so buffered logs are flushed.
    let _log_guard = init_logging();

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_process::init());
    }
    builder
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::list_platforms,
            commands::list_accounts,
            commands::switch_account,
            commands::current_account_id,
            commands::add_current_account,
            commands::prepare_new_login,
            commands::renew_active_tokens,
            commands::update_account,
            commands::forget_account,
            commands::get_data_dir,
            commands::set_data_dir,
            commands::get_settings,
            commands::update_settings,
            commands::set_platform_enabled,
            commands::set_platform_silent,
            commands::get_log_path,
            commands::open_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
