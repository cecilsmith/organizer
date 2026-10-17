//! Account tokens. A pasted token lives in the macOS Keychain, never in the
//! SQLite file; a token imported from `gh` or `glab` isn't stored at all, since
//! those CLIs already hold it — it is re-read from them once per run.
//!
//! Either way a token is read once per run and kept in memory, so the Keychain
//! is touched at most once per account — each touch of an item this build
//! didn't create raises a macOS confirmation dialog.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use crate::error::{Error, Result};
use crate::model::{Account, Provider, TokenSource};

const SERVICE: &str = "com.nathanaelsmith.organizer.github";

#[derive(Default)]
pub struct Secrets {
    cache: RwLock<HashMap<String, String>>,
    /// Held while a token is loaded, so that concurrent syncs of the same
    /// account raise one Keychain dialog between them rather than one each.
    loading: Mutex<()>,
}

impl Secrets {
    /// The token for `account`, loaded once per run and cached in memory.
    pub fn token(&self, account: &Account) -> Result<String> {
        if let Some(t) = self.cached(&account.id) {
            return Ok(t);
        }
        let _loading = self.loading.lock().unwrap_or_else(|e| e.into_inner());
        // Another task may have loaded it while we waited.
        if let Some(t) = self.cached(&account.id) {
            return Ok(t);
        }
        let token = match account.token_source {
            TokenSource::Cli => match account.provider {
                Provider::Github => gh_cli_token(&account.host, &account.login)?,
                Provider::Gitlab => glab_cli_token(&account.host)?,
            },
            TokenSource::Keychain => keychain::get(&account.id)?.ok_or_else(|| {
                Error::msg(format!("No token stored for {}. Re-link the account.", account.id))
            })?,
        };
        self.remember(&account.id, &token);
        Ok(token)
    }

    fn cached(&self, account_id: &str) -> Option<String> {
        self.cache.read().unwrap_or_else(|e| e.into_inner()).get(account_id).cloned()
    }

    /// Keeps a token for this run without storing it anywhere.
    pub fn remember(&self, account_id: &str, token: &str) {
        self.cache.write().unwrap_or_else(|e| e.into_inner()).insert(account_id.into(), token.into());
    }

    pub fn set(&self, account_id: &str, token: &str) -> Result<()> {
        keychain::set(account_id, token)?;
        self.remember(account_id, token);
        Ok(())
    }

    pub fn delete(&self, account_id: &str) -> Result<()> {
        self.cache.write().unwrap_or_else(|e| e.into_inner()).remove(account_id);
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

/// Finder-launched apps get a minimal PATH, so check the usual install
/// locations before falling back to the bare name.
fn cli_path(name: &str) -> std::path::PathBuf {
    ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
        .iter()
        .map(|dir| std::path::Path::new(dir).join(name))
        .find(|p| p.exists())
        .unwrap_or_else(|| name.into())
}

/// `(stdout, stderr)` from a CLI, whatever its exit status.
fn run(bin: &std::path::Path, args: &[&str]) -> std::io::Result<(String, String)> {
    let out = std::process::Command::new(bin).args(args).output()?;
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// Asks `gh` for a stored token.
pub fn gh_cli_token(host: &str, user: &str) -> Result<String> {
    let out = std::process::Command::new(cli_path("gh"))
        .args(["auth", "token", "--hostname", host, "--user", user])
        .output()
        .map_err(|e| Error::msg(format!("Couldn't run the GitHub CLI: {e}")))?;
    if !out.status.success() {
        return Err(Error::msg(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ---- GitLab CLI (`glab`) --------------------------------------------------
//
// `glab` allows one account per host, so a host is enough to identify it; the
// login comes back from the API when the account is linked. The token may sit
// in the OS keyring, an environment variable or the config file, so ask `glab`
// for it rather than reading the file — and fall back to the file when the
// binary isn't there to ask.

/// Hosts the GitLab CLI (`glab`) is logged into.
pub fn glab_cli_accounts() -> Vec<String> {
    let hosts = glab_status_hosts();
    if !hosts.is_empty() {
        return hosts;
    }
    glab_config_hosts().into_iter().map(|(host, _)| host).collect()
}

/// `glab auth status --all --output json`, which lists every configured host.
/// Older versions have no `--output` flag; then this comes back empty.
fn glab_status_hosts() -> Vec<String> {
    let Ok((stdout, _)) = run(&cli_path("glab"), &["auth", "status", "--all", "--output", "json"]) else {
        return vec![];
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout.trim()) else {
        return vec![];
    };
    value["hosts"]
        .as_array()
        .map(|hosts| hosts.iter().filter_map(|h| h["host"].as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// Asks `glab` for the token it holds for `host`.
pub fn glab_cli_token(host: &str) -> Result<String> {
    let glab = cli_path("glab");
    // `auth status` prints its lines to stderr, and covers every place glab
    // keeps a token (keyring, environment, config file).
    let status = run(&glab, &["auth", "status", "--hostname", host, "--show-token"]);
    if let Ok((stdout, stderr)) = &status
        && let Some(token) = parse_glab_token(&format!("{stdout}\n{stderr}"))
    {
        return Ok(token);
    }
    // Older versions without `--show-token`.
    if let Ok((stdout, _)) = run(&glab, &["config", "get", "token", "--host", host]) {
        let token = stdout.trim();
        if !token.is_empty() && !token.contains(char::is_whitespace) {
            return Ok(token.into());
        }
    }
    // No usable `glab` binary — read its config file directly.
    if let Some(token) = glab_config_hosts().into_iter().find(|(h, _)| h == host).and_then(|(_, t)| t) {
        return Ok(token);
    }
    Err(Error::msg(match status {
        Ok((_, stderr)) if !stderr.trim().is_empty() => stderr.trim().to_string(),
        Ok(_) => format!("`glab` has no token for {host}. Run `glab auth login --hostname {host}`."),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "Couldn't find the `glab` CLI — install it, or paste a token instead.".to_string()
        }
        Err(e) => format!("Couldn't run the GitLab CLI: {e}"),
    }))
}

/// Pulls the token out of `glab auth status --show-token` output, whose line
/// reads `✓ Token found in configuration file (plaintext): glpat-…` (older
/// versions printed `✓ Token: glpat-…`). Without `--show-token` the value is
/// masked with asterisks, and the scopes line is a near miss, so skip both.
fn parse_glab_token(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("token") || lower.contains("scope") {
            return None;
        }
        let value = line.rsplit_once(':')?.1.trim();
        let plausible = value.chars().count() >= 8
            && !value.contains('*')
            && !value.contains(char::is_whitespace);
        plausible.then(|| value.to_string())
    })
}

/// glab's `config.yml`, in the order glab itself looks for it.
fn glab_config_path() -> Option<std::path::PathBuf> {
    let dirs = [
        std::env::var_os("GLAB_CONFIG_DIR").map(std::path::PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(|d| std::path::PathBuf::from(d).join("glab-cli")),
        dirs::home_dir().map(|h| h.join(".config/glab-cli")),
        dirs::home_dir().map(|h| h.join("Library/Application Support/glab-cli")),
    ];
    dirs.into_iter().flatten().map(|d| d.join("config.yml")).find(|p| p.exists())
}

fn glab_config_hosts() -> Vec<(String, Option<String>)> {
    glab_config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| parse_glab_hosts(&text))
        .unwrap_or_default()
}

/// Minimal reader for glab's `config.yml`: a top-level `hosts:` map keyed by
/// hostname, each with its own settings — including `token` when glab keeps it
/// in the file rather than the OS keyring.
fn parse_glab_hosts(text: &str) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    // Indent of the `hosts:` key's children, once we've seen the first one.
    let mut in_hosts = false;
    let mut host_indent: Option<usize> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            // Any other top-level key ends the `hosts:` block.
            in_hosts = trimmed == "hosts:";
            host_indent = None;
            continue;
        }
        if !in_hosts {
            continue;
        }
        if *host_indent.get_or_insert(indent) == indent {
            if let Some(name) = trimmed.strip_suffix(':') {
                out.push((name.trim().into(), None));
            }
        } else if let Some(("token", value)) = trimmed.split_once(':').map(|(k, v)| (k.trim(), v)) {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if let (Some(host), false) = (out.last_mut(), value.is_empty()) {
                host.1 = Some(value.into());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{parse_gh_hosts, parse_glab_hosts, parse_glab_token};

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

    #[test]
    fn parses_glab_config_hosts_and_tokens() {
        let yml = concat!(
            "git_protocol: ssh\n",
            "check_update: true\n",
            "hosts:\n",
            "    gitlab.com:\n",
            "        api_protocol: https\n",
            "        api_host: gitlab.com\n",
            "        token: glpat-aaa\n",
            "        user: alice\n",
            "    git.corp.com:\n",
            "        token: \"glpat-bbb\"\n",
            "    keyring.example.com:\n",
            "        api_protocol: https\n",
            "editor:\n",
            "    other: value\n",
        );
        assert_eq!(
            parse_glab_hosts(yml),
            vec![
                ("gitlab.com".to_string(), Some("glpat-aaa".to_string())),
                ("git.corp.com".to_string(), Some("glpat-bbb".to_string())),
                ("keyring.example.com".to_string(), None),
            ]
        );
    }

    #[test]
    fn ignores_a_config_without_hosts() {
        assert!(parse_glab_hosts("git_protocol: ssh\neditor:\n").is_empty());
    }

    #[test]
    fn reads_the_token_out_of_auth_status() {
        let text = concat!(
            "gitlab.com\n",
            "  ✓ Logged in to gitlab.com as alice (GITLAB_TOKEN)\n",
            "  ✓ REST API Endpoint: https://gitlab.com/api/v4/\n",
            "  ✓ Token found in configuration file (plaintext): glpat-aaaaaaaaaaaa\n",
            "  ✓ Token Scopes: api, read_user\n",
        );
        assert_eq!(parse_glab_token(text).unwrap(), "glpat-aaaaaaaaaaaa");
        // An older release's wording.
        assert_eq!(parse_glab_token("  ✓ Token: glpat-bbbbbbbbbbbb\n").unwrap(), "glpat-bbbbbbbbbbbb");
        // Masked (no `--show-token`), or missing altogether.
        assert!(parse_glab_token("  ✓ Token: **************\n").is_none());
        assert!(parse_glab_token("  x No token found for gitlab.com\n").is_none());
        assert!(parse_glab_token("  ✓ Token Scopes: api, read_user\n").is_none());
    }
}
