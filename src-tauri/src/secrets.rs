//! GitHub tokens live in the macOS Keychain, never in the SQLite file.
//! Reads are cached in memory so the Keychain is only touched once per account.

use std::collections::HashMap;
use std::sync::RwLock;

use crate::error::{Error, Result};

const SERVICE: &str = "com.nathanaelsmith.organizer.github";

#[derive(Default)]
pub struct Secrets {
    cache: RwLock<HashMap<String, String>>,
}

impl Secrets {
    pub fn get(&self, account_id: &str) -> Result<String> {
        if let Some(t) = self.cache.read().unwrap().get(account_id) {
            return Ok(t.clone());
        }
        let token = keychain::get(account_id)?
            .ok_or_else(|| Error::msg(format!("No token stored for {account_id}. Re-link the account.")))?;
        self.cache.write().unwrap().insert(account_id.into(), token.clone());
        Ok(token)
    }

    pub fn set(&self, account_id: &str, token: &str) -> Result<()> {
        keychain::set(account_id, token)?;
        self.cache.write().unwrap().insert(account_id.into(), token.into());
        Ok(())
    }

    pub fn delete(&self, account_id: &str) -> Result<()> {
        self.cache.write().unwrap().remove(account_id);
        keychain::delete(account_id)
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use super::*;
    use security_framework::passwords;

    // errSecItemNotFound
    const NOT_FOUND: i32 = -25300;

    pub fn get(account: &str) -> Result<Option<String>> {
        match passwords::get_generic_password(SERVICE, account) {
            Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(Error::msg(format!("Keychain read failed: {e}"))),
        }
    }

    pub fn set(account: &str, token: &str) -> Result<()> {
        passwords::set_generic_password(SERVICE, account, token.as_bytes())
            .map_err(|e| Error::msg(format!("Keychain write failed: {e}")))
    }

    pub fn delete(account: &str) -> Result<()> {
        match passwords::delete_generic_password(SERVICE, account) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == NOT_FOUND => Ok(()),
            Err(e) => Err(Error::msg(format!("Keychain delete failed: {e}"))),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    use super::*;

    pub fn get(_: &str) -> Result<Option<String>> {
        Err(Error::msg("secure token storage is only implemented on macOS"))
    }
    pub fn set(_: &str, _: &str) -> Result<()> {
        Err(Error::msg("secure token storage is only implemented on macOS"))
    }
    pub fn delete(_: &str) -> Result<()> {
        Ok(())
    }
}

/// Accounts the GitHub CLI (`gh`) is logged into, as `(host, user)`.
pub fn gh_cli_accounts() -> Vec<(String, String)> {
    let dir = std::env::var_os("GH_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".config/gh")));
    let Some(text) = dir.and_then(|d| std::fs::read_to_string(d.join("hosts.yml")).ok()) else {
        return vec![];
    };
    parse_gh_hosts(&text)
}

/// Minimal reader for gh's `hosts.yml`: top-level keys are hosts, and each
/// host has a `users:` map whose keys are logins.
fn parse_gh_hosts(text: &str) -> Vec<(String, String)> {
    struct Host {
        name: String,
        users: Vec<String>,
        active: Option<String>,
    }
    let mut hosts: Vec<Host> = Vec::new();
    // Indent of the `users:` key, then of its children once we see the first one.
    let mut users_indent: Option<usize> = None;
    let mut login_indent: Option<usize> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            if let Some(name) = trimmed.strip_suffix(':') {
                hosts.push(Host { name: name.into(), users: vec![], active: None });
            }
            users_indent = None;
            login_indent = None;
            continue;
        }
        let Some(host) = hosts.last_mut() else { continue };
        if let Some(ui) = users_indent {
            if indent > ui {
                if *login_indent.get_or_insert(indent) == indent {
                    let login = trimmed.trim_end_matches(':').split(':').next().unwrap_or("");
                    if !login.is_empty() {
                        host.users.push(login.into());
                    }
                }
                continue;
            }
            users_indent = None;
            login_indent = None;
        }
        if trimmed == "users:" {
            users_indent = Some(indent);
        } else if let Some(u) = trimmed.strip_prefix("user:") {
            host.active = Some(u.trim().into());
        }
    }

    hosts
        .into_iter()
        .flat_map(|h| {
            let users = if h.users.is_empty() { h.active.into_iter().collect() } else { h.users };
            users.into_iter().map(move |u| (h.name.clone(), u))
        })
        .collect()
}

/// Asks `gh` for a stored token. Finder-launched apps get a minimal PATH, so
/// check the usual install locations first.
pub fn gh_cli_token(host: &str, user: &str) -> Result<String> {
    let candidates = ["/opt/homebrew/bin/gh", "/usr/local/bin/gh", "/usr/bin/gh", "gh"];
    let gh = candidates
        .iter()
        .find(|p| !p.starts_with('/') || std::path::Path::new(p).exists())
        .unwrap();
    let out = std::process::Command::new(gh)
        .args(["auth", "token", "--hostname", host, "--user", user])
        .output()
        .map_err(|e| Error::msg(format!("Couldn't run the GitHub CLI: {e}")))?;
    if !out.status.success() {
        return Err(Error::msg(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_gh_hosts;

    #[test]
    fn parses_multi_account_hosts_file() {
        let yml = "github.com:\n    users:\n        alice:\n        bob:\n            oauth_token: x\n    git_protocol: https\n    user: alice\nghe.corp.com:\n    user: carol\n    oauth_token: y\n";
        assert_eq!(
            parse_gh_hosts(yml),
            vec![
                ("github.com".into(), "alice".into()),
                ("github.com".into(), "bob".into()),
                ("ghe.corp.com".into(), "carol".into()),
            ]
        );
    }
}
