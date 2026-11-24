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

/// One comment in a review thread.
#[derive(Debug, Clone, Serialize)]
pub struct ThreadComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub url: String,
}

/// An inline review thread (a conversation on a line of the PR's diff).
#[derive(Debug, Clone, Serialize)]
pub struct ReviewThread {
    /// GraphQL node id (for replying / resolving).
    pub id: String,
    pub path: String,
    /// Line in the PR's current head (new side) or base (old side); `None` when outdated.
    pub line: Option<u64>,
    /// Line in the version the first comment was written on.
    pub original_line: Option<u64>,
    /// The thread is on the old (removed / left) side of the diff.
    pub old_side: bool,
    /// Commit the first comment was written on.
    pub original_commit: String,
    pub resolved: bool,
    /// The code it was written on has changed since.
    pub outdated: bool,
    /// Oldest first.
    pub comments: Vec<ThreadComment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrDetail {
    #[serde(flatten)]
    pub summary: PrSummary,
    /// GraphQL node id (for new review comments).
    pub id: String,
    pub base: String,
    pub checks: Vec<Check>,
    /// Conversation comments, reviews, and unresolved review threads — newest first.
    pub comments: Vec<PrComment>,
    pub unresolved_threads: usize,
    pub resolved_threads: usize,
    /// All review threads with every comment (for showing them in the diff).
    pub threads: Vec<ReviewThread>,
    /// The description, its author, and the branch it comes from.
    pub body: String,
    pub author: String,
    pub head: String,
    /// Size of the change.
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
}

/// An open pull request, for choosing one to review.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrListItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
    /// APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED
    pub review_decision: Option<String>,
    pub updated_at: String,
    pub additions: u64,
    pub deletions: u64,
    /// A review is requested from the signed-in user.
    pub review_requested: bool,
    /// The signed-in user opened it.
    pub mine: bool,
}

/// How a review is submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}

impl ReviewEvent {
    fn api(self) -> &'static str {
        match self {
            ReviewEvent::Comment => "COMMENT",
            ReviewEvent::Approve => "APPROVE",
            ReviewEvent::RequestChanges => "REQUEST_CHANGES",
        }
    }
}

/// A line comment written during a review, sent with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftComment {
    pub path: String,
    pub line: u64,
    /// On the removed (left) side of the diff.
    pub old_side: bool,
    pub body: String,
}

const SUMMARY: &str = "fragment S on PullRequest {
  number title url state isDraft mergeStateStatus reviewDecision headRefOid
  commentCount: comments { totalCount }
  lastCommit: commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
}";

const DETAIL: &str = "
  id baseRefName headRefName body additions deletions changedFiles author { login }
  comments(last: 30) { nodes { author { login } body createdAt url } }
  reviews(last: 30) { nodes { author { login } state body submittedAt url } }
  reviewThreads(last: 80) { nodes { id isResolved isOutdated path line originalLine diffSide
    comments(first: 50) { totalCount nodes { author { login } body createdAt url originalCommit { oid } } } } }
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
            Error(format!("no GitHub token for {}: add one in Settings → Accounts", rr.host))
        })?;
        Ok(Some(GitHub { repo: rr, api, auth: format!("Bearer {token}"), agent: crate::http::agent() }))
    }

    /// Who `token` signs in as on `host` (github.com or an Enterprise host).
    pub fn whoami(host: &str, token: &str, api_url: Option<&str>) -> Result<String> {
        let api = api_url.map(str::to_string).unwrap_or_else(|| match host {
            "github.com" => "https://api.github.com/graphql".into(),
            h if h.ends_with(".ghe.com") => format!("https://api.{h}/graphql"),
            h => format!("https://{h}/api/graphql"),
        });
        let gh = GitHub {
            repo: RemoteRepo { host: host.into(), owner: String::new(), name: String::new() },
            api,
            auth: format!("Bearer {}", token.trim()),
            agent: crate::http::agent(),
        };
        let v = gh.graphql("query { viewer { login } }", json!({}))?;
        Ok(v["viewer"]["login"].as_str().unwrap_or("signed in").to_string())
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

    /// Reply to a review thread.
    pub fn reply_thread(&self, thread_id: &str, body: &str) -> Result<()> {
        let q = "mutation($id: ID!, $body: String!) { addPullRequestReviewThreadReply(input: {pullRequestReviewThreadId: $id, body: $body}) { comment { id } } }";
        self.graphql(q, json!({ "id": thread_id, "body": body })).map(drop)
    }

    /// Resolve (or reopen) a review thread.
    pub fn set_thread_resolved(&self, thread_id: &str, resolved: bool) -> Result<()> {
        let q = if resolved {
            "mutation($id: ID!) { resolveReviewThread(input: {threadId: $id}) { thread { isResolved } } }"
        } else {
            "mutation($id: ID!) { unresolveReviewThread(input: {threadId: $id}) { thread { isResolved } } }"
        };
        self.graphql(q, json!({ "id": thread_id })).map(drop)
    }

    /// Start a new review thread on `line` of `path` as of `commit` (old side:
    /// the line number before the change). Posted as a one-comment review.
    pub fn new_thread(&self, pr_id: &str, commit: &str, path: &str, line: u64, old_side: bool, body: &str) -> Result<()> {
        let q = "mutation($pr: ID!, $commit: GitObjectID!, $path: String!, $line: Int!, $side: DiffSide!, $body: String!) {
  addPullRequestReview(input: {pullRequestId: $pr, commitOID: $commit, event: COMMENT,
    threads: [{path: $path, line: $line, side: $side, body: $body}]}) { pullRequestReview { id } } }";
        let side = if old_side { "LEFT" } else { "RIGHT" };
        self.graphql(q, json!({ "pr": pr_id, "commit": commit, "path": path, "line": line, "side": side, "body": body })).map(drop)
    }

    /// Browser link to issue `#n` (or a bare number).
    pub fn issue_url(&self, key: &str) -> String {
        format!("https://{}/{}/{}/issues/{}", self.repo.host, self.repo.owner, self.repo.name, key.trim_start_matches('#'))
    }

    /// Issue (or pull request) `#n` of this repository, as a task.
    pub fn issue(&self, key: &str) -> Result<crate::tasks::Task> {
        let n: i64 = key.trim_start_matches('#').parse().map_err(|_| Error("not an issue number".into()))?;
        let q = "query($owner: String!, $name: String!, $n: Int!) { repository(owner: $owner, name: $name) {
  issueOrPullRequest(number: $n) {
    ... on Issue { title state url updatedAt body assignees(first: 1) { nodes { login } } labels(first: 1) { nodes { name } } }
    ... on PullRequest { title state url updatedAt body assignees(first: 1) { nodes { login } } }
  } } }";
        let data = self.graphql(q, json!({ "owner": self.repo.owner, "name": self.repo.name, "n": n }))?;
        let i = &data["repository"]["issueOrPullRequest"];
        if i.is_null() {
            bail!("#{n} not found in {}/{}", self.repo.owner, self.repo.name);
        }
        Ok(issue_task(&format!("#{n}"), i))
    }

    /// Open issues of this repository assigned to the signed-in user.
    pub fn my_issues(&self, limit: usize) -> Result<Vec<crate::tasks::Task>> {
        let q = "query($q: String!, $n: Int!) { search(query: $q, type: ISSUE, first: $n) { nodes {
  ... on Issue { number title state url updatedAt body assignees(first: 1) { nodes { login } } labels(first: 1) { nodes { name } } } } } }";
        let search = format!("repo:{}/{} is:issue is:open assignee:@me sort:updated-desc", self.repo.owner, self.repo.name);
        let data = self.graphql(q, json!({ "q": search, "n": limit.min(50) }))?;
        Ok(data["search"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|i| i["number"].is_number())
            .map(|i| issue_task(&format!("#{}", i["number"]), i))
            .collect())
    }

    /// Pull request `number` with everything shown in review mode.
    pub fn pr_by_number(&self, number: u64) -> Result<Option<PrDetail>> {
        let q = format!(
            "query($owner: String!, $name: String!, $n: Int!) {{ repository(owner: $owner, name: $name) {{
               pullRequest(number: $n) {{ ...S {DETAIL} }} }} }}\n{SUMMARY}"
        );
        let data = self.graphql(&q, json!({ "owner": self.repo.owner, "name": self.repo.name, "n": number }))?;
        let pr = &data["repository"]["pullRequest"];
        Ok((!pr.is_null()).then(|| parse_detail(pr)))
    }

    /// Open pull requests, most recently updated first.
    /// People who can be @mentioned in this repository whose login or name
    /// matches `query` (collaborators, contributors, people in its issues and
    /// pull requests): (login, name), at most 20.
    pub fn mentionable_users(&self, query: &str) -> Result<Vec<(String, String)>> {
        let q = "query($owner: String!, $name: String!, $q: String) { repository(owner: $owner, name: $name) {
    mentionableUsers(first: 20, query: $q) { nodes { login name } } } }";
        let data = self.graphql(q, json!({ "owner": self.repo.owner, "name": self.repo.name, "q": query }))?;
        Ok(data["repository"]["mentionableUsers"]["nodes"]
            .as_array()
            .map(|a| a.iter().map(|u| (s(&u["login"]), s(&u["name"]))).filter(|(l, _)| !l.is_empty()).collect())
            .unwrap_or_default())
    }

    pub fn open_prs(&self, limit: usize) -> Result<Vec<PrListItem>> {
        let q = "query($owner: String!, $name: String!, $n: Int!) { viewer { login }
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: $n, orderBy: {field: UPDATED_AT, direction: DESC}) { nodes {
      number title url isDraft headRefName baseRefName updatedAt reviewDecision additions deletions author { login }
      reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login } } } } } } } }";
        let data = self.graphql(q, json!({ "owner": self.repo.owner, "name": self.repo.name, "n": limit.min(100) }))?;
        Ok(parse_pr_list(&data))
    }

    /// Submit a review of `commit` (the PR's head as reviewed): approve,
    /// request changes, or comment, with `comments` on lines. Returns its URL.
    pub fn submit_review(&self, pr_id: &str, commit: &str, event: ReviewEvent, body: &str, comments: &[DraftComment]) -> Result<String> {
        let q = "mutation($pr: ID!, $commit: GitObjectID!, $event: PullRequestReviewEvent!, $body: String, $threads: [DraftPullRequestReviewThread]) {
  addPullRequestReview(input: {pullRequestId: $pr, commitOID: $commit, event: $event, body: $body, threads: $threads}) { pullRequestReview { url } } }";
        let threads: Vec<Value> = comments
            .iter()
            .map(|c| json!({ "path": c.path, "line": c.line, "side": if c.old_side { "LEFT" } else { "RIGHT" }, "body": c.body }))
            .collect();
        let body = (!body.trim().is_empty()).then(|| body.trim().to_string());
        let data = self.graphql(q, json!({ "pr": pr_id, "commit": commit, "event": event.api(), "body": body, "threads": threads }))?;
        Ok(s(&data["addPullRequestReview"]["pullRequestReview"]["url"]))
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

fn parse_pr_list(data: &Value) -> Vec<PrListItem> {
    let me = data["viewer"]["login"].as_str().unwrap_or("");
    data["repository"]["pullRequests"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let author = author(p);
            PrListItem {
                number: p["number"].as_u64().unwrap_or(0),
                title: s(&p["title"]),
                url: s(&p["url"]),
                head: s(&p["headRefName"]),
                base: s(&p["baseRefName"]),
                draft: p["isDraft"].as_bool().unwrap_or(false),
                review_decision: p["reviewDecision"].as_str().map(String::from),
                updated_at: s(&p["updatedAt"]),
                additions: p["additions"].as_u64().unwrap_or(0),
                deletions: p["deletions"].as_u64().unwrap_or(0),
                review_requested: !me.is_empty()
                    && p["reviewRequests"]["nodes"].as_array().into_iter().flatten().any(|r| r["requestedReviewer"]["login"].as_str() == Some(me)),
                mine: !me.is_empty() && author == me,
                author,
            }
        })
        .collect()
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
    let mut threads = vec![];
    for t in pr["reviewThreads"]["nodes"].as_array().into_iter().flatten() {
        let tc: Vec<ThreadComment> = t["comments"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| ThreadComment { author: author(c), body: s(&c["body"]), created_at: s(&c["createdAt"]), url: s(&c["url"]) })
            .collect();
        threads.push(ReviewThread {
            id: s(&t["id"]),
            path: s(&t["path"]),
            line: t["line"].as_u64(),
            original_line: t["originalLine"].as_u64().or(t["line"].as_u64()),
            old_side: t["diffSide"].as_str() == Some("LEFT"),
            original_commit: s(&t["comments"]["nodes"][0]["originalCommit"]["oid"]),
            resolved: t["isResolved"].as_bool().unwrap_or(false),
            outdated: t["isOutdated"].as_bool().unwrap_or(false),
            comments: tc,
        });
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
        id: s(&pr["id"]),
        base: s(&pr["baseRefName"]),
        checks,
        comments,
        unresolved_threads: unresolved,
        threads,
        resolved_threads: resolved,
        body: s(&pr["body"]),
        author: author(pr),
        head: s(&pr["headRefName"]),
        additions: pr["additions"].as_u64().unwrap_or(0),
        deletions: pr["deletions"].as_u64().unwrap_or(0),
        changed_files: pr["changedFiles"].as_u64().unwrap_or(0),
    }
}

/// A GitHub issue / pull request node as a task (state OPEN → "new", else "done").
fn issue_task(key: &str, i: &Value) -> crate::tasks::Task {
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let state = s(&i["state"]);
    crate::tasks::Task {
        key: key.to_string(),
        summary: s(&i["title"]),
        status_category: if state == "OPEN" { "new".into() } else { "done".into() },
        status: match state.as_str() {
            "OPEN" => "Open".into(),
            "CLOSED" => "Closed".into(),
            "MERGED" => "Merged".into(),
            other => other.to_string(),
        },
        assignee: i["assignees"]["nodes"][0]["login"].as_str().map(str::to_string),
        issue_type: i["labels"]["nodes"][0]["name"].as_str().unwrap_or("Issue").to_string(),
        priority: None,
        url: s(&i["url"]),
        description: s(&i["body"]),
        updated: s(&i["updatedAt"]),
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
        { "id": "T1", "isResolved": false, "isOutdated": false, "path": "src/auth.rs", "line": 42, "originalLine": 40, "diffSide": "RIGHT", "comments": { "totalCount": 3, "nodes": [
          { "author": { "login": "lee" }, "body": "Handle 429 here", "createdAt": "2026-11-10T08:00:00Z", "url": "u4", "originalCommit": { "oid": "c1" } },
          { "author": { "login": "kim" }, "body": "Done in the next commit", "createdAt": "2026-11-10T09:00:00Z", "url": "u6", "originalCommit": { "oid": "c1" } },
          { "author": { "login": "lee" }, "body": "Thanks", "createdAt": "2026-11-10T09:30:00Z", "url": "u7", "originalCommit": { "oid": "c1" } } ] } },
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
        // full threads for the diff
        assert_eq!(d.threads.len(), 2);
        let t = &d.threads[0];
        assert_eq!((t.path.as_str(), t.line, t.original_line, t.old_side), ("src/auth.rs", Some(42), Some(40), false));
        assert_eq!((t.original_commit.as_str(), t.id.as_str()), ("c1", "T1"));
        assert_eq!(t.comments.iter().map(|c| c.author.as_str()).collect::<Vec<_>>(), ["lee", "kim", "lee"]);
        assert!(d.threads[1].resolved);
    }

    #[test]
    fn open_pr_list() {
        let data = json!({
            "viewer": {"login": "me"},
            "repository": {"pullRequests": {"nodes": [
                {"number": 12, "title": "Add reminders", "url": "u12", "isDraft": false, "headRefName": "feat/x", "baseRefName": "main",
                 "updatedAt": "2026-11-20T10:00:00Z", "reviewDecision": "REVIEW_REQUIRED", "additions": 40, "deletions": 3,
                 "author": {"login": "jo"}, "reviewRequests": {"nodes": [{"requestedReviewer": {"login": "me"}}, {"requestedReviewer": {}}]}},
                {"number": 9, "title": "Mine", "url": "u9", "isDraft": true, "headRefName": "fix/y", "baseRefName": "main",
                 "updatedAt": "2026-11-19T10:00:00Z", "reviewDecision": null, "additions": 1, "deletions": 1,
                 "author": {"login": "me"}, "reviewRequests": {"nodes": []}}
            ]}}
        });
        let l = parse_pr_list(&data);
        assert_eq!(l.len(), 2);
        assert!(l[0].review_requested && !l[0].mine && l[0].head == "feat/x" && l[0].additions == 40);
        assert!(!l[1].review_requested && l[1].mine && l[1].draft);
        assert_eq!(ReviewEvent::RequestChanges.api(), "REQUEST_CHANGES");
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
