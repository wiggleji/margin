//! Task-tracker integration.
//!
//! `TaskProvider` is the seam for Jira / Linear / GitHub Issues / GitLab /
//! Azure Boards. Jira ships first, supporting both:
//!   * Jira Cloud        – REST v3, Basic auth (email + API token)
//!   * Jira Data Center  – REST v2, Bearer auth (Personal Access Token)
//!
//! HTTP is blocking `ureq` (no async runtime → small binary); call from a
//! worker thread. TLS uses the OS trust store (rustls-platform-verifier), so
//! corporate TLS-inspection CAs installed in the macOS Keychain just work,
//! and `HTTPS_PROXY`/`NO_PROXY` are honoured.

use crate::config::{JiraConfig, JiraFlavor};
use crate::{Error, Result, bail};
use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Default)]
pub struct Task {
    pub key: String,
    pub summary: String,
    pub status: String,
    /// "new" | "indeterminate" | "done" (Jira status category)
    pub status_category: String,
    pub assignee: Option<String>,
    pub issue_type: String,
    pub priority: Option<String>,
    pub url: String,
    pub description: String,
    pub updated: String,
}

pub trait TaskProvider: Send + Sync {
    fn name(&self) -> &str;
    fn get(&self, key: &str) -> Result<Task>;
    /// Provider-native query (JQL for Jira).
    fn search(&self, query: &str, limit: usize) -> Result<Vec<Task>>;
    fn my_open(&self, limit: usize) -> Result<Vec<Task>>;
    fn browse_url(&self, key: &str) -> String;
}

/// Extract Jira-style keys (`ABC-123`) from branch names / commit messages.
pub fn extract_keys(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut keys = Vec::new();
    let mut i = 0;
    while i < b.len() {
        // key must start at a word boundary with an uppercase letter
        let boundary = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        if boundary && b[i].is_ascii_uppercase() {
            let start = i;
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_uppercase() || b[j].is_ascii_digit() || b[j] == b'_') {
                j += 1;
            }
            if j < b.len() && b[j] == b'-' && j - start >= 2 {
                let mut k = j + 1;
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                let end_ok = k == b.len() || !b[k].is_ascii_alphanumeric();
                if k > j + 1 && end_ok {
                    let key = &s[start..k];
                    if !keys.iter().any(|x| x == key) {
                        keys.push(key.to_string());
                    }
                    i = k;
                    continue;
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    // Lowercase branch names like `feature/proj-123-x` are common too.
    if keys.is_empty() && s.chars().any(|c| c.is_ascii_lowercase()) {
        let upper = s.to_ascii_uppercase();
        if upper != s {
            return extract_keys(&upper);
        }
    }
    keys
}

pub fn provider_from_config(cfg: &crate::config::Config) -> Option<Box<dyn TaskProvider>> {
    let j = cfg.jira.as_ref()?;
    match Jira::new(j) {
        Ok(p) => Some(Box::new(p)),
        Err(e) => {
            eprintln!("margin: jira disabled: {e}");
            None
        }
    }
}

pub struct Jira {
    base: String,
    flavor: JiraFlavor,
    auth: String,
    agent: ureq::Agent,
}

const FIELDS: &str = "summary,status,assignee,issuetype,priority,description,updated";

impl Jira {
    pub fn new(cfg: &JiraConfig) -> Result<Jira> {
        let token = crate::config::jira_token(cfg)
            .ok_or_else(|| Error("no Jira token (set MARGIN_JIRA_TOKEN or add it to the macOS Keychain)".into()))?;
        let auth = match (&cfg.flavor, &cfg.email) {
            (JiraFlavor::Cloud, Some(email)) => {
                format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}")))
            }
            (JiraFlavor::Cloud, None) => bail!("Jira Cloud needs `email` in config"),
            (JiraFlavor::DataCenter, _) => format!("Bearer {token}"),
        };
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .tls_config(tls)
            .user_agent("margin-git-client")
            .build()
            .into();
        Ok(Jira { base: cfg.base_url.trim_end_matches('/').to_string(), flavor: cfg.flavor.clone(), auth, agent })
    }

    fn api(&self) -> &'static str {
        match self.flavor {
            JiraFlavor::Cloud => "/rest/api/3",
            JiraFlavor::DataCenter => "/rest/api/2",
        }
    }

    fn get_json(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let url = format!("{}{}{}", self.base, self.api(), path);
        let mut req = self.agent.get(&url).header("Authorization", &self.auth).header("Accept", "application/json");
        for (k, v) in query {
            req = req.query(k, v);
        }
        let mut resp = req.call().map_err(|e| match e {
            ureq::Error::StatusCode(401) => Error("Jira: 401 unauthorized — check token/email".into()),
            ureq::Error::StatusCode(404) => Error("Jira: not found (or no permission)".into()),
            other => Error(format!("Jira: {other}")),
        })?;
        resp.body_mut().read_json::<Value>().map_err(|e| Error(format!("Jira: bad response: {e}")))
    }

    fn to_task(&self, v: &Value) -> Task {
        let f = &v["fields"];
        let s = |p: &str| f.pointer(p).and_then(Value::as_str).map(String::from);
        let key = v["key"].as_str().unwrap_or_default().to_string();
        let description = match &f["description"] {
            Value::String(s) => s.clone(),
            Value::Object(_) => {
                let mut out = String::new();
                adf_to_text(&f["description"], &mut out);
                out.trim().to_string()
            }
            _ => String::new(),
        };
        Task {
            url: self.browse_url(&key),
            key,
            summary: s("/summary").unwrap_or_default(),
            status: s("/status/name").unwrap_or_default(),
            status_category: s("/status/statusCategory/key").unwrap_or_default(),
            assignee: s("/assignee/displayName"),
            issue_type: s("/issuetype/name").unwrap_or_default(),
            priority: s("/priority/name"),
            description,
            updated: s("/updated").unwrap_or_default(),
        }
    }
}

impl TaskProvider for Jira {
    fn name(&self) -> &str {
        "Jira"
    }

    fn get(&self, key: &str) -> Result<Task> {
        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            bail!("invalid issue key");
        }
        let v = self.get_json(&format!("/issue/{key}"), &[("fields", FIELDS)])?;
        Ok(self.to_task(&v))
    }

    fn search(&self, jql: &str, limit: usize) -> Result<Vec<Task>> {
        let max = limit.min(100).to_string();
        // Cloud moved search to /search/jql (old /search is deprecated there).
        let path = match self.flavor {
            JiraFlavor::Cloud => "/search/jql",
            JiraFlavor::DataCenter => "/search",
        };
        let v = self.get_json(path, &[("jql", jql), ("maxResults", &max), ("fields", FIELDS)])?;
        Ok(v["issues"].as_array().map(|a| a.iter().map(|i| self.to_task(i)).collect()).unwrap_or_default())
    }

    fn my_open(&self, limit: usize) -> Result<Vec<Task>> {
        self.search("assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC", limit)
    }

    fn browse_url(&self, key: &str) -> String {
        format!("{}/browse/{key}", self.base)
    }
}

/// Flatten Atlassian Document Format into readable plain text / light Markdown.
fn adf_to_text(node: &Value, out: &mut String) {
    let ty = node["type"].as_str().unwrap_or("");
    match ty {
        "text" => out.push_str(node["text"].as_str().unwrap_or("")),
        "hardBreak" => out.push('\n'),
        "mention" => out.push_str(node["attrs"]["text"].as_str().unwrap_or("@?")),
        "inlineCard" => out.push_str(node["attrs"]["url"].as_str().unwrap_or("")),
        "mediaSingle" | "mediaGroup" | "media" => out.push_str("[attachment]"),
        _ => {}
    }
    if ty == "listItem" {
        out.push_str("- ");
    }
    if ty == "codeBlock" {
        out.push_str("```\n");
    }
    if let Some(children) = node["content"].as_array() {
        for c in children {
            adf_to_text(c, out);
        }
    }
    match ty {
        "codeBlock" => out.push_str("\n```\n\n"),
        "paragraph" | "heading" | "blockquote" | "rule" => out.push_str("\n\n"),
        "listItem" => {
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(extract_keys("feature/PROJ-123-login"), vec!["PROJ-123"]);
        assert_eq!(extract_keys("fix(ABC-1): also XY2-77, not a-1 or AB-x"), vec!["ABC-1", "XY2-77"]);
        assert_eq!(extract_keys("bugfix/ops-42_crash"), vec!["OPS-42"]);
        assert!(extract_keys("main").is_empty());
        assert!(extract_keys("UTF-8 is fine").contains(&"UTF-8".to_string())); // known false positive
    }

    #[test]
    fn adf() {
        let v: Value = serde_json::from_str(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]},
            {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]}]}]}"#,
        )
        .unwrap();
        let mut s = String::new();
        adf_to_text(&v, &mut s);
        assert!(s.starts_with("Hello\n\n- one"));
    }
}
