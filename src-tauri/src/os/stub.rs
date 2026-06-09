//! Placeholder host for non-Windows targets.
//!
//! This lets the project *compile* on Linux/macOS today. Implementing each
//! method (and adding per-OS platform defs) is the "expand later" work.

use super::Host;
use anyhow::{bail, Result};

pub struct StubHost;

impl Host for StubHost {
    fn expand_vars(&self, input: &str) -> String {
        // Best-effort %VAR% expansion via environment variables.
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
                    _ => {
                        out.push('%');
                        out.push_str(&name);
                        if closed {
                            out.push('%');
                        }
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn read_registry(&self, _key: &str, _value: &str) -> Result<Option<String>> {
        Ok(None) // no registry off-Windows
    }
    fn write_registry(&self, _key: &str, _value: &str, _data: &str) -> Result<()> {
        Ok(())
    }
    fn delete_registry_value(&self, _key: &str, _value: &str) -> Result<()> {
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

    fn launch(&self, _exe: &str, _args: &str, _elevated: bool) -> Result<()> {
        bail!("launching platforms is not implemented on this OS yet");
    }
}
