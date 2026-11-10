//! GitHub pull-request status for branches: merge readiness, CI, comments.
//! Display-only — for anything else the user clicks through to GitHub.
//!
//! One GraphQL request per refresh covers every local branch (aliased
//! fields), and one more loads the comments/checks of the branch in focus.
//! Works with github.com and GitHub Enterprise Server (`/api/graphql`).
//!
//! Token: `GH_TOKEN`/`GITHUB_TOKEN` → `gh auth token` → Keychain (see config).

use crate::config::Config;
use crate::git::Repo;
use crate::{Error, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRepo {
    pub host: String,
    pub owner: String,
    pub name: String,
}

/// Parse `git@host:o/r.git`, `ssh://git@host:22/o/r`, `https://host/o/r.git`.
pub fn parse_remote(url: &str) -> Option<RemoteRepo> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.split_once("://").map(|(_, r)| r) {
        let (auth_host, path) = rest.split_once('/')?;
        let host = auth_host.rsplit('@').next()?;
        let host = host.split(':').next()?; // drop port
        (host, path)
    } else {
        // scp-like: [user@]host:owner/repo
        let (auth_host, path) = url.split_once(':')?;
        (auth_host.rsplit('@').next()?, path)
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.rsplitn(2, '/');
    let name = parts.next()?.to_string();
    let owner = parts.next()?.rsplit('/').next()?.to_string();
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(RemoteRepo { host: host.to_ascii_lowercase(), owner, name })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CiState {
    None,
    Pending,
    Success,
    Failure,
}

impl CiState {
    fn from_rollup(s: Option<&str>) -> CiState {
        match s {
            Some("SUCCESS") => CiState::Success,
            Some("FAILURE") | Some("ERROR") => CiState::Failure,
            Some("PENDING") | Some("EXPECTED") => CiState::Pending,
            _ => CiState::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Good,
    Warn,
    Bad,
    Neutral,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrSummary {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// OPEN | MERGED | CLOSED
    pub state: String,
    pub draft: bool,
    /// CLEAN | BLOCKED | BEHIND | DIRTY | UNSTABLE | DRAFT | HAS_HOOKS | UNKNOWN
    pub merge_state: String,
    /// APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED
    pub review_decision: Option<String>,
    pub ci: CiState,
    pub comment_count: u64,
    pub head_oid: Option<String>,
}

impl PrSummary {
    /// Human label for "can this be merged?".
    pub fn readiness(&self) -> (&'static str, Tone) {
        match (self.state.as_str(), self.draft) {
            ("MERGED", _) => return ("Merged", Tone::Neutral),
            ("CLOSED", _) => return ("Closed", Tone::Neutral),
            (_, true) => return ("Draft", Tone::Neutral),
            _ => {}
        }
        match self.merge_state.as_str() {
            "CLEAN" | "HAS_HOOKS" => ("Ready to merge", Tone::Good),
            "UNSTABLE" => ("Mergeable, some checks failing", Tone::Warn),
            "DIRTY" => ("Merge conflicts", Tone::Bad),
            "BEHIND" => ("Behind base branch", Tone::Warn),
            "BLOCKED" => match self.review_decision.as_deref() {
                Some("CHANGES_REQUESTED") => ("Changes requested", Tone::Bad),
                Some("REVIEW_REQUIRED") => ("Review required", Tone::Warn),
                _ if self.ci == CiState::Failure => ("Blocked by failing checks", Tone::Bad),
                _ if self.ci == CiState::Pending => ("Waiting for checks", Tone::Warn),
                _ => ("Blocked", Tone::Warn),
            },
            _ => ("Checking mergeability…", Tone::Neutral),
        }
    }

    pub fn ci_label(&self) -> (&'static str, Tone) {
        match self.ci {
            CiState::Success => ("CI passing", Tone::Good),
            CiState::Failure => ("CI failing", Tone::Bad),
            CiState::Pending => ("CI running", Tone::Warn),
            CiState::None => ("No CI", Tone::Neutral),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub state: CiState,
    /// Raw GitHub status/conclusion, e.g. "FAILURE", "IN_PROGRESS".
    pub detail: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommentKind {
    Comment,
    Review { state: String },
    Thread { path: String, line: Option<u64>, replies: u64 },
}

#[derive(Debug, Clone, Serialize)]
pub struct PrComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub url: String,
    #[serde(flatten)]
    pub kind: CommentKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrDetail {
    #[serde(flatten)]
    pub summary: PrSummary,
    pub base: String,
    pub checks: Vec<Check>,
    /// Conversation comments, reviews, and unresolved review threads — newest first.
    pub comments: Vec<PrComment>,
    pub unresolved_threads: usize,
    pub resolved_threads: usize,
}

const SUMMARY: &str = "fragment S on PullRequest {
  number title url state isDraft mergeStateStatus reviewDecision headRefOid
  commentCount: comments { totalCount }
  lastCommit: commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
}";

const DETAIL: &str = "
  baseRefName
  comments(last: 30) { nodes { author { login } body createdAt url } }
  reviews(last: 30) { nodes { author { login } state body submittedAt url } }
  reviewThreads(last: 80) { nodes { isResolved path line
    comments(first: 1) { totalCount nodes { author { login } body createdAt url } } } }
  checks: commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes {
    __typename
    ... on CheckRun { name status conclusion detailsUrl }
    ... on StatusContext { context state targetUrl } } } } } } }";

pub struct GitHub {
    pub repo: RemoteRepo,
    api: String,
    auth: String,
    agent: ureq::Agent,
}

impl GitHub {
    /// `Ok(None)` when the remote isn't a GitHub host (or GitHub is disabled).
    pub fn for_repo(repo: &Repo, cfg: &Config, remote: Option<&str>) -> Result<Option<GitHub>> {
        let gh = &cfg.github;
        if gh.disabled {
            return Ok(None);
        }
        let remotes = repo.remotes();
        let name = match remote {
            Some(r) => r.to_string(),
            None if remotes.iter().any(|r| r == "origin") => "origin".into(),
            None => match remotes.first() {
                Some(r) => r.clone(),
                None => return Ok(None),
            },
        };
        let Some(url) = repo.remote_url(&name) else { return Ok(None) };
        let Some(mut rr) = parse_remote(&url) else { return Ok(None) };
        if let Some(real) = gh.host_aliases.get(&rr.host) {
            rr.host = real.to_ascii_lowercase();
        }
        let is_github = rr.host == "github.com" || rr.host.ends_with(".ghe.com") || gh.enterprise_hosts.iter().any(|h| h.eq_ignore_ascii_case(&rr.host));
        if !is_github {
            return Ok(None);
        }
        let api = gh.api_url.clone().unwrap_or_else(|| {
            if rr.host == "github.com" {
                "https://api.github.com/graphql".into()
            } else if rr.host.ends_with(".ghe.com") {
                format!("https://api.{}/graphql", rr.host)
            } else {
                format!("https://{}/api/graphql", rr.host)
            }
        });
        let token = crate::config::github_token(&rr.host).ok_or_else(|| {
            Error(format!("no GitHub token for {}: run `gh auth login` or set GITHUB_TOKEN", rr.host))
        })?;
        Ok(Some(GitHub { repo: rr, api, auth: format!("Bearer {token}"), agent: crate::http::agent() }))
    }

    fn graphql(&self, query: &str, vars: Value) -> Result<Value> {
        let mut resp = self
            .agent
            .post(&self.api)
            .header("Authorization", &self.auth)
            .header("Accept", "application/vnd.github.merge-info-preview+json")
            .send_json(json!({ "query": query, "variables": vars }))
            .map_err(|e| match e {
                ureq::Error::StatusCode(401) => Error("GitHub: 401 — token invalid or expired (`gh auth login`)".into()),
                ureq::Error::StatusCode(403) => Error("GitHub: 403 — forbidden or rate limited (SAML SSO may need authorizing)".into()),
                other => Error(format!("GitHub: {other}")),
            })?;
        let v: Value = resp.body_mut().read_json().map_err(|e| Error(format!("GitHub: bad response: {e}")))?;
        if v["data"].is_null() {
            let msg = v["errors"][0]["message"].as_str().unwrap_or("unknown error");
            bail!("GitHub: {msg}");
        }
        Ok(v["data"].clone())
    }

    /// Latest PR (any state) for each head branch. Branches without a PR are absent.
    pub fn pr_summaries(&self, heads: &[String]) -> Result<HashMap<String, PrSummary>> {
        let mut out = HashMap::new();
        for chunk in heads.chunks(25) {
            let mut decl = String::from("$owner: String!, $name: String!");
            let mut fields = String::new();
            let mut vars = json!({ "owner": self.repo.owner, "name": self.repo.name });
            for (i, h) in chunk.iter().enumerate() {
                decl += &format!(", $h{i}: String!");
                fields += &format!(
                    "b{i}: pullRequests(headRefName: $h{i}, first: 1, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ ...S }} }}\n"
                );
                vars[format!("h{i}")] = json!(h);
            }
            let q = format!("query({decl}) {{ repository(owner: $owner, name: $name) {{ {fields} }} }}\n{SUMMARY}");
            let data = self.graphql(&q, vars)?;
            for (i, h) in chunk.iter().enumerate() {
                if let Some(pr) = data["repository"][format!("b{i}")]["nodes"].get(0) {
                    out.insert(h.clone(), parse_summary(pr));
                }
            }
        }
        Ok(out)
    }

    pub fn pr_detail(&self, head: &str) -> Result<Option<PrDetail>> {
        let q = format!(
            "query($owner: String!, $name: String!, $head: String!) {{ repository(owner: $owner, name: $name) {{
               pullRequests(headRefName: $head, first: 1, orderBy: {{field: CREATED_AT, direction: DESC}}) {{
                 nodes {{ ...S {DETAIL} }} }} }} }}\n{SUMMARY}"
        );
        let data = self.graphql(&q, json!({ "owner": self.repo.owner, "name": self.repo.name, "head": head }))?;
        Ok(data["repository"]["pullRequests"]["nodes"].get(0).map(parse_detail))
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn parse_summary(pr: &Value) -> PrSummary {
    PrSummary {
        number: pr["number"].as_u64().unwrap_or(0),
        title: s(&pr["title"]),
        url: s(&pr["url"]),
        state: s(&pr["state"]),
        draft: pr["isDraft"].as_bool().unwrap_or(false),
        merge_state: s(&pr["mergeStateStatus"]),
        review_decision: pr["reviewDecision"].as_str().map(String::from),
        ci: CiState::from_rollup(pr["lastCommit"]["nodes"][0]["commit"]["statusCheckRollup"]["state"].as_str()),
        comment_count: pr["commentCount"]["totalCount"].as_u64().unwrap_or(0),
        head_oid: pr["headRefOid"].as_str().map(String::from),
    }
}

fn author(v: &Value) -> String {
    v["author"]["login"].as_str().unwrap_or("ghost").to_string()
}

fn parse_detail(pr: &Value) -> PrDetail {
    let mut comments = Vec::new();
    for c in pr["comments"]["nodes"].as_array().into_iter().flatten() {
        comments.push(PrComment {
            author: author(c),
            body: s(&c["body"]),
            created_at: s(&c["createdAt"]),
            url: s(&c["url"]),
            kind: CommentKind::Comment,
        });
    }
    for r in pr["reviews"]["nodes"].as_array().into_iter().flatten() {
        let state = s(&r["state"]);
        let body = s(&r["body"]);
        // Skip empty "COMMENTED" reviews: they only wrap inline thread comments.
        if body.trim().is_empty() && state == "COMMENTED" {
            continue;
        }
        comments.push(PrComment {
            author: author(r),
            body,
            created_at: s(&r["submittedAt"]),
            url: s(&r["url"]),
            kind: CommentKind::Review { state },
        });
    }
    let (mut unresolved, mut resolved) = (0, 0);
    for t in pr["reviewThreads"]["nodes"].as_array().into_iter().flatten() {
        if t["isResolved"].as_bool().unwrap_or(false) {
            resolved += 1;
            continue;
        }
        unresolved += 1;
        let first = &t["comments"]["nodes"][0];
        comments.push(PrComment {
            author: author(first),
            body: s(&first["body"]),
            created_at: s(&first["createdAt"]),
            url: s(&first["url"]),
            kind: CommentKind::Thread {
                path: s(&t["path"]),
                line: t["line"].as_u64(),
                replies: t["comments"]["totalCount"].as_u64().unwrap_or(1).saturating_sub(1),
            },
        });
    }
    comments.sort_by(|a, b| b.created_at.cmp(&a.created_at)); // ISO-8601 sorts lexically

    let mut checks = Vec::new();
    let ctxs = &pr["checks"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"];
    for c in ctxs.as_array().into_iter().flatten() {
        if c["__typename"] == "CheckRun" {
            let status = s(&c["status"]);
            let conclusion = c["conclusion"].as_str();
            let state = match (status.as_str(), conclusion) {
                (_, Some("SUCCESS")) | (_, Some("NEUTRAL")) | (_, Some("SKIPPED")) => CiState::Success,
                (_, Some(_)) => CiState::Failure,
                _ => CiState::Pending,
            };
            checks.push(Check {
                name: s(&c["name"]),
                state,
                detail: conclusion.map(String::from).unwrap_or(status),
                url: c["detailsUrl"].as_str().map(String::from),
            });
        } else {
            let st = s(&c["state"]);
            checks.push(Check {
                name: s(&c["context"]),
                state: CiState::from_rollup(Some(&st)),
                detail: st,
                url: c["targetUrl"].as_str().map(String::from),
            });
        }
    }
    // Failures first, then running, then passing.
    checks.sort_by_key(|c| match c.state {
        CiState::Failure => 0,
        CiState::Pending => 1,
        _ => 2,
    });

    PrDetail {
        summary: parse_summary(pr),
        base: s(&pr["baseRefName"]),
        checks,
        comments,
        unresolved_threads: unresolved,
        resolved_threads: resolved,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn remotes() {
        let want = |h: &str| Some(RemoteRepo { host: h.into(), owner: "acme".into(), name: "app".into() });
        assert_eq!(parse_remote("git@github.com:acme/app.git"), want("github.com"));
        assert_eq!(parse_remote("https://github.com/acme/app"), want("github.com"));
        assert_eq!(parse_remote("https://user@GitHub.com/acme/app.git/"), want("github.com"));
        assert_eq!(parse_remote("ssh://git@ghe.corp.com:2222/acme/app.git"), want("ghe.corp.com"));
        assert_eq!(parse_remote("github-work:acme/app.git"), want("github-work"));
        assert_eq!(parse_remote("/local/path/repo"), None);
    }

    pub const FIXTURE: &str = r#"{
      "number": 12, "title": "Login rework", "url": "https://github.com/acme/app/pull/12",
      "state": "OPEN", "isDraft": false, "mergeStateStatus": "BLOCKED", "reviewDecision": "CHANGES_REQUESTED",
      "headRefOid": "abc", "baseRefName": "main",
      "commentCount": { "totalCount": 1 },
      "lastCommit": { "nodes": [ { "commit": { "statusCheckRollup": { "state": "FAILURE" } } } ] },
      "comments": { "nodes": [ { "author": { "login": "kim" }, "body": "Looks close!", "createdAt": "2026-11-09T10:00:00Z", "url": "u1" } ] },
      "reviews": { "nodes": [
        { "author": { "login": "lee" }, "state": "CHANGES_REQUESTED", "body": "Please add a test.", "submittedAt": "2026-11-10T09:00:00Z", "url": "u2" },
        { "author": { "login": "lee" }, "state": "COMMENTED", "body": "", "submittedAt": "2026-11-10T09:00:01Z", "url": "u3" } ] },
      "reviewThreads": { "nodes": [
        { "isResolved": false, "path": "src/auth.rs", "line": 42, "comments": { "totalCount": 3, "nodes": [ { "author": { "login": "lee" }, "body": "Handle 429 here", "createdAt": "2026-11-10T08:00:00Z", "url": "u4" } ] } },
        { "isResolved": true, "path": "x", "line": 1, "comments": { "totalCount": 1, "nodes": [ { "author": null, "body": "ok", "createdAt": "2026-11-01T00:00:00Z", "url": "u5" } ] } } ] },
      "checks": { "nodes": [ { "commit": { "statusCheckRollup": { "contexts": { "nodes": [
        { "__typename": "CheckRun", "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS", "detailsUrl": "d1" },
        { "__typename": "CheckRun", "name": "test", "status": "COMPLETED", "conclusion": "FAILURE", "detailsUrl": "d2" },
        { "__typename": "CheckRun", "name": "lint", "status": "IN_PROGRESS", "conclusion": null, "detailsUrl": null },
        { "__typename": "StatusContext", "context": "ci/jenkins", "state": "SUCCESS", "targetUrl": "d3" } ] } } } } ] }
    }"#;

    #[test]
    fn detail_parsing() {
        let d = parse_detail(&serde_json::from_str(FIXTURE).unwrap());
        assert_eq!(d.summary.number, 12);
        assert_eq!(d.summary.readiness(), ("Changes requested", Tone::Bad));
        assert_eq!(d.summary.ci, CiState::Failure);
        assert_eq!(d.checks.len(), 4);
        assert_eq!(d.checks[0].name, "test"); // failures first
        assert_eq!(d.checks[1].name, "lint");
        assert_eq!((d.unresolved_threads, d.resolved_threads), (1, 1));
        // comment + changes-requested review + unresolved thread; empty COMMENTED review dropped
        assert_eq!(d.comments.len(), 3);
        assert_eq!(d.comments[0].author, "lee"); // newest first
        assert!(matches!(d.comments[1].kind, CommentKind::Thread { line: Some(42), replies: 2, .. }));
    }

    #[test]
    fn readiness_labels() {
        let mut p = parse_summary(&serde_json::from_str(FIXTURE).unwrap());
        p.merge_state = "CLEAN".into();
        assert_eq!(p.readiness().0, "Ready to merge");
        p.merge_state = "DIRTY".into();
        assert_eq!(p.readiness().0, "Merge conflicts");
        p.state = "MERGED".into();
        assert_eq!(p.readiness().0, "Merged");
    }
}
