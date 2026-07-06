//! Windows implementation of [`Host`].

use super::Host;
use anyhow::{anyhow, Context, Result};
use std::os::windows::process::CommandExt;
use std::process::Command;

#[cfg(windows)]
use winreg::{enums::*, RegKey};

/// Spawn without flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct WindowsHost;

impl WindowsHost {
    pub fn new() -> Self {
        WindowsHost
    }
}

/// Split a `HKXX\\Sub\\Path` string into (predefined hive, subkey path).
#[cfg(windows)]
fn split_hive(key: &str) -> Result<(RegKey, String)> {
    let (root, rest) = key
        .split_once('\\')
        .ok_or_else(|| anyhow!("registry key missing hive: {key}"))?;
    let hive = match root.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCR" | "HKEY_CLASSES_ROOT" => HKEY_CLASSES_ROOT,
        "HKU" | "HKEY_USERS" => HKEY_USERS,
        other => return Err(anyhow!("unsupported registry hive: {other}")),
    };
    Ok((RegKey::predef(hive), rest.to_string()))
}

impl Host for WindowsHost {
    fn expand_vars(&self, input: &str) -> String {
        // Replace each %NAME% with the matching environment variable.
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                let mut name = String::new();
                let mut closed = false;
                for c2 in chars.by_ref() {
                    if c2 == '%' {
                        closed = true;
                        break;
                    }
                    name.push(c2);
                }
                match (closed, std::env::var(&name)) {
                    (true, Ok(val)) => out.push_str(&val),
                    // Unknown / unterminated: emit literally so nothing is silently lost.
                    (true, Err(_)) => {
                        out.push('%');
                        out.push_str(&name);
                        out.push('%');
                    }
                    (false, _) => {
                        out.push('%');
                        out.push_str(&name);
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[cfg(windows)]
    fn read_registry(&self, key: &str, value: &str) -> Result<Option<String>> {
        let (hive, sub) = split_hive(key)?;
        let sub = self.expand_vars(&sub);
        let opened = match hive.open_subkey(&sub) {
            Ok(k) => k,
            Err(_) => return Ok(None),
        };
        match opened.get_value::<String, _>(value) {
            Ok(v) => Ok(Some(v)),
            Err(_) => Ok(None),
        }
    }

    #[cfg(windows)]
    fn write_registry(&self, key: &str, value: &str, data: &str) -> Result<()> {
        let (hive, sub) = split_hive(key)?;
        let sub = self.expand_vars(&sub);
        let (opened, _) = hive
            .create_subkey(&sub)
            .with_context(|| format!("create_subkey {sub}"))?;
        opened
            .set_value(value, &data.to_string())
            .with_context(|| format!("set {value}"))?;
        Ok(())
    }

    #[cfg(windows)]
    fn delete_registry_value(&self, key: &str, value: &str) -> Result<()> {
        let (hive, sub) = split_hive(key)?;
        let sub = self.expand_vars(&sub);
        if let Ok(opened) = hive.open_subkey_with_flags(&sub, KEY_SET_VALUE) {
            let _ = opened.delete_value(value); // ignore "not found"
        }
        Ok(())
    }

    fn kill_processes(&self, exe_names: &[String]) -> Result<()> {
        use sysinfo::System;
        if exe_names.is_empty() {
            return Ok(());
        }
        let targets: Vec<String> = exe_names.iter().map(|n| n.to_ascii_lowercase()).collect();
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        // Collect the PIDs of the matching processes up front.
        let pids: std::collections::HashSet<u32> = sys
            .processes()
            .iter()
            .filter(|(_, p)| {
                let name = p.name().to_string_lossy().to_ascii_lowercase();
                targets.iter().any(|t| *t == name)
            })
            .map(|(pid, _)| pid.as_u32())
            .collect();
        if pids.is_empty() {
            return Ok(());
        }

        // 1. Ask nicely first: WM_CLOSE to each top-level window. Killing a
        //    launcher mid-write corrupts its own login files, so give it a moment
        //    to flush and exit cleanly before resorting to TerminateProcess.
        #[cfg(windows)]
        graceful_close(&pids);

        // 2. Wait briefly for the graceful close to take effect (~1.5s).
        for _ in 0..15 {
            if !self.are_running(exe_names) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // 3. Force-kill whatever is still alive.
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        for proc in sys.processes().values() {
            let name = proc.name().to_string_lossy().to_ascii_lowercase();
            if targets.iter().any(|t| *t == name) {
                proc.kill();
            }
        }
        Ok(())
    }

    fn are_running(&self, exe_names: &[String]) -> bool {
        use sysinfo::System;
        if exe_names.is_empty() {
            return false;
        }
        let targets: Vec<String> = exe_names.iter().map(|n| n.to_ascii_lowercase()).collect();
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        sys.processes().values().any(|proc| {
            let name = proc.name().to_string_lossy().to_ascii_lowercase();
            targets.iter().any(|t| *t == name)
        })
    }

    fn launch(&self, exe: &str, args: &str, elevated: bool) -> Result<()> {
        let exe = self.expand_vars(exe);
        if elevated {
            // Use the shell "runas" verb to trigger UAC elevation.
            let mut ps_args = format!("Start-Process -Verb RunAs -FilePath '{}'", exe);
            if !args.trim().is_empty() {
                ps_args.push_str(&format!(" -ArgumentList '{}'", args));
            }
            Command::new("powershell")
                .args(["-NoProfile", "-Command", &ps_args])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .with_context(|| format!("elevated launch {exe}"))?;
        } else {
            let mut cmd = Command::new(&exe);
            if !args.trim().is_empty() {
                cmd.args(args.split_whitespace());
            }
            cmd.creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .with_context(|| format!("launch {exe}"))?;
        }
        Ok(())
    }
}

/// Post `WM_CLOSE` to every top-level window owned by one of `pids`, asking those
/// processes to shut down cleanly (flushing any in-flight writes) before a caller
/// resorts to a hard kill. Best-effort: windowless/background processes are simply
/// left for the fallback `TerminateProcess`.
///
/// Uses raw `user32` FFI to avoid pulling in a heavyweight Windows-API crate.
#[cfg(windows)]
fn graceful_close(pids: &std::collections::HashSet<u32>) {
    use std::ffi::c_void;

    type Hwnd = *mut c_void;
    type Lparam = isize;
    type Wparam = usize;
    type Bool = i32;
    type Dword = u32;

    const WM_CLOSE: u32 = 0x0010;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(cb: extern "system" fn(Hwnd, Lparam) -> Bool, lparam: Lparam) -> Bool;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut Dword) -> Dword;
        fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: Wparam, lparam: Lparam) -> Bool;
    }

    // Passed by raw pointer through `EnumWindows`'s LPARAM to the callback.
    struct Ctx<'a> {
        targets: &'a std::collections::HashSet<u32>,
        hwnds: Vec<Hwnd>,
    }

    extern "system" fn enum_cb(hwnd: Hwnd, lparam: Lparam) -> Bool {
        // SAFETY: `lparam` is the `&mut Ctx` we handed to `EnumWindows` below, and
        // the callback only runs during that synchronous call.
        let ctx = unsafe { &mut *(lparam as *mut Ctx) };
        let mut pid: Dword = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if ctx.targets.contains(&pid) {
            ctx.hwnds.push(hwnd);
        }
        1 // TRUE: keep enumerating
    }

    let mut ctx = Ctx {
        targets: pids,
        hwnds: Vec::new(),
    };
    // SAFETY: FFI into user32; `enum_cb` matches the WNDENUMPROC signature and the
    // context pointer outlives the (synchronous) enumeration.
    unsafe {
        EnumWindows(enum_cb, &mut ctx as *mut Ctx as Lparam);
        for hwnd in ctx.hwnds {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
    }
}
