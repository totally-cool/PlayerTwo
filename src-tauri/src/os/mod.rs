//! Operating-system abstraction — **the portability seam**.
//!
//! The engine only ever talks to a [`Host`]. To add Linux/macOS later you
//! implement this trait in a new module and return it from [`host`]; nothing
//! in `core/` needs to change.

use anyhow::Result;

#[cfg(windows)]
pub mod windows;

#[cfg(not(windows))]
pub mod stub;

/// All OS-specific operations the engine needs.
pub trait Host: Send + Sync {
    /// Expand `%VAR%`-style placeholders (e.g. `%AppData%`) into a concrete path.
    fn expand_vars(&self, input: &str) -> String;

    /// Read a registry value as a string, or `None` if missing.
    /// `key` is like `HKCU\\Software\\Foo`.
    fn read_registry(&self, key: &str, value: &str) -> Result<Option<String>>;

    /// Write a string registry value, creating the key if needed.
    fn write_registry(&self, key: &str, value: &str, data: &str) -> Result<()>;

    /// Delete a registry value (no error if already absent).
    fn delete_registry_value(&self, key: &str, value: &str) -> Result<()>;

    /// Terminate all running processes whose image name matches any of `exe_names`.
    fn kill_processes(&self, exe_names: &[String]) -> Result<()>;

    /// Whether any process matching `exe_names` is currently running.
    /// Used to wait for a killed launcher to fully exit before relaunching.
    fn are_running(&self, exe_names: &[String]) -> bool;

    /// Launch an executable, optionally elevated, with the given argument string.
    fn launch(&self, exe: &str, args: &str, elevated: bool) -> Result<()>;
}

/// Construct the host implementation for the current platform.
pub fn host() -> Box<dyn Host> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsHost::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(stub::StubHost)
    }
}
