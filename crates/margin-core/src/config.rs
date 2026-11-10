//! User config (`config.json`, hand-edited) and app state (`state.json`).
//!
//! macOS: `~/Library/Application Support/Margin/`
//! other: `$XDG_CONFIG_HOME/margin/` or `~/.config/margin/`
//! Override with `MARGIN_CONFIG_DIR`.
//!
//! Secrets never live in these files. Jira tokens come from, in order:
//!   1. the env var named by `token_env` (default `MARGIN_JIRA_TOKEN`)
//!   2. macOS Keychain: `security add-generic-password -s margin-jira -a <account> -w`

use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub jira: Option<JiraConfig>,
    /// Name used for notes appended from the GUI (default: git user.name).
    #[serde(default)]
    pub note_author: Option<String>,
    /// UI font size in points.
    #[serde(default)]
    pub font_size: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JiraConfig {
    /// e.g. https://acme.atlassian.net or https://jira.corp.example.com
    pub base_url: String,
    #[serde(default)]
    pub flavor: JiraFlavor,
    /// Required for Cloud (Basic auth). Also used as Keychain account.
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub keychain_service: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JiraFlavor {
    #[default]
    Cloud,
    DataCenter,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub recent_repos: Vec<String>,
}

pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("MARGIN_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        return home.join("Library/Application Support/Margin");
    }
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"))
        .join("margin")
}

pub fn load_config() -> Config {
    let p = config_dir().join("config.json");
    match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            eprintln!("margin: ignoring invalid {}: {e}", p.display());
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

pub fn load_state() -> State {
    std::fs::read_to_string(config_dir().join("state.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_state(s: &State) -> Result<()> {
    let d = config_dir();
    std::fs::create_dir_all(&d)?;
    std::fs::write(d.join("state.json"), serde_json::to_string_pretty(s)?)?;
    Ok(())
}

pub fn jira_token(cfg: &JiraConfig) -> Option<String> {
    let env = cfg.token_env.as_deref().unwrap_or("MARGIN_JIRA_TOKEN");
    if let Ok(t) = std::env::var(env) {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    keychain_lookup(
        cfg.keychain_service.as_deref().unwrap_or("margin-jira"),
        cfg.email.as_deref(),
    )
}

#[cfg(target_os = "macos")]
fn keychain_lookup(service: &str, account: Option<&str>) -> Option<String> {
    let mut c = std::process::Command::new("/usr/bin/security");
    c.args(["find-generic-password", "-s", service, "-w"]);
    if let Some(a) = account {
        c.args(["-a", a]);
    }
    let out = c.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn keychain_lookup(_service: &str, _account: Option<&str>) -> Option<String> {
    None
}
