//! In-memory host used off-Windows (to keep the project compiling on Linux/macOS)
//! and as the controllable test double for engine tests.
//!
//! By default it behaves like a lightweight placeholder: `%VAR%` expansion falls
//! back to real environment variables, registry access is in-memory, and process
//! enumeration uses the real OS. Tests can override any of these — inject a
//! variable map, seed registry values, model a fixed set of "running" processes,
//! and make specific registry reads/writes fail — to drive the engine
//! deterministically without touching the real machine.
//!
//! State lives behind an `Arc`, so a test can clone a handle, hand one copy to the
//! [`Engine`](crate::switcher::engine::Engine) as a `Box<dyn Host>`, and still
//! inspect the other after a switch.

use super::Host;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Inner {
    /// Overrides for `%VAR%` expansion (checked before the real environment).
    vars: Mutex<HashMap<String, String>>,
    /// In-memory registry: `(key, value) -> data`.
    registry: Mutex<HashMap<(String, String), String>>,
    /// Modeled running processes (lowercased image names). `None` means "use the
    /// real OS" (runtime); `Some` means "use this set" (tests).
    running: Mutex<Option<HashSet<String>>>,
    /// Record of `launch` calls, for test assertions.
    launched: Mutex<Vec<(String, String)>>,
    /// If set, `read_registry` errors when reading this value name.
    fail_read_value: Mutex<Option<String>>,
    /// If set, `write_registry` errors when writing this exact data.
    fail_write_data: Mutex<Option<String>>,
}

#[derive(Clone, Default)]
pub struct StubHost {
    inner: Arc<Inner>,
}

impl StubHost {
    pub fn new() -> Self {
        StubHost::default()
    }

    /// Add a `%VAR%` override (builder style).
    #[cfg(test)]
    pub fn with_var(self, name: &str, value: &str) -> Self {
        self.inner
            .vars
            .lock()
            .unwrap()
            .insert(name.to_string(), value.to_string());
        self
    }

    /// Seed a registry value.
    #[cfg(test)]
    pub fn set_registry(&self, key: &str, value: &str, data: &str) {
        self.inner
            .registry
            .lock()
            .unwrap()
            .insert((key.to_string(), value.to_string()), data.to_string());
    }

    /// Read back a registry value (test assertions).
    #[cfg(test)]
    pub fn get_registry(&self, key: &str, value: &str) -> Option<String> {
        self.inner
            .registry
            .lock()
            .unwrap()
            .get(&(key.to_string(), value.to_string()))
            .cloned()
    }

    /// Make `read_registry` fail for a given value name (to exercise capture failure).
    #[cfg(test)]
    pub fn fail_registry_read(&self, value: &str) {
        *self.inner.fail_read_value.lock().unwrap() = Some(value.to_string());
    }

    /// Make `write_registry` fail when writing a given data string (to exercise
    /// a restore failure and its rollback).
    #[cfg(test)]
    pub fn fail_registry_write_data(&self, data: &str) {
        *self.inner.fail_write_data.lock().unwrap() = Some(data.to_string());
    }
}

impl Host for StubHost {
    fn expand_vars(&self, input: &str) -> String {
        let vars = self.inner.vars.lock().unwrap();
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
                let resolved = if closed {
                    vars.get(&name)
                        .cloned()
                        .or_else(|| std::env::var(&name).ok())
                } else {
                    None
                };
                match resolved {
                    Some(val) => out.push_str(&val),
                    None => {
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

    fn read_registry(&self, key: &str, value: &str) -> Result<Option<String>> {
        if self.inner.fail_read_value.lock().unwrap().as_deref() == Some(value) {
            return Err(anyhow!("stub: registry read of {value} failed"));
        }
        let key = self.expand_vars(key);
        Ok(self
            .inner
            .registry
            .lock()
            .unwrap()
            .get(&(key, value.to_string()))
            .cloned())
    }

    fn write_registry(&self, key: &str, value: &str, data: &str) -> Result<()> {
        if self.inner.fail_write_data.lock().unwrap().as_deref() == Some(data) {
            return Err(anyhow!("stub: registry write of {data} failed"));
        }
        let key = self.expand_vars(key);
        self.inner
            .registry
            .lock()
            .unwrap()
            .insert((key, value.to_string()), data.to_string());
        Ok(())
    }

    fn delete_registry_value(&self, key: &str, value: &str) -> Result<()> {
        let key = self.expand_vars(key);
        self.inner
            .registry
            .lock()
            .unwrap()
            .remove(&(key, value.to_string()));
        Ok(())
    }

    fn kill_processes(&self, exe_names: &[String]) -> Result<()> {
        if exe_names.is_empty() {
            return Ok(());
        }
        let targets: Vec<String> = exe_names.iter().map(|n| n.to_ascii_lowercase()).collect();
        {
            let mut running = self.inner.running.lock().unwrap();
            if let Some(set) = running.as_mut() {
                set.retain(|name| !targets.contains(name));
                return Ok(());
            }
        }
        // Runtime path: real process enumeration.
        use sysinfo::System;
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
        if exe_names.is_empty() {
            return false;
        }
        let targets: Vec<String> = exe_names.iter().map(|n| n.to_ascii_lowercase()).collect();
        {
            let running = self.inner.running.lock().unwrap();
            if let Some(set) = running.as_ref() {
                return targets.iter().any(|t| set.contains(t));
            }
        }
        use sysinfo::System;
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        sys.processes().values().any(|proc| {
            let name = proc.name().to_string_lossy().to_ascii_lowercase();
            targets.iter().any(|t| *t == name)
        })
    }

    fn launch(&self, exe: &str, args: &str, _elevated: bool) -> Result<()> {
        self.inner
            .launched
            .lock()
            .unwrap()
            .push((exe.to_string(), args.to_string()));
        Ok(())
    }
}
