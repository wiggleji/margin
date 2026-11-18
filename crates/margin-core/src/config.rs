//! User config (`config.json`, hand-edited) and app state (`state.json`).
//!
//! macOS: `~/Library/Application Support/Margin/`
//! other: `$XDG_CONFIG_HOME/margin/` or `~/.config/margin/`
//! Override with `MARGIN_CONFIG_DIR`.
//!
//! Secrets never live in these files. Tokens entered in Settings → Accounts
//! go to the macOS Keychain (services `margin-github` / `margin-jira`), or
//! elsewhere to `secrets.json` in the config folder, readable only by you.
//! Jira tokens come from, in order:
//!   1. the env var named by `token_env` (default `MARGIN_JIRA_TOKEN`)
//!   2. the saved token (Keychain / secrets.json)

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
    #[serde(default)]
    pub github: GithubConfig,
    /// Link-out for task keys when no task API is configured, e.g.
    /// `https://linear.app/acme/issue/{key}` or `https://acme.atlassian.net/browse/{key}`.
    #[serde(default)]
    pub task_url_template: Option<String>,
    /// Move detached commit notes to their rewritten commit when `git patch-id`
    /// finds exactly one match (default: true).
    #[serde(default)]
    pub auto_reattach: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GithubConfig {
    /// Set to true to never contact GitHub.
    #[serde(default)]
    pub disabled: bool,
    /// GitHub Enterprise Server hosts (github.com is always recognised).
    #[serde(default)]
    pub enterprise_hosts: Vec<String>,
    /// SSH host aliases from ~/.ssh/config, e.g. {"github-work": "github.com"}.
    #[serde(default)]
    pub host_aliases: std::collections::BTreeMap<String, String>,
    /// Override the GraphQL endpoint (testing / unusual proxies).
    #[serde(default)]
    pub api_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// Repositories open as tabs (restored on start), and the active one.
    #[serde(default)]
    pub open_tabs: Vec<String>,
    #[serde(default)]
    pub active_tab: usize,
    /// First launch (unix time): start of the Margin Pro trial.
    #[serde(default)]
    pub trial_started: Option<i64>,
    /// Command palette: recently run commands (`Group/Title`), newest first.
    #[serde(default)]
    pub recent_commands: Vec<String>,
    /// Notes panel: height chosen by dragging, collapsed to its tab row, and
    /// editing side by side with the preview ("Split").
    #[serde(default)]
    pub notes_height: Option<f32>,
    #[serde(default)]
    pub notes_collapsed: bool,
    #[serde(default)]
    pub notes_split: bool,
    /// Notes panel under the history (graph) instead of under the commit details.
    #[serde(default)]
    pub notes_under_history: bool,
    /// Diffs side by side instead of inline.
    #[serde(default)]
    pub diff_split: bool,
    /// Diffs ignore whitespace-only changes (git diff -w).
    #[serde(default)]
    pub diff_ignore_ws: bool,
    /// Which diffs start with their files expanded (Settings → Diffs):
    /// commits and staged files expanded, unstaged collapsed, by default.
    #[serde(default)]
    pub diff_collapse_commits: bool,
    #[serde(default)]
    pub diff_collapse_staged: bool,
    #[serde(default)]
    pub diff_expand_unstaged: bool,
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

/// Preferences edited in the Settings window (app-managed `settings.json`;
/// `config.json` stays hand-edited).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Fetch all remotes in the background (off by default: no surprise network traffic).
    #[serde(default)]
    pub auto_fetch: bool,
    #[serde(default = "default_fetch_minutes")]
    pub auto_fetch_minutes: u32,
    /// Cloud-synced folder for notes (iCloud Drive, Dropbox, Google Drive…).
    #[serde(default)]
    pub sync_folder: Option<String>,
    /// Repo ids (see sync::repo_id) with notes sync on, in the order enabled.
    /// Without Pro only the first one syncs.
    #[serde(default)]
    pub synced_repos: Vec<String>,
    /// Where task keys link to (Settings → Tasks): a site such as
    /// `https://acme.atlassian.net` (→ /browse/KEY) or a template with `{key}`.
    #[serde(default)]
    pub ticket_url: Option<String>,
    /// Per repository (keyed by repo id, see sync::repo_id): where its task
    /// keys come from. Missing = `TaskSource::Default`.
    #[serde(default)]
    pub repo_tasks: std::collections::BTreeMap<String, TaskSource>,
    /// Repos already asked "use GitHub Issues for tasks?" (never asked again).
    #[serde(default)]
    pub tasks_prompted: Vec<String>,
    /// Jira, from Settings → Accounts (wins over config.json's `jira`).
    #[serde(default)]
    pub jira: Option<JiraConfig>,
    /// GitHub Enterprise hosts added in Settings → Accounts (with config.json's).
    #[serde(default)]
    pub github_hosts: Vec<String>,
    /// Colour theme id (Settings → Appearance); `None` = Margin.
    #[serde(default)]
    pub theme: Option<String>,
    /// Code and notes text size in points (Settings → Appearance); `None` =
    /// config.json's `font_size`, else 13.
    #[serde(default)]
    pub font_size: Option<f32>,
    /// Interface text size (lists, toolbar, menus); `None` = 13.
    #[serde(default)]
    pub ui_font_size: Option<f32>,
    /// Interface and code fonts (Settings → Appearance); `None` = built-in.
    #[serde(default)]
    pub ui_font: Option<FontChoice>,
    #[serde(default)]
    pub code_font: Option<FontChoice>,
}

/// Where a repository's tasks come from (Settings → Tasks).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskSource {
    /// The app-wide setting: Jira from config.json, else the ticket link.
    #[default]
    Default,
    /// No task keys in this repository.
    Off,
    /// Keys like PROJ-123 linking to this site or `{key}` template.
    Link { url: String },
    /// Jira, through the API in config.json.
    Jira,
    /// GitHub Issues of the repository's GitHub remote (#123).
    Github,
}

/// An installed font: family name, and the file (and face in it) to load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontChoice {
    pub family: String,
    pub path: String,
    #[serde(default)]
    pub index: u32,
}

fn default_fetch_minutes() -> u32 {
    10
}

impl Default for Settings {
    fn default() -> Self {
        Settings { auto_fetch: false, auto_fetch_minutes: default_fetch_minutes(), sync_folder: None, synced_repos: vec![], ticket_url: None, theme: None, font_size: None, ui_font_size: None, ui_font: None, code_font: None, repo_tasks: Default::default(), tasks_prompted: vec![], jira: None, github_hosts: vec![] }
    }
}

/// config.json with what was set in Settings → Accounts on top (Jira, extra
/// GitHub Enterprise hosts): what the app and the MCP server both use, so an
/// account added in the app works for agents too.
pub fn with_accounts(mut cfg: Config, settings: &Settings) -> Config {
    if let Some(j) = &settings.jira {
        cfg.jira = Some(j.clone());
    }
    for h in &settings.github_hosts {
        if !cfg.github.enterprise_hosts.iter().any(|x| x.eq_ignore_ascii_case(h)) {
            cfg.github.enterprise_hosts.push(h.clone());
        }
    }
    cfg
}

/// `load_config()` + Settings → Accounts.
pub fn load_effective_config() -> Config {
    with_accounts(load_config(), &load_settings())
}

pub fn load_settings() -> Settings {
    std::fs::read_to_string(config_dir().join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) -> Result<()> {
    let d = config_dir();
    std::fs::create_dir_all(&d)?;
    std::fs::write(d.join("settings.json"), serde_json::to_string_pretty(s)?)?;
    Ok(())
}

pub fn save_state(s: &State) -> Result<()> {
    let d = config_dir();
    std::fs::create_dir_all(&d)?;
    std::fs::write(d.join("state.json"), serde_json::to_string_pretty(s)?)?;
    Ok(())
}

/// GitHub token lookup: `GITHUB_TOKEN`/`GH_TOKEN` → `gh auth token` (GitHub CLI,
/// common in enterprises) → macOS Keychain service `margin-github`.
pub fn github_token(host: &str) -> Option<String> {
    github_token_with_source(host).map(|(t, _)| t)
}

/// Same, with where it came from ("Margin", "GITHUB_TOKEN", "GitHub CLI", …).
/// A token saved in Settings → Accounts wins over the environment and gh.
pub fn github_token_with_source(host: &str) -> Option<(String, &'static str)> {
    let env = |v: &'static str| std::env::var(v).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).map(|t| (t, v));
    if let Some(t) = env("MARGIN_GITHUB_TOKEN") {
        return Some(t);
    }
    if let Some(t) = keychain_lookup("margin-github", Some(host)) {
        return Some((t, "saved in Margin"));
    }
    for v in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Some(t) = env(v) {
            return Some(t);
        }
    }
    // Apps launched from Finder don't get the shell PATH, so try Homebrew paths too.
    for gh in ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"] {
        let out = std::process::Command::new(gh)
            .args(["auth", "token", "--hostname", host])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(o) = out {
            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if o.status.success() && !t.is_empty() {
                return Some((t, "GitHub CLI"));
            }
            break; // gh exists but has no token for this host
        }
    }
    None
}

/// Save a token (Settings → Accounts): the macOS Keychain, elsewhere
/// `secrets.json` in the config folder (permissions 0600).
pub fn store_secret(service: &str, account: &str, value: &str) -> Result<()> {
    store_secret_impl(service, account, value.trim())
}

/// Forget a saved token.
pub fn delete_secret(service: &str, account: &str) -> Result<()> {
    delete_secret_impl(service, account)
}

#[cfg(target_os = "macos")]
fn store_secret_impl(service: &str, account: &str, value: &str) -> Result<()> {
    // -U: update an existing item
    let st = std::process::Command::new("/usr/bin/security").args(["add-generic-password", "-U", "-s", service, "-a", account, "-w", value]).status()?;
    if !st.success() {
        crate::bail!("couldn't save to the Keychain");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn delete_secret_impl(service: &str, account: &str) -> Result<()> {
    let _ = std::process::Command::new("/usr/bin/security").args(["delete-generic-password", "-s", service, "-a", account]).output();
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn secrets_file() -> PathBuf {
    config_dir().join("secrets.json")
}

#[cfg(not(target_os = "macos"))]
fn read_secrets() -> std::collections::BTreeMap<String, String> {
    std::fs::read_to_string(secrets_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn write_secrets(m: &std::collections::BTreeMap<String, String>) -> Result<()> {
    let p = secrets_file();
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(&p, serde_json::to_string_pretty(m)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn store_secret_impl(service: &str, account: &str, value: &str) -> Result<()> {
    let mut m = read_secrets();
    m.insert(format!("{service}/{account}"), value.to_string());
    write_secrets(&m)
}

#[cfg(not(target_os = "macos"))]
fn delete_secret_impl(service: &str, account: &str) -> Result<()> {
    let mut m = read_secrets();
    if m.remove(&format!("{service}/{account}")).is_some() {
        write_secrets(&m)?;
    }
    Ok(())
}

/// The Keychain / secrets.json account a Jira token is saved under.
pub fn jira_account(cfg: &JiraConfig) -> String {
    cfg.email.clone().filter(|e| !e.trim().is_empty()).unwrap_or_else(|| cfg.base_url.trim_end_matches('/').to_string())
}

pub fn jira_token(cfg: &JiraConfig) -> Option<String> {
    let env = cfg.token_env.as_deref().unwrap_or("MARGIN_JIRA_TOKEN");
    if let Ok(t) = std::env::var(env) {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    keychain_lookup(cfg.keychain_service.as_deref().unwrap_or("margin-jira"), Some(&jira_account(cfg)))
        .or_else(|| keychain_lookup(cfg.keychain_service.as_deref().unwrap_or("margin-jira"), cfg.email.as_deref()))
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
fn keychain_lookup(service: &str, account: Option<&str>) -> Option<String> {
    let m = read_secrets();
    match account {
        Some(a) => m.get(&format!("{service}/{a}")).cloned(),
        None => m.iter().find(|(k, _)| k.starts_with(&format!("{service}/"))).map(|(_, v)| v.clone()),
    }
}
