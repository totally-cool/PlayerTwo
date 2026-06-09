//! Steam-specific account switching.
//!
//! Steam keeps ALL accounts in one shared `config/loginusers.vdf` and decides
//! who to auto-login via that file's `MostRecent` / `AllowAutoLogin` flags plus
//! the `AutoLoginUser` registry value. So, unlike the generic file-swap engine,
//! we don't copy files — we read the account list straight from Steam and switch
//! by flipping those flags (Steam must be closed during the switch).
//!
//! This relies on Steam's "remember password" token for each account already
//! existing (i.e. you logged in once with "remember me" ticked). Accounts that
//! Steam can't silently resume will land on the login screen pre-filled.

use crate::os::Host;
use crate::switcher::model::Account;
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

const STEAM_REG: &str = "HKCU\\Software\\Valve\\Steam";

pub fn steam_path(host: &dyn Host) -> Option<PathBuf> {
    host.read_registry(STEAM_REG, "SteamPath")
        .ok()
        .flatten()
        .map(|p| PathBuf::from(p.replace('/', "\\")))
}

fn loginusers_path(host: &dyn Host) -> Option<PathBuf> {
    steam_path(host).map(|p| p.join("config").join("loginusers.vdf"))
}

/// One `"<steamid>" { ... }` block; key order is preserved so we round-trip
/// unknown fields untouched.
struct UserBlock {
    steamid: String,
    kvs: Vec<(String, String)>,
}

impl UserBlock {
    fn get(&self, key: &str) -> Option<&str> {
        self.kvs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
    fn set(&mut self, key: &str, val: &str) {
        if let Some(kv) = self.kvs.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            kv.1 = val.to_string();
        } else {
            self.kvs.push((key.to_string(), val.to_string()));
        }
    }
}

enum Tok {
    Str(String),
    Open,
    Close,
}

/// Tokenize VDF text into quoted strings and braces (whitespace ignored).
fn tokenize(text: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    match n {
                        '\\' => {
                            if let Some(&e) = chars.peek() {
                                chars.next();
                                s.push(e);
                            }
                        }
                        '"' => break,
                        _ => s.push(n),
                    }
                }
                toks.push(Tok::Str(s));
            }
            '{' => toks.push(Tok::Open),
            '}' => toks.push(Tok::Close),
            _ => {}
        }
    }
    toks
}

/// Parse the user blocks out of loginusers.vdf.
fn parse(text: &str) -> Vec<UserBlock> {
    let toks = tokenize(text);
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Open => {
                depth += 1;
                i += 1;
            }
            Tok::Close => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            Tok::Str(s) => {
                // At depth 1, a string followed by `{` is a steamid block.
                if depth == 1 && matches!(toks.get(i + 1), Some(Tok::Open)) {
                    let steamid = s.clone();
                    i += 2; // consume the id and its opening brace
                    let mut kvs = Vec::new();
                    while i < toks.len() {
                        match &toks[i] {
                            Tok::Close => {
                                i += 1;
                                break;
                            }
                            Tok::Str(k) => {
                                if let Some(Tok::Str(v)) = toks.get(i + 1) {
                                    kvs.push((k.clone(), v.clone()));
                                    i += 2;
                                } else {
                                    i += 1;
                                }
                            }
                            Tok::Open => i += 1,
                        }
                    }
                    blocks.push(UserBlock { steamid, kvs });
                } else {
                    i += 1;
                }
            }
        }
    }
    blocks
}

/// Serialize blocks back to Steam's tab-indented VDF format.
fn serialize(blocks: &[UserBlock]) -> String {
    let mut out = String::from("\"users\"\n{\n");
    for b in blocks {
        out.push_str(&format!("\t\"{}\"\n\t{{\n", b.steamid));
        for (k, v) in &b.kvs {
            out.push_str(&format!("\t\t\"{}\"\t\t\"{}\"\n", k, v));
        }
        out.push_str("\t}\n");
    }
    out.push_str("}\n");
    out
}

/// List accounts Steam knows about (SteamID64 + persona/account name).
pub fn list_accounts(host: &dyn Host) -> Vec<Account> {
    let Some(path) = loginusers_path(host) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse(&text)
        .into_iter()
        .map(|b| {
            let persona = b.get("PersonaName").unwrap_or("").to_string();
            let account = b.get("AccountName").unwrap_or("").to_string();
            Account {
                display_name: if persona.is_empty() { account.clone() } else { persona },
                note: if account.is_empty() { None } else { Some(account) },
                image: None,
                id: b.steamid,
            }
        })
        .collect()
}

/// The SteamID64 of the account Steam will currently auto-login.
pub fn current(host: &dyn Host) -> Option<String> {
    let auto = host
        .read_registry(STEAM_REG, "AutoLoginUser")
        .ok()
        .flatten()
        .unwrap_or_default()
        .to_lowercase();
    let path = loginusers_path(host)?;
    let text = std::fs::read_to_string(path).ok()?;
    let blocks = parse(&text);

    if !auto.is_empty() {
        if let Some(b) = blocks
            .iter()
            .find(|b| b.get("AccountName").map(|n| n.to_lowercase()).as_deref() == Some(&auto))
        {
            return Some(b.steamid.clone());
        }
    }
    blocks
        .iter()
        .find(|b| b.get("MostRecent") == Some("1"))
        .map(|b| b.steamid.clone())
}

/// Make `steamid` the account Steam auto-logs into next launch.
pub fn switch(host: &dyn Host, steamid: &str) -> Result<()> {
    let path = loginusers_path(host).ok_or_else(|| anyhow!("Steam install not found"))?;
    let text = std::fs::read_to_string(&path).context("read loginusers.vdf")?;
    let mut blocks = parse(&text);

    let account_name = blocks
        .iter()
        .find(|b| b.steamid == steamid)
        .and_then(|b| b.get("AccountName"))
        .ok_or_else(|| anyhow!("account not found in loginusers.vdf"))?
        .to_string();

    for b in blocks.iter_mut() {
        let is_target = b.steamid == steamid;
        b.set("MostRecent", if is_target { "1" } else { "0" });
        if is_target {
            b.set("AllowAutoLogin", "1");
        }
    }

    std::fs::write(&path, serialize(&blocks)).context("write loginusers.vdf")?;
    host.write_registry(STEAM_REG, "AutoLoginUser", &account_name)?;
    tracing::info!(steamid, account = %account_name, "steam switch: flipped flags + AutoLoginUser");
    Ok(())
}
