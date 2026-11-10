//! Git access through the user's installed `git` binary.
//!
//! Why the CLI and not libgit2? Enterprise setups depend on things libgit2
//! does not do (or does differently): hooks (pre-commit, commit-msg),
//! GPG/SSH commit signing, credential helpers / SSO, `includeIf` configs,
//! LFS, partial clone, sparse checkout, fsmonitor. Shelling out keeps 100%
//! behavioural parity with the terminal — the same thing Sublime Merge does
//! for its write operations. All reads use machine-readable formats
//! (porcelain v2, `-z`, explicit `--format`) so they're cheap to parse.

use crate::{Error, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct Repo {
    pub workdir: PathBuf,
    /// The *common* git dir (shared by all worktrees). Notes live under it.
    pub common_dir: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub branch: Option<String>,
    pub head_oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub orig_path: Option<String>,
    /// Index (staged) status: '.', 'M', 'A', 'D', 'R', 'C', 'T', 'U'
    pub index: char,
    /// Worktree status, same alphabet; '?' = untracked.
    pub worktree: char,
    pub conflicted: bool,
}

impl FileChange {
    pub fn is_staged(&self) -> bool {
        !self.conflicted && self.index != '.' && self.index != '?'
    }
    pub fn is_unstaged(&self) -> bool {
        self.conflicted || self.worktree != '.'
    }
    pub fn is_untracked(&self) -> bool {
        self.worktree == '?'
    }
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub refs: Vec<String>,
    pub subject: String,
}

impl Commit {
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(8)]
    }
}

#[derive(Debug, Clone)]
pub struct CommitDetail {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub author_time: i64,
    pub committer: String,
    pub commit_time: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Debug, Clone)]
pub struct RefInfo {
    pub kind: RefKind,
    /// Short name: `main`, `origin/main`, `v1.0`.
    pub name: String,
    pub target: String,
    pub upstream: Option<String>,
    /// Remote of the upstream (`origin`) and its branch name on that remote.
    pub upstream_remote: Option<String>,
    pub upstream_branch: Option<String>,
    /// e.g. "ahead 2, behind 1"
    pub track: String,
    pub is_head: bool,
}

#[derive(Debug, Clone)]
pub struct Stash {
    pub name: String, // stash@{0}
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

impl Repo {
    /// Open the repository containing `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Repo> {
        let path = path.as_ref();
        let out = run_in(path, &["rev-parse", "--show-toplevel", "--git-common-dir"])?;
        let mut lines = out.lines();
        let top = lines.next().ok_or_else(|| Error("not a git repository".into()))?;
        let common = lines.next().ok_or_else(|| Error("not a git repository".into()))?;
        let workdir = PathBuf::from(top);
        let common_dir = {
            let p = PathBuf::from(common);
            if p.is_absolute() { p } else { path.join(p) }
        };
        let common_dir = common_dir.canonicalize().unwrap_or(common_dir);
        Ok(Repo { workdir, common_dir })
    }

    pub fn name(&self) -> String {
        self.workdir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.workdir.display().to_string())
    }

    fn cmd(&self) -> Command {
        base_cmd(&self.workdir)
    }

    /// Run git, require exit code 0, return stdout.
    pub fn run(&self, args: &[&str]) -> Result<String> {
        run_cmd(self.cmd().args(args), None, &[0])
    }

    fn run_ok_codes(&self, args: &[&str], ok: &[i32]) -> Result<String> {
        run_cmd(self.cmd().args(args), None, ok)
    }

    fn run_stdin(&self, args: &[&str], input: &[u8]) -> Result<String> {
        run_cmd(self.cmd().args(args), Some(input), &[0])
    }

    // ------------------------------------------------------------------ reads

    pub fn status(&self) -> Result<Status> {
        let out = self.run(&["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all"])?;
        Ok(parse_status(&out))
    }

    pub fn has_head(&self) -> bool {
        self.run(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok()
    }

    /// Commits reachable from all branches, remotes and tags, topo-ordered
    /// (what Sublime Merge shows by default).
    pub fn log(&self, limit: usize, skip: usize) -> Result<Vec<Commit>> {
        if !self.has_head() && self.run(&["for-each-ref", "--count=1"])?.trim().is_empty() {
            return Ok(vec![]);
        }
        let max = format!("--max-count={limit}");
        let skip = format!("--skip={skip}");
        let mut args = vec![
            "log",
            "--topo-order",
            "--branches",
            "--remotes",
            "--tags",
            &max,
            &skip,
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e",
        ];
        if self.has_head() {
            args.push("HEAD");
        }
        args.push("--");
        Ok(parse_log(&self.run(&args)?))
    }

    /// Log of a single revision / range (used by MCP).
    pub fn log_rev(&self, rev: &str, limit: usize) -> Result<Vec<Commit>> {
        let max = format!("--max-count={limit}");
        let out = self.run(&[
            "log",
            &max,
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e",
            rev,
            "--",
        ])?;
        Ok(parse_log(&out))
    }

    pub fn resolve(&self, rev: &str) -> Result<String> {
        let spec = format!("{rev}^{{commit}}");
        Ok(self.run(&["rev-parse", "--verify", "-q", &spec])
            .map_err(|_| Error(format!("unknown revision: {rev}")))?
            .trim()
            .to_string())
    }

    pub fn commit_detail(&self, id: &str) -> Result<CommitDetail> {
        let out = self.run(&[
            "show",
            "-s",
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ct%x1f%B",
            id,
        ])?;
        let f: Vec<&str> = out.splitn(8, '\x1f').collect();
        if f.len() < 8 {
            bail!("unexpected git show output");
        }
        Ok(CommitDetail {
            id: f[0].to_string(),
            parents: f[1].split_whitespace().map(String::from).collect(),
            author: f[2].to_string(),
            email: f[3].to_string(),
            author_time: f[4].parse().unwrap_or(0),
            committer: f[5].to_string(),
            commit_time: f[6].parse().unwrap_or(0),
            message: f[7].trim_end().to_string(),
        })
    }

    /// Patch of a commit vs its first parent (root commits diff against empty).
    pub fn commit_diff(&self, id: &str) -> Result<String> {
        self.run(&[
            "show",
            "--format=",
            "--no-color",
            "--no-ext-diff",
            "-M",
            "--diff-merges=first-parent",
            "--patch",
            id,
        ])
    }

    /// Working-tree diff for one file. `staged` = index vs HEAD.
    pub fn file_diff(&self, change: &FileChange, staged: bool) -> Result<String> {
        if !staged && change.is_untracked() {
            // `--no-index` exits 1 when files differ.
            return self.run_ok_codes(
                &["diff", "--no-color", "--no-ext-diff", "--no-index", "--", "/dev/null", &change.path],
                &[0, 1],
            );
        }
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-M"];
        if staged {
            args.push("--cached");
        }
        args.push("--");
        if let Some(o) = &change.orig_path {
            args.push(o);
        }
        args.push(&change.path);
        self.run(&args)
    }

    pub fn refs(&self) -> Result<Vec<RefInfo>> {
        let out = self.run(&[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%1f%(objectname)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(HEAD)%1f%(*objectname)%1f%(upstream:remotename)%1f%(upstream:remoteref)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?;
        let mut refs = Vec::new();
        for line in out.lines() {
            let f: Vec<&str> = line.split('\x1f').collect();
            if f.len() < 8 {
                continue;
            }
            let (kind, name) = if let Some(n) = f[0].strip_prefix("refs/heads/") {
                (RefKind::Local, n)
            } else if let Some(n) = f[0].strip_prefix("refs/remotes/") {
                if n.ends_with("/HEAD") {
                    continue;
                }
                (RefKind::Remote, n)
            } else if let Some(n) = f[0].strip_prefix("refs/tags/") {
                (RefKind::Tag, n)
            } else {
                continue;
            };
            refs.push(RefInfo {
                kind,
                name: name.to_string(),
                // Annotated tags point at a tag object; use the peeled commit.
                target: if f[5].is_empty() { f[1] } else { f[5] }.to_string(),
                upstream: (!f[2].is_empty()).then(|| f[2].to_string()),
                upstream_remote: (!f[6].is_empty()).then(|| f[6].to_string()),
                upstream_branch: f[7].strip_prefix("refs/heads/").map(String::from),
                track: f[3].to_string(),
                is_head: f[4] == "*",
            });
        }
        Ok(refs)
    }

    /// Which of `ids` still exist as commit objects (`git cat-file --batch-check`).
    pub fn existing_commits(&self, ids: &[String]) -> Result<std::collections::HashSet<String>> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let input = ids.iter().map(|i| format!("{i}\n")).collect::<String>();
        let out = self.run_stdin(&["cat-file", "--batch-check=%(objectname) %(objecttype)"], input.as_bytes())?;
        Ok(out
            .lines()
            .filter_map(|l| l.strip_suffix(" commit").map(String::from))
            .collect())
    }

    /// Is `id` contained in any branch, remote branch, tag, stash or HEAD?
    pub fn is_reachable(&self, id: &str) -> bool {
        let found = self
            .run(&["for-each-ref", "--count=1", "--format=x", "--contains", id, "refs/heads", "refs/remotes", "refs/tags", "refs/stash"])
            .map(|o| !o.trim().is_empty())
            .unwrap_or(false);
        found || self.run(&["merge-base", "--is-ancestor", id, "HEAD"]).is_ok()
    }

    pub fn subject_of(&self, id: &str) -> Option<String> {
        self.run(&["log", "-1", "--format=%s", id, "--"]).ok().map(|s| s.trim().to_string())
    }

    /// Branch renames recorded in the reflogs of the given local branches:
    /// `(old, new, unix_time)`, in the order they happened per branch.
    /// Git moves a branch's reflog on `branch -m`, so a branch's log holds its
    /// whole rename chain ("Branch: renamed refs/heads/a to refs/heads/b").
    pub fn branch_renames(&self, branches: &[String]) -> Vec<(String, String, i64)> {
        let mut out = Vec::new();
        for b in branches {
            let file = self.common_dir.join("logs/refs/heads").join(b);
            // (time, message) entries
            let msgs: Vec<(i64, String)> = match std::fs::read_to_string(&file) {
                // "<old> <new> Name <email> <unix-ts> <tz>\t<message>"
                Ok(t) => t
                    .lines()
                    .filter_map(|l| {
                        let (head, msg) = l.split_once('\t')?;
                        let ts = head.split_whitespace().rev().nth(1)?.parse().ok()?;
                        Some((ts, msg.to_string()))
                    })
                    .collect(),
                // reftable backend or reflog elsewhere: ask git (newest first → reverse).
                Err(_) => {
                    let r = format!("refs/heads/{b}");
                    let mut v: Vec<(i64, String)> = self
                        .run(&["reflog", "show", "--date=unix", "--format=%gd%x1f%gs", &r, "--"])
                        .map(|o| {
                            o.lines()
                                .filter_map(|l| {
                                    let (sel, msg) = l.split_once('\x1f')?;
                                    let ts = sel.rsplit_once("@{")?.1.trim_end_matches('}').parse().ok()?;
                                    Some((ts, msg.to_string()))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    v.reverse();
                    v
                }
            };
            for (ts, m) in msgs {
                if let Some(rest) = m.strip_prefix("Branch: renamed refs/heads/") {
                    if let Some((old, new)) = rest.split_once(" to refs/heads/") {
                        out.push((old.to_string(), new.trim().to_string(), ts));
                    }
                }
            }
        }
        out
    }

    /// `git patch-id --stable` for each commit: a hash of the change itself
    /// (ignores SHA, message, author, line offsets). Merge commits and empty
    /// commits have no patch and are absent from the result.
    pub fn patch_ids(&self, ids: &[String]) -> Result<std::collections::HashMap<String, String>> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let input = ids.iter().map(|i| format!("{i}\n")).collect::<String>();
        let patches = self.run_stdin(
            &["log", "--no-walk=unsorted", "--stdin", "-p", "--no-color", "--no-ext-diff", "--format=commit %H"],
            input.as_bytes(),
        )?;
        let out = self.run_stdin(&["patch-id", "--stable"], patches.as_bytes())?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once(' ').map(|(pid, c)| (c.trim().to_string(), pid.to_string())))
            .collect())
    }

    /// (author email, author time) — preserved by rebase, amend and cherry-pick.
    pub fn author_stamp(&self, id: &str) -> Option<(String, i64)> {
        let o = self.run(&["log", "-1", "--format=%ae%x1f%at", id, "--"]).ok()?;
        let (e, t) = o.trim().split_once('\x1f')?;
        Some((e.to_string(), t.parse().ok()?))
    }

    /// Non-merge commits on branches/tags/remotes/HEAD that are NOT ancestors
    /// of `old` — where a rewritten copy of `old` would be. Newest first.
    pub fn rewrite_candidates(&self, old: &str, limit: usize) -> Result<Vec<(String, String, i64)>> {
        let max = format!("--max-count={limit}");
        let not = format!("^{old}");
        let mut args = vec!["log", "--no-merges", &max, "--format=%H%x1f%ae%x1f%at", "--branches", "--tags", "--remotes"];
        if self.has_head() {
            args.push("HEAD");
        }
        args.extend_from_slice(&[&not, "--"]);
        Ok(self
            .run(&args)?
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\x1f').collect();
                (f.len() == 3).then(|| (f[0].to_string(), f[1].to_string(), f[2].parse().unwrap_or(0)))
            })
            .collect())
    }

    pub fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        self.run(&["branch", "-m", old, new]).map(drop)
    }

    pub fn remote_url(&self, remote: &str) -> Option<String> {
        let key = format!("remote.{remote}.url");
        self.run(&["config", "--get", &key]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    pub fn remotes(&self) -> Vec<String> {
        self.run(&["remote"]).map(|s| s.lines().map(String::from).collect()).unwrap_or_default()
    }

    pub fn stashes(&self) -> Result<Vec<Stash>> {
        let out = self.run(&["stash", "list", "--format=%gd%x1f%H%x1f%s"])?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.splitn(3, '\x1f').collect();
                (f.len() == 3).then(|| Stash {
                    name: f[0].into(),
                    id: f[1].into(),
                    message: f[2].into(),
                })
            })
            .collect())
    }

    pub fn last_commit_message(&self) -> Result<String> {
        Ok(self.run(&["log", "-1", "--format=%B"])?.trim_end().to_string())
    }

    /// In-progress operation, if any (merge / rebase / cherry-pick / revert).
    pub fn pending_operation(&self) -> Option<&'static str> {
        let gd = self.run(&["rev-parse", "--absolute-git-dir"]).ok()?;
        let gd = PathBuf::from(gd.trim());
        [
            ("rebase-merge", "rebase"),
            ("rebase-apply", "rebase"),
            ("MERGE_HEAD", "merge"),
            ("CHERRY_PICK_HEAD", "cherry-pick"),
            ("REVERT_HEAD", "revert"),
        ]
        .into_iter()
        .find(|(f, _)| gd.join(f).exists())
        .map(|(_, n)| n)
    }

    // ----------------------------------------------------------------- writes

    pub fn stage(&self, paths: &[&str]) -> Result<()> {
        let mut a = vec!["add", "-A", "--"];
        a.extend_from_slice(paths);
        self.run(&a).map(drop)
    }

    pub fn unstage(&self, paths: &[&str]) -> Result<()> {
        let mut a = if self.has_head() {
            vec!["restore", "--staged", "--"]
        } else {
            vec!["rm", "--cached", "-q", "-r", "--"]
        };
        a.extend_from_slice(paths);
        self.run(&a).map(drop)
    }

    /// Discard worktree changes. Untracked files are removed.
    pub fn discard(&self, changes: &[&FileChange]) -> Result<()> {
        let tracked: Vec<&str> = changes.iter().filter(|c| !c.is_untracked()).map(|c| c.path.as_str()).collect();
        let untracked: Vec<&str> = changes.iter().filter(|c| c.is_untracked()).map(|c| c.path.as_str()).collect();
        if !tracked.is_empty() {
            let mut a = vec!["restore", "--worktree", "--"];
            a.extend_from_slice(&tracked);
            self.run(&a)?;
        }
        if !untracked.is_empty() {
            let mut a = vec!["clean", "-f", "-q", "--"];
            a.extend_from_slice(&untracked);
            self.run(&a)?;
        }
        Ok(())
    }

    /// Apply a patch (used for hunk staging / unstaging / discarding).
    pub fn apply_patch(&self, patch: &str, cached: bool, reverse: bool) -> Result<()> {
        let mut a = vec!["apply", "--whitespace=nowarn"];
        if cached {
            a.push("--cached");
        }
        if reverse {
            a.push("--reverse");
        }
        a.push("-");
        self.run_stdin(&a, patch.as_bytes()).map(drop)
    }

    /// Commit with the user's own config: hooks run, signing applies.
    pub fn commit(&self, message: &str, amend: bool) -> Result<String> {
        let mut a = vec!["commit", "--cleanup=strip", "-F", "-"];
        if amend {
            a.push("--amend");
        }
        self.run_stdin(&a, message.as_bytes())
    }

    pub fn checkout(&self, branch: &str) -> Result<()> {
        self.run(&["switch", branch]).map(drop)
    }

    /// Check out a remote branch as a local tracking branch.
    pub fn checkout_remote(&self, remote_branch: &str) -> Result<()> {
        self.run(&["switch", "--track", remote_branch]).map(drop)
    }

    pub fn checkout_detached(&self, rev: &str) -> Result<()> {
        self.run(&["switch", "--detach", rev]).map(drop)
    }

    pub fn create_branch(&self, name: &str, start: Option<&str>, switch: bool) -> Result<()> {
        let mut a = if switch { vec!["switch", "-c", name] } else { vec!["branch", name] };
        if let Some(s) = start {
            a.push(s);
        }
        self.run(&a).map(drop)
    }

    pub fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        self.run(&["branch", if force { "-D" } else { "-d" }, name]).map(drop)
    }

    pub fn merge(&self, rev: &str) -> Result<String> {
        self.run(&["merge", "--no-edit", rev])
    }

    pub fn cherry_pick(&self, id: &str) -> Result<String> {
        self.run(&["cherry-pick", id])
    }

    pub fn revert(&self, id: &str) -> Result<String> {
        self.run(&["revert", "--no-edit", id])
    }

    pub fn reset(&self, rev: &str, mode: ResetMode) -> Result<()> {
        let m = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        };
        self.run(&["reset", "-q", m, rev]).map(drop)
    }

    pub fn create_tag(&self, name: &str, rev: &str) -> Result<()> {
        self.run(&["tag", name, rev]).map(drop)
    }

    pub fn stash_push(&self, message: Option<&str>) -> Result<()> {
        let mut a = vec!["stash", "push", "--include-untracked"];
        if let Some(m) = message {
            a.extend_from_slice(&["-m", m]);
        }
        self.run(&a).map(drop)
    }

    pub fn stash_pop(&self, name: &str) -> Result<()> {
        self.run(&["stash", "pop", name]).map(drop)
    }

    pub fn stash_apply(&self, name: &str) -> Result<()> {
        self.run(&["stash", "apply", name]).map(drop)
    }

    pub fn stash_drop(&self, name: &str) -> Result<()> {
        self.run(&["stash", "drop", name]).map(drop)
    }

    // --------------------------------------------------------------- network
    // Run these off the UI thread. GIT_TERMINAL_PROMPT=0 makes git fail fast
    // instead of hanging on a hidden password prompt; credential helpers
    // (osxkeychain, GCM, SSO helpers) and ssh-agent keep working.

    pub fn fetch(&self) -> Result<String> {
        self.run(&["fetch", "--all", "--prune", "--tags"])
    }

    pub fn pull(&self) -> Result<String> {
        self.run(&["pull"]) // honours pull.rebase / pull.ff from user config
    }

    pub fn push(&self, set_upstream: bool, force_with_lease: bool) -> Result<String> {
        let mut a = vec!["push"];
        if force_with_lease {
            a.push("--force-with-lease");
        }
        if set_upstream {
            a.extend_from_slice(&["-u", "origin", "HEAD"]);
        }
        self.run(&a)
    }
}

// ---------------------------------------------------------------- plumbing

fn base_cmd(dir: &Path) -> Command {
    let mut c = Command::new(git_binary());
    c.arg("-C")
        .arg(dir)
        // Never block on an invisible prompt.
        .env("GIT_TERMINAL_PROMPT", "0")
        // Don't fight the user's terminal for index.lock on read-only calls.
        .env("GIT_OPTIONAL_LOCKS", "0")
        // Stable, parseable output.
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// `MARGIN_GIT` lets enterprises pin a specific git (e.g. a vendored build).
fn git_binary() -> String {
    std::env::var("MARGIN_GIT").unwrap_or_else(|_| "git".into())
}

fn run_in(dir: &Path, args: &[&str]) -> Result<String> {
    run_cmd(base_cmd(dir).args(args), None, &[0])
}

fn run_cmd(cmd: &mut Command, input: Option<&[u8]>, ok: &[i32]) -> Result<String> {
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().map_err(|e| Error(format!("failed to run git: {e}")))?;
    if let Some(data) = input {
        // Write then drop stdin so git sees EOF.
        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin.write_all(data)?;
    }
    let out = child.wait_with_output()?;
    let code = out.status.code().unwrap_or(-1);
    if ok.contains(&code) {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = if err.trim().is_empty() { String::from_utf8_lossy(&out.stdout).into_owned() } else { err.into_owned() };
        Err(Error(msg.trim().to_string()))
    }
}

fn parse_status(out: &str) -> Status {
    let mut st = Status::default();
    let mut parts = out.split('\0').peekable();
    while let Some(entry) = parts.next() {
        if entry.is_empty() {
            continue;
        }
        if let Some(h) = entry.strip_prefix("# ") {
            if let Some(v) = h.strip_prefix("branch.oid ") {
                st.head_oid = (v != "(initial)").then(|| v.to_string());
            } else if let Some(v) = h.strip_prefix("branch.head ") {
                st.branch = (v != "(detached)").then(|| v.to_string());
            } else if let Some(v) = h.strip_prefix("branch.upstream ") {
                st.upstream = Some(v.to_string());
            } else if let Some(v) = h.strip_prefix("branch.ab ") {
                for t in v.split_whitespace() {
                    if let Some(n) = t.strip_prefix('+') {
                        st.ahead = n.parse().unwrap_or(0);
                    } else if let Some(n) = t.strip_prefix('-') {
                        st.behind = n.parse().unwrap_or(0);
                    }
                }
            }
            continue;
        }
        let kind = entry.as_bytes()[0];
        match kind {
            b'1' | b'2' | b'u' => {
                // fields: kind XY sub mH mI mW hH hI [X-score] path
                let nfields = match kind {
                    b'1' => 9,
                    b'2' => 10,
                    _ => 11,
                };
                let f: Vec<&str> = entry.splitn(nfields, ' ').collect();
                if f.len() < nfields {
                    continue;
                }
                let xy: Vec<char> = f[1].chars().collect();
                let orig = if kind == b'2' { parts.next().map(String::from) } else { None };
                st.files.push(FileChange {
                    path: f[nfields - 1].to_string(),
                    orig_path: orig,
                    index: xy[0],
                    worktree: xy[1],
                    conflicted: kind == b'u',
                });
            }
            b'?' => st.files.push(FileChange {
                path: entry[2..].to_string(),
                orig_path: None,
                index: '.',
                worktree: '?',
                conflicted: false,
            }),
            _ => {}
        }
    }
    st
}

fn parse_log(out: &str) -> Vec<Commit> {
    out.split('\x1e')
        .filter_map(|rec| {
            let rec = rec.trim_start_matches('\n');
            if rec.is_empty() {
                return None;
            }
            let f: Vec<&str> = rec.splitn(7, '\x1f').collect();
            if f.len() < 7 {
                return None;
            }
            Some(Commit {
                id: f[0].to_string(),
                parents: f[1].split_whitespace().map(String::from).collect(),
                author: f[2].to_string(),
                email: f[3].to_string(),
                time: f[4].parse().unwrap_or(0),
                refs: f[5]
                    .split(", ")
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect(),
                subject: f[6].trim_end().to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_v2() {
        let raw = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 aaa aaa src/a.rs\0\
2 R. N... 100644 100644 100644 aaa bbb R100 new name.rs\0old.rs\0\
u UU N... 100644 100644 100644 100644 a b c conflict.txt\0\
? new file.txt\0";
        let s = parse_status(raw);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!((s.ahead, s.behind), (2, 1));
        assert_eq!(s.files.len(), 4);
        assert!(s.files[0].is_unstaged() && !s.files[0].is_staged());
        assert_eq!(s.files[1].path, "new name.rs");
        assert_eq!(s.files[1].orig_path.as_deref(), Some("old.rs"));
        assert!(s.files[2].conflicted);
        assert!(s.files[3].is_untracked());
        assert_eq!(s.files[3].path, "new file.txt");
    }
}
