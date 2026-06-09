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
