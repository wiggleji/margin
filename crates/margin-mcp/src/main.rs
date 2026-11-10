//! margin-mcp — Model Context Protocol server over stdio.
//!
//! Lets agents (Claude Code, Cursor, Copilot, …) read git context and
//! read/write the same Markdown notes the Margin GUI shows. The GUI watches
//! the notes dir, so an agent's edits appear live.
//!
//! Git access here is deliberately READ-ONLY. Agents may write notes and
//! task links, never commits/refs.
//!
//!   claude mcp add margin -- margin-mcp --repo /path/to/repo
//!
//! Hand-rolled JSON-RPC (newline-delimited) — no async runtime, ~1 MB binary.

use margin_core::git::Repo;
use margin_core::notes::{NoteStore, NoteTarget};
use margin_core::tasks::{self, TaskProvider};
use margin_core::{Error, Result, bail, config};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const SUPPORTED_PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_PATCH: usize = 200 * 1024;

struct Server {
    repo: Repo,
    notes: NoteStore,
    tasks: Option<Box<dyn TaskProvider>>,
    author: String,
}

fn main() {
    let mut repo_path = std::env::current_dir().unwrap_or_default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--repo" => repo_path = args.next().map(Into::into).unwrap_or(repo_path),
            "--help" | "-h" => {
                eprintln!("usage: margin-mcp [--repo <path>]   (speaks MCP on stdin/stdout)");
                return;
            }
            "--version" => {
                println!("margin-mcp {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => {}
        }
    }
    let repo = match Repo::open(&repo_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("margin-mcp: {}: {e}", repo_path.display());
            std::process::exit(2);
        }
    };
    let notes = NoteStore::for_repo(&repo).expect("create notes dir");
    let cfg = config::load_config();
    let mut srv = Server {
        tasks: tasks::provider_from_config(&cfg),
        author: std::env::var("MARGIN_AGENT_NAME").unwrap_or_else(|_| "agent".into()),
        repo,
        notes,
    };

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => srv.handle(&msg),
            Err(e) => Some(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(r) = resp {
            let _ = writeln!(stdout, "{r}");
            let _ = stdout.flush();
        }
    }
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}
fn rpc_error(id: Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

impl Server {
    fn handle(&mut self, msg: &Value) -> Option<Value> {
        let method = msg["method"].as_str().unwrap_or("");
        let id = msg.get("id").cloned();
        let params = &msg["params"];
        let id = match id {
            Some(id) => id,
            None => return None, // notification (e.g. notifications/initialized)
        };
        Some(match method {
            "initialize" => {
                if let Some(n) = params["clientInfo"]["name"].as_str() {
                    if std::env::var("MARGIN_AGENT_NAME").is_err() {
                        self.author = n.to_string();
                    }
                }
                let asked = params["protocolVersion"].as_str().unwrap_or("");
                let ver = if SUPPORTED_PROTOCOLS.contains(&asked) { asked } else { SUPPORTED_PROTOCOLS[0] };
                rpc_ok(
                    id,
                    json!({
                        "protocolVersion": ver,
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": {"name": "margin", "version": env!("CARGO_PKG_VERSION")},
                        "instructions": INSTRUCTIONS,
                    }),
                )
            }
            "ping" => rpc_ok(id, json!({})),
            "tools/list" => rpc_ok(id, json!({"tools": tool_defs()})),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or("");
                let empty = json!({});
                let args = params.get("arguments").unwrap_or(&empty);
                let result = match self.call(name, args) {
                    Ok(v) => json!({
                        "content": [{"type": "text", "text": as_text(&v)}],
                        "structuredContent": if v.is_object() { v.clone() } else { json!({"result": v}) },
                        "isError": false
                    }),
                    Err(e) => json!({"content": [{"type": "text", "text": e.0}], "isError": true}),
                };
                rpc_ok(id, result)
            }
            _ => rpc_error(id, -32601, &format!("method not found: {method}")),
        })
    }

    fn call(&mut self, name: &str, a: &Value) -> Result<Value> {
        let s = |k: &str| a[k].as_str();
        let n = |k: &str, d: u64| a[k].as_u64().unwrap_or(d) as usize;
        match name {
            "get_context" => self.context(),
            "git_status" => {
                let st = self.repo.status()?;
                Ok(json!({
                    "branch": st.branch, "upstream": st.upstream, "ahead": st.ahead, "behind": st.behind,
                    "files": st.files.iter().map(|f| json!({
                        "path": f.path, "orig_path": f.orig_path,
                        "staged": f.index.to_string(), "worktree": f.worktree.to_string(), "conflicted": f.conflicted
                    })).collect::<Vec<_>>()
                }))
            }
            "git_log" => {
                let rev = s("rev").unwrap_or("HEAD");
                if rev.starts_with('-') {
                    bail!("invalid rev");
                }
                let commits = self.repo.log_rev(rev, n("limit", 30).min(500))?;
                Ok(json!({"commits": commits.iter().map(|c| json!({
                    "id": c.id, "parents": c.parents, "author": c.author, "time": margin_core::format_utc(c.time),
                    "refs": c.refs, "subject": c.subject,
                    "has_note": self.notes.exists(&NoteTarget::Commit(c.id.clone())),
                })).collect::<Vec<_>>()}))
            }
            "git_show" => {
                let id = self.repo.resolve(s("rev").ok_or_else(|| Error("rev is required".into()))?)?;
                let d = self.repo.commit_detail(&id)?;
                let stat = self.repo.run(&["show", "--stat", "--format=", "--diff-merges=first-parent", &id])?;
                let mut out = json!({
                    "id": d.id, "parents": d.parents, "author": format!("{} <{}>", d.author, d.email),
                    "date": margin_core::format_utc(d.author_time), "message": d.message, "stat": stat.trim_end(),
                    "note": self.notes.read(&NoteTarget::Commit(id.clone()))?.map(|n| n.content),
                });
                if a["include_patch"].as_bool().unwrap_or(false) {
                    let mut p = self.repo.commit_diff(&id)?;
                    if p.len() > MAX_PATCH {
                        let mut end = MAX_PATCH;
                        while !p.is_char_boundary(end) {
                            end -= 1;
                        }
                        p.truncate(end);
                        p.push_str("\n… [truncated]");
                    }
                    out["patch"] = json!(p);
                }
                Ok(out)
            }
            "list_branches" => {
                let refs = self.repo.refs()?;
                Ok(json!({"branches": refs.iter().filter(|r| r.kind != margin_core::git::RefKind::Tag).map(|r| json!({
                    "name": r.name,
                    "remote": r.kind == margin_core::git::RefKind::Remote,
                    "current": r.is_head, "upstream": r.upstream, "track": r.track,
                    "task": (r.kind == margin_core::git::RefKind::Local).then(|| self.notes.task_for_branch(&r.name)).flatten(),
                    "has_note": self.notes.exists(&NoteTarget::Branch(r.name.clone())),
                })).collect::<Vec<_>>()}))
            }
            "list_notes" => {
                let kind = s("kind");
                let list = self.notes.list()?;
                Ok(json!({"notes": list.iter().filter(|m| kind.is_none_or(|k| m.target.kind() == k)).map(|m| json!({
                    "target": m.target.to_string(), "title": m.title, "bytes": m.bytes,
                    "modified": margin_core::format_utc(m.modified),
                })).collect::<Vec<_>>()}))
            }
            "read_note" => {
                let t = self.target(s("target"))?;
                Ok(match self.notes.read(&t)? {
                    Some(n) => json!({"target": t.to_string(), "exists": true, "version": n.version, "content": n.content}),
                    None => json!({"target": t.to_string(), "exists": false, "version": "", "content": ""}),
                })
            }
            "write_note" => {
                let t = self.target(s("target"))?;
                let content = s("content").ok_or_else(|| Error("content is required".into()))?;
                let note = self.notes.write(&t, content, s("if_version"))?;
                Ok(json!({"target": t.to_string(), "version": note.version, "deleted": note.content.is_empty()}))
            }
            "append_note" => {
                let t = self.target(s("target"))?;
                let content = s("content").ok_or_else(|| Error("content is required".into()))?;
                let author = s("author").unwrap_or(&self.author).to_string();
                let note = self.notes.append(&t, content, &author)?;
                Ok(json!({"target": t.to_string(), "version": note.version}))
            }
            "search_notes" => {
                let q = s("query").ok_or_else(|| Error("query is required".into()))?;
                let hits = self.notes.search(q, n("limit", 50))?;
                Ok(json!({"hits": hits.iter().map(|h| json!({
                    "target": h.target.to_string(), "line": h.line, "text": h.text
                })).collect::<Vec<_>>()}))
            }
            "link_task" => {
                let branch = match s("branch") {
                    Some(b) if b != "@" => b.to_string(),
                    _ => self.current_branch()?,
                };
                let key = a["task_key"].as_str();
                self.notes.link_task(&branch, key)?;
                Ok(json!({"branch": branch, "task": self.notes.task_for_branch(&branch)}))
            }
            "get_task" => {
                let p = self.provider()?;
                let key = s("key").ok_or_else(|| Error("key is required".into()))?;
                let t = p.get(key)?;
                let mut v = serde_json::to_value(&t)?;
                v["note"] = json!(self.notes.read(&NoteTarget::Task(t.key.clone()))?.map(|n| n.content));
                Ok(v)
            }
            "search_tasks" => {
                let p = self.provider()?;
                let q = s("query").ok_or_else(|| Error("query is required".into()))?;
                Ok(json!({"tasks": p.search(q, n("limit", 20))?}))
            }
            "my_tasks" => {
                let p = self.provider()?;
                Ok(json!({"tasks": p.my_open(n("limit", 20))?}))
            }
            _ => bail!("unknown tool: {name}"),
        }
    }

    fn provider(&self) -> Result<&dyn TaskProvider> {
        self.tasks
            .as_deref()
            .ok_or_else(|| Error("no task provider configured (add `jira` to Margin config.json)".into()))
    }

    fn current_branch(&self) -> Result<String> {
        self.repo.status()?.branch.ok_or_else(|| Error("HEAD is detached".into()))
    }

    /// Accepts shorthands: `branch:@` (current), `commit:HEAD~1` (any rev), `task:@` (current branch's task).
    fn target(&self, raw: Option<&str>) -> Result<NoteTarget> {
        let raw = raw.ok_or_else(|| Error("target is required".into()))?.trim();
        if let Some(rev) = raw.strip_prefix("commit:") {
            if rev.starts_with('-') {
                bail!("invalid rev");
            }
            return Ok(NoteTarget::Commit(self.repo.resolve(rev)?));
        }
        if raw == "branch:@" || raw == "branch:" {
            return Ok(NoteTarget::Branch(self.current_branch()?));
        }
        if raw == "task:@" || raw == "task:" {
            let b = self.current_branch()?;
            let key = self.notes.task_for_branch(&b).ok_or_else(|| Error(format!("branch '{b}' has no linked task")))?;
            return Ok(NoteTarget::Task(key));
        }
        NoteTarget::parse(raw)
    }

    fn context(&self) -> Result<Value> {
        let st = self.repo.status()?;
        let branch_note = st.branch.as_ref().map(|b| NoteTarget::Branch(b.clone()));
        let task = st.branch.as_ref().and_then(|b| self.notes.task_for_branch(b));
        let read = |t: &NoteTarget| -> Result<Value> {
            Ok(self.notes.read(t)?.map(|n| json!({"target": t.to_string(), "version": n.version, "content": n.content})).unwrap_or(Value::Null))
        };
        Ok(json!({
            "repo": self.repo.workdir.display().to_string(),
            "branch": st.branch, "head": st.head_oid, "upstream": st.upstream,
            "ahead": st.ahead, "behind": st.behind,
            "changed_files": st.files.len(),
            "pending_operation": self.repo.pending_operation(),
            "task_key": task,
            "task_url": task.as_ref().and_then(|k| self.tasks.as_ref().map(|p| p.browse_url(k))),
            "repo_note": read(&NoteTarget::Repo)?,
            "branch_note": match &branch_note { Some(t) => read(t)?, None => Value::Null },
            "task_note": match &task { Some(k) => read(&NoteTarget::Task(k.clone()))?, None => Value::Null },
        }))
    }
}

fn as_text(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

const INSTRUCTIONS: &str = "Margin shares Markdown notes between a developer and AI agents, attached to the repo, \
branches, commits and tasks (e.g. Jira tickets). Call get_context first. Targets: `repo`, `branch:<name>` \
(`branch:@` = current), `commit:<rev>`, `task:<KEY>` (`task:@` = current branch's task). Prefer append_note for \
progress logs; use write_note with if_version (from read_note) for edits so you never overwrite concurrent changes. \
Notes are Markdown: links are allowed; images and raw HTML are rejected. Git access is read-only.";

fn tool_defs() -> Value {
    let target = json!({"type": "string", "description": "repo | branch:<name> | branch:@ | commit:<rev> | task:<KEY> | task:@"});
    let ro = json!({"readOnlyHint": true, "openWorldHint": false});
    json!([
        {"name": "get_context", "title": "Current work context",
         "description": "Current branch, ahead/behind, linked task key, and the repo/branch/task notes. Start here.",
         "inputSchema": {"type": "object", "properties": {}}, "annotations": ro},
        {"name": "git_status", "description": "Working tree status (porcelain).",
         "inputSchema": {"type": "object", "properties": {}}, "annotations": ro},
        {"name": "git_log", "description": "Commit log for a revision or range (default HEAD).",
         "inputSchema": {"type": "object", "properties": {
            "rev": {"type": "string", "description": "e.g. HEAD, main..HEAD, origin/main"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 500}}}, "annotations": ro},
        {"name": "git_show", "description": "Commit metadata, message, diffstat and its note; optional patch (truncated at 200KB).",
         "inputSchema": {"type": "object", "required": ["rev"], "properties": {
            "rev": {"type": "string"}, "include_patch": {"type": "boolean"}}}, "annotations": ro},
        {"name": "list_branches", "description": "Local and remote branches with tracking info, linked task and note presence.",
         "inputSchema": {"type": "object", "properties": {}}, "annotations": ro},
        {"name": "list_notes", "description": "List existing notes, newest first.",
         "inputSchema": {"type": "object", "properties": {
            "kind": {"type": "string", "enum": ["repo", "branch", "commit", "task"]}}}, "annotations": ro},
        {"name": "read_note", "description": "Read a note. Returns content and a version token for write_note.",
         "inputSchema": {"type": "object", "required": ["target"], "properties": {"target": target}}, "annotations": ro},
        {"name": "write_note", "description": "Replace a note's Markdown (empty content deletes). Pass if_version from read_note to avoid clobbering concurrent edits (\"\" = must not exist).",
         "inputSchema": {"type": "object", "required": ["target", "content"], "properties": {
            "target": target, "content": {"type": "string"}, "if_version": {"type": "string"}}},
         "annotations": {"destructiveHint": true, "idempotentHint": true, "openWorldHint": false}},
        {"name": "append_note", "description": "Append a timestamped, attributed Markdown section to a note (creates it if missing). Safe under concurrency.",
         "inputSchema": {"type": "object", "required": ["target", "content"], "properties": {
            "target": target, "content": {"type": "string"}, "author": {"type": "string"}}},
         "annotations": {"destructiveHint": false, "openWorldHint": false}},
        {"name": "search_notes", "description": "Case-insensitive full-text search across all notes.",
         "inputSchema": {"type": "object", "required": ["query"], "properties": {
            "query": {"type": "string"}, "limit": {"type": "integer"}}}, "annotations": ro},
        {"name": "link_task", "description": "Link a branch to a task key (overrides the key inferred from the branch name). task_key null removes the link.",
         "inputSchema": {"type": "object", "properties": {
            "branch": {"type": "string", "description": "default: current branch"},
            "task_key": {"type": ["string", "null"]}}},
         "annotations": {"idempotentHint": true, "openWorldHint": false}},
        {"name": "get_task", "description": "Fetch a ticket from the task tracker (Jira) plus its local note.",
         "inputSchema": {"type": "object", "required": ["key"], "properties": {"key": {"type": "string"}}},
         "annotations": {"readOnlyHint": true, "openWorldHint": true}},
        {"name": "search_tasks", "description": "Search tickets with the tracker's query language (JQL for Jira).",
         "inputSchema": {"type": "object", "required": ["query"], "properties": {
            "query": {"type": "string"}, "limit": {"type": "integer"}}},
         "annotations": {"readOnlyHint": true, "openWorldHint": true}},
        {"name": "my_tasks", "description": "Open tickets assigned to the configured user.",
         "inputSchema": {"type": "object", "properties": {"limit": {"type": "integer"}}},
         "annotations": {"readOnlyHint": true, "openWorldHint": true}}
    ])
}
