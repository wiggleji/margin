//! Markdown notes attached to git objects and tasks.
//!
//! Storage is plain files so humans, the GUI and MCP agents can all work on
//! them concurrently without a database:
//!
//! ```text
//! <git-common-dir>/margin/notes/
//!   repo.md                     # general notes for the repository
//!   branch/feature%2Flogin.md   # per branch (name percent-encoded)
//!   commit/<full-sha>.md        # per commit
//!   task/PROJ-123.md            # per task / ticket
//!   links.json                  # explicit branch -> task links
//! ```
//!
//! Living inside the git dir means notes are never committed by accident,
//! survive branch switches, and are shared by all worktrees of the repo.
//!
//! Concurrency: every read returns a `version` (content hash). Writers may
//! pass `if_version`; a mismatch is rejected so an agent and a human editing
//! the same note never silently clobber each other. Writes are atomic
//! (temp file + rename).

use crate::markdown;
use crate::{Error, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NoteTarget {
    Repo,
    Branch(String),
    Commit(String),
    Task(String),
}

impl NoteTarget {
    /// Parse `repo`, `branch:<name>`, `commit:<sha>`, `task:<KEY>`.
    pub fn parse(s: &str) -> Result<NoteTarget> {
        let s = s.trim();
        if s == "repo" {
            return Ok(NoteTarget::Repo);
        }
        let (kind, val) = s
            .split_once(':')
            .ok_or_else(|| Error(format!("bad note target '{s}': expected repo | branch:<name> | commit:<sha> | task:<key>")))?;
        let val = val.trim();
        if val.is_empty() {
            bail!("empty note target value");
        }
        Ok(match kind {
            "branch" => NoteTarget::Branch(val.to_string()),
            "commit" => {
                if val.len() < 4 || !val.chars().all(|c| c.is_ascii_hexdigit()) {
                    bail!("commit target must be a hex sha");
                }
                NoteTarget::Commit(val.to_ascii_lowercase())
            }
            "task" => NoteTarget::Task(val.to_string()),
            _ => bail!("unknown note target kind '{kind}'"),
        })
    }

    fn rel_path(&self) -> PathBuf {
        match self {
            NoteTarget::Repo => PathBuf::from("repo.md"),
            NoteTarget::Branch(b) => Path::new("branch").join(format!("{}.md", encode(b))),
            NoteTarget::Commit(c) => Path::new("commit").join(format!("{c}.md")),
            NoteTarget::Task(t) => Path::new("task").join(format!("{}.md", encode(t))),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            NoteTarget::Repo => "repo",
            NoteTarget::Branch(_) => "branch",
            NoteTarget::Commit(_) => "commit",
            NoteTarget::Task(_) => "task",
        }
    }
}

impl std::fmt::Display for NoteTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoteTarget::Repo => f.write_str("repo"),
            NoteTarget::Branch(b) => write!(f, "branch:{b}"),
            NoteTarget::Commit(c) => write!(f, "commit:{c}"),
            NoteTarget::Task(t) => write!(f, "task:{t}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Note {
    pub target: NoteTarget,
    pub content: String,
    pub version: String,
    pub modified: i64,
}

#[derive(Debug, Clone)]
pub struct NoteMeta {
    pub target: NoteTarget,
    pub modified: i64,
    pub bytes: u64,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub target: NoteTarget,
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Links {
    #[serde(default)]
    branch_tasks: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct NoteStore {
    root: PathBuf,
}

impl NoteStore {
    pub fn for_repo(repo: &crate::git::Repo) -> Result<NoteStore> {
        NoteStore::at(repo.common_dir.join("margin").join("notes"))
    }

    pub fn at(root: impl Into<PathBuf>) -> Result<NoteStore> {
        let root = root.into();
        for d in ["branch", "commit", "task"] {
            fs::create_dir_all(root.join(d))?;
        }
        Ok(NoteStore { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read(&self, t: &NoteTarget) -> Result<Option<Note>> {
        let p = self.root.join(t.rel_path());
        match fs::read_to_string(&p) {
            Ok(content) => Ok(Some(Note {
                target: t.clone(),
                version: version_of(&content),
                modified: mtime(&p),
                content,
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Version token of the current content ("" if the note does not exist).
    pub fn current_version(&self, t: &NoteTarget) -> Result<String> {
        Ok(self.read(t)?.map(|n| n.version).unwrap_or_default())
    }

    /// Replace a note. Empty content deletes it.
    /// `if_version`: `Some("")` = must not exist yet; `Some(v)` = must match.
    pub fn write(&self, t: &NoteTarget, content: &str, if_version: Option<&str>) -> Result<Note> {
        if let Err(v) = markdown::validate(content) {
            bail!("note rejected: {}", markdown::format_violations(&v));
        }
        if let Some(expected) = if_version {
            let cur = self.current_version(t)?;
            if cur != expected {
                bail!("version conflict: note '{t}' changed (current version '{cur}'); re-read and merge");
            }
        }
        let p = self.root.join(t.rel_path());
        if content.trim().is_empty() {
            let _ = fs::remove_file(&p);
            return Ok(Note { target: t.clone(), content: String::new(), version: String::new(), modified: 0 });
        }
        atomic_write(&p, content.as_bytes())?;
        Ok(Note { target: t.clone(), content: content.to_string(), version: version_of(content), modified: mtime(&p) })
    }

    /// Append a timestamped section. Safe for many agents at once
    /// (retries on version conflict).
    pub fn append(&self, t: &NoteTarget, text: &str, author: &str) -> Result<Note> {
        if let Err(v) = markdown::validate(text) {
            bail!("note rejected: {}", markdown::format_violations(&v));
        }
        for _ in 0..5 {
            let cur = self.read(t)?;
            let (base, ver) = cur.map(|n| (n.content, n.version)).unwrap_or_default();
            let mut next = base.trim_end().to_string();
            if !next.is_empty() {
                next.push_str("\n\n");
            }
            next.push_str(&format!(
                "### {} · {}\n\n{}\n",
                crate::format_utc(crate::time::now_unix()),
                author,
                text.trim()
            ));
            match self.write(t, &next, Some(&ver)) {
                Ok(n) => return Ok(n),
                Err(e) if e.0.starts_with("version conflict") => continue,
                Err(e) => return Err(e),
            }
        }
        bail!("could not append: note keeps changing")
    }

    pub fn delete(&self, t: &NoteTarget) -> Result<()> {
        match fs::remove_file(self.root.join(t.rel_path())) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Move a note to another target. If the destination already has a note,
    /// the moved text is appended under a "Moved from …" marker (nothing is lost).
    pub fn move_note(&self, from: &NoteTarget, to: &NoteTarget) -> Result<Note> {
        if from == to {
            bail!("source and destination are the same");
        }
        let Some(src) = self.read(from)? else { bail!("note '{from}' does not exist") };
        let note = match self.read(to)? {
            None => self.write(to, &src.content, Some(""))?,
            Some(dst) => {
                let merged = format!(
                    "{}\n\n---\n\n_Moved from `{from}`:_\n\n{}\n",
                    dst.content.trim_end(),
                    src.content.trim()
                );
                self.write(to, &merged, Some(&dst.version))?
            }
        };
        self.delete(from)?;
        Ok(note)
    }

    /// Carry a branch's note and task link over to its new name.
    pub fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        let from = NoteTarget::Branch(old.to_string());
        if self.exists(&from) {
            self.move_note(&from, &NoteTarget::Branch(new.to_string()))?;
        }
        let mut l = self.links();
        if let Some(task) = l.branch_tasks.remove(old) {
            l.branch_tasks.entry(new.to_string()).or_insert(task);
            atomic_write(&self.root.join("links.json"), serde_json::to_string_pretty(&l)?.as_bytes())?;
        }
        Ok(())
    }

    pub fn exists(&self, t: &NoteTarget) -> bool {
        self.root.join(t.rel_path()).exists()
    }

    pub fn list(&self) -> Result<Vec<NoteMeta>> {
        let mut out = Vec::new();
        let mut push = |t: NoteTarget, p: &Path| {
            if let Ok(meta) = fs::metadata(p) {
                let title = fs::read_to_string(p)
                    .ok()
                    .and_then(|c| c.lines().find(|l| !l.trim().is_empty()).map(|l| l.trim_start_matches('#').trim().to_string()))
                    .unwrap_or_default();
                out.push(NoteMeta { target: t, modified: mtime(p), bytes: meta.len(), title });
            }
        };
        let repo = self.root.join("repo.md");
        if repo.exists() {
            push(NoteTarget::Repo, &repo);
        }
        for (dir, mk) in [
            ("branch", NoteTarget::Branch as fn(String) -> NoteTarget),
            ("commit", NoteTarget::Commit),
            ("task", NoteTarget::Task),
        ] {
            let Ok(rd) = fs::read_dir(self.root.join(dir)) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(stem) = name.strip_suffix(".md") {
                    push(mk(decode(stem)), &e.path());
                }
            }
        }
        out.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(out)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let q = query.to_lowercase();
        let mut hits = Vec::new();
        for m in self.list()? {
            if let Some(n) = self.read(&m.target)? {
                for (i, line) in n.content.lines().enumerate() {
                    if line.to_lowercase().contains(&q) {
                        hits.push(SearchHit { target: m.target.clone(), line: i + 1, text: line.trim().to_string() });
                        if hits.len() >= limit {
                            return Ok(hits);
                        }
                    }
                }
            }
        }
        Ok(hits)
    }

    /// Cheap change detector for the GUI (poll ~1/s): hash of names+mtimes+sizes.
    pub fn stamp(&self) -> u64 {
        let mut h = Fnv::new();
        for d in ["", "branch", "commit", "task"] {
            if let Ok(rd) = fs::read_dir(self.root.join(d)) {
                for e in rd.flatten() {
                    if let Ok(m) = e.metadata() {
                        h.write(e.file_name().as_encoded_bytes());
                        h.write(&m.len().to_le_bytes());
                        if let Ok(t) = m.modified() {
                            h.write(&format!("{t:?}").into_bytes());
                        }
                    }
                }
            }
        }
        h.finish()
    }

    // ---------------------------------------------------------- task links

    fn links(&self) -> Links {
        fs::read_to_string(self.root.join("links.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn linked_task(&self, branch: &str) -> Option<String> {
        self.links().branch_tasks.get(branch).cloned()
    }

    /// Explicit link wins; otherwise infer from branch name (`feature/PROJ-12-x`).
    pub fn task_for_branch(&self, branch: &str) -> Option<String> {
        self.linked_task(branch).or_else(|| crate::tasks::extract_keys(branch).into_iter().next())
    }

    pub fn link_task(&self, branch: &str, task: Option<&str>) -> Result<()> {
        let mut l = self.links();
        match task {
            Some(t) => l.branch_tasks.insert(branch.to_string(), t.to_string()),
            None => l.branch_tasks.remove(branch),
        };
        atomic_write(&self.root.join("links.json"), serde_json::to_string_pretty(&l)?.as_bytes())
    }
}

fn atomic_write(p: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, p)?;
    Ok(())
}

fn mtime(p: &Path) -> i64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn version_of(s: &str) -> String {
    let mut h = Fnv::new();
    h.write(s.as_bytes());
    format!("{:016x}", h.finish())
}

/// Percent-encode everything that isn't safe in a filename.
fn encode(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    if o.starts_with('.') {
        o.replace_range(0..1, "%2E");
    }
    o
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> NoteStore {
        let d = std::env::temp_dir().join(format!("margin-notes-test-{}-{}", std::process::id(), crate::time::now_unix()));
        let _ = fs::remove_dir_all(&d);
        NoteStore::at(d).unwrap()
    }

    #[test]
    fn roundtrip_conflict_and_append() {
        let s = store();
        let t = NoteTarget::parse("branch:feature/PROJ-1-login").unwrap();
        let n = s.write(&t, "# Login\n", Some("")).unwrap();
        assert!(s.write(&t, "x", Some("")).is_err(), "must not exist check");
        assert!(s.write(&t, "x", Some("bogus")).is_err());
        s.append(&t, "agent finding", "claude").unwrap();
        let r = s.read(&t).unwrap().unwrap();
        assert!(r.content.starts_with("# Login\n\n### ") && r.content.contains("· claude\n\nagent finding"));
        assert_ne!(r.version, n.version);
        let list = s.list().unwrap();
        assert_eq!(list[0].target, t);
        assert_eq!(s.search("FINDING", 10).unwrap().len(), 1);
        assert_eq!(s.task_for_branch("feature/PROJ-1-login").as_deref(), Some("PROJ-1"));
        s.link_task("feature/PROJ-1-login", Some("OPS-9")).unwrap();
        assert_eq!(s.task_for_branch("feature/PROJ-1-login").as_deref(), Some("OPS-9"));
        assert!(s.write(&t, "![i](x.png)", None).is_err());

        // rename: note + link follow; merging into an existing note keeps both
        s.write(&NoteTarget::Branch("feature/new".into()), "existing", None).unwrap();
        s.rename_branch("feature/PROJ-1-login", "feature/new").unwrap();
        assert!(!s.exists(&t));
        let merged = s.read(&NoteTarget::Branch("feature/new".into())).unwrap().unwrap().content;
        assert!(merged.starts_with("existing") && merged.contains("_Moved from `branch:feature/PROJ-1-login`:_") && merged.contains("# Login"));
        assert_eq!(s.task_for_branch("feature/new").as_deref(), Some("OPS-9"));
    }

    #[test]
    fn encoding_roundtrip() {
        for n in ["feature/x", "a b%c", ".hidden", "유니코드"] {
            assert_eq!(decode(&encode(n)), n);
        }
    }
}
