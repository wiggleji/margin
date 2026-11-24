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

impl RefInfo {
    /// `(ahead, behind)` vs the upstream, parsed from `%(upstream:track,nobracket)`
    /// ("ahead 1, behind 2" / "ahead 1" / "behind 2" / "gone" / "").
    pub fn ahead_behind(&self) -> (u32, u32) {
        let (mut a, mut b) = (0, 0);
        for part in self.track.split(", ") {
            if let Some(n) = part.strip_prefix("ahead ") {
                a = n.trim().parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                b = n.trim().parse().unwrap_or(0);
            }
        }
        (a, b)
    }

    /// The configured upstream branch no longer exists on the remote.
    pub fn upstream_gone(&self) -> bool {
        self.track.trim() == "gone"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    /// Short branch name; `None` when detached.
    pub branch: Option<String>,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
    /// The main worktree (first in `git worktree list`).
    pub main: bool,
}

#[derive(Debug, Clone)]
pub struct Stash {
    pub name: String, // stash@{0}
    pub id: String,
    /// What git shows: `On main: message` or `WIP on main: abc123 subject`.
    pub message: String,
    /// The message alone (`message`, or the commit subject for a WIP stash).
    pub title: String,
    /// The branch it was made on.
    pub branch: String,
    pub time: i64,
}

/// What `git stash push` takes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StashOptions {
    pub message: String,
    /// Also untracked files (with everything, or with `paths`).
    pub untracked: bool,
    /// Only what's staged (`--staged`): stage some hunks, then stash just those.
    pub staged_only: bool,
    /// Leave the staged changes in place as well (`--keep-index`).
    pub keep_staged: bool,
    /// Only these files (empty: all changes).
    pub paths: Vec<String>,
}

/// `On main: msg` → ("main", "msg"); `WIP on main: abc1234 subject` → ("main", "subject").
fn stash_title(message: &str) -> (String, String) {
    let (wip, rest) = match message.strip_prefix("WIP on ") {
        Some(r) => (true, r),
        None => (false, message.strip_prefix("On ").unwrap_or(message)),
    };
    match rest.split_once(": ") {
        Some((branch, m)) => {
            // a WIP stash: "abc1234 subject of HEAD"
            let m = if wip { m.split_once(' ').map_or(m, |(_, s)| s) } else { m };
            (branch.to_string(), if wip { format!("WIP: {m}") } else { m.to_string() })
        }
        None => (String::new(), message.to_string()),
    }
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
        let common_dir = crate::platform::canonical(&common_dir);
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

    /// Run git with extra environment variables (exit code 0 required).
    pub(crate) fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
        let mut c = self.cmd();
        c.args(args);
        for (k, v) in env {
            c.env(k, v);
        }
        run_cmd(&mut c, None, &[0])
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
        self.log_with(&["--branches".into(), "--remotes".into(), "--tags".into()], limit, skip)
    }

    /// History from `revs` (e.g. `--branches`, `--exclude=x`) plus HEAD.
    pub fn log_with(&self, revs: &[String], limit: usize, skip: usize) -> Result<Vec<Commit>> {
        if !self.has_head() && self.run(&["for-each-ref", "--count=1"])?.trim().is_empty() {
            return Ok(vec![]);
        }
        let max = format!("--max-count={limit}");
        let skip = format!("--skip={skip}");
        let mut args = vec!["log", "--topo-order", &max, &skip, "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e"];
        args.extend(revs.iter().map(String::as_str));
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

    /// What `head` brings in compared with `base`, from the point where it
    /// split off (`git diff base...head`): a pull request's "Files changed".
    pub fn range_diff(&self, base: &str, head: &str, ignore_ws: bool) -> Result<String> {
        check_rev(base)?;
        check_rev(head)?;
        let range = format!("{base}...{head}");
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-M"];
        if ignore_ws {
            args.push("-w");
        }
        args.extend([range.as_str(), "--"]);
        self.run(&args)
    }

    /// Commits on `head` that aren't on `base` (`base..head`), newest first:
    /// a pull request's "Commits".
    pub fn range_commits(&self, base: &str, head: &str, limit: usize) -> Result<Vec<Commit>> {
        check_rev(base)?;
        check_rev(head)?;
        self.log_rev(&format!("{base}..{head}"), limit)
    }

    /// Fetch pull request `number`'s head from `remote` into
    /// `refs/margin/pr/<number>` (no branch of yours is created or moved).
    /// Returns its commit.
    pub fn fetch_pr(&self, remote: &str, number: u64) -> Result<String> {
        check_rev(remote)?;
        let dst = format!("refs/margin/pr/{number}");
        self.run(&["fetch", "--no-tags", "--quiet", remote, &format!("+refs/pull/{number}/head:{dst}")])?;
        self.resolve(&dst)
    }

    /// Bring `remote`'s `branch` up to date in `refs/remotes/<remote>/<branch>`.
    pub fn fetch_branch(&self, remote: &str, branch: &str) -> Result<()> {
        check_rev(remote)?;
        check_rev(branch)?;
        self.run(&["fetch", "--no-tags", "--quiet", remote, &format!("+refs/heads/{branch}:refs/remotes/{remote}/{branch}")]).map(drop)
    }

    /// The branch comparisons start from: the remote's default branch
    /// (`origin/HEAD` → `main`), else main / master / develop if they exist.
    pub fn default_base(&self) -> Option<String> {
        if let Ok(r) = self.run(&["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]) {
            let r = r.trim();
            if let Some(name) = r.strip_prefix("origin/").filter(|n| self.resolve(n).is_ok()) {
                return Some(name.to_string());
            }
            if !r.is_empty() {
                return Some(r.to_string());
            }
        }
        ["main", "master", "develop", "trunk"].into_iter().find(|b| self.resolve(b).is_ok()).map(str::to_string)
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

    /// Branches (local first, then remote) whose history contains `id`.
    pub fn branches_containing(&self, id: &str) -> Result<Vec<String>> {
        let out = self.run(&["for-each-ref", "--contains", id, "--format=%(refname)", "refs/heads", "refs/remotes"])?;
        let mut local = vec![];
        let mut remote = vec![];
        for l in out.lines() {
            if let Some(b) = l.strip_prefix("refs/heads/") {
                local.push(b.to_string());
            } else if let Some(b) = l.strip_prefix("refs/remotes/") {
                if !b.ends_with("/HEAD") {
                    remote.push(b.to_string());
                }
            }
        }
        local.extend(remote);
        Ok(local)
    }

    /// Patch of a commit vs its first parent (root commits diff against empty).
    pub fn commit_diff(&self, id: &str) -> Result<String> {
        self.commit_diff_opts(id, false)
    }

    /// `commit_diff`, optionally ignoring whitespace changes (`git -w`).
    pub fn commit_diff_opts(&self, id: &str, ignore_ws: bool) -> Result<String> {
        let mut args = vec!["show", "--format=", "--no-color", "--no-ext-diff", "-M", "--diff-merges=first-parent", "--patch"];
        if ignore_ws {
            args.push("-w");
        }
        args.push(id);
        self.run(&args)
    }

    /// Diff of the whole working tree (`staged`: index vs HEAD, else worktree
    /// vs index). Untracked files aren't included; `exclude` paths are left out
    /// (e.g. conflicted files, whose combined diff isn't a normal patch).
    pub fn worktree_diff(&self, staged: bool, ignore_ws: bool, exclude: &[String]) -> Result<String> {
        let mut args: Vec<String> = ["diff", "--no-color", "--no-ext-diff", "-M"].map(String::from).to_vec();
        if staged {
            args.push("--cached".into());
        }
        if ignore_ws {
            args.push("-w".into());
        }
        args.push("--".into());
        if !exclude.is_empty() {
            args.push(".".into());
            args.extend(exclude.iter().map(|p| format!(":(exclude,literal){p}")));
        }
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&a)
    }

    /// Working-tree diff for one file. `staged` = index vs HEAD.
    pub fn file_diff(&self, change: &FileChange, staged: bool) -> Result<String> {
        self.file_diff_opts(change, staged, false)
    }

    /// `file_diff`, optionally ignoring whitespace changes (`git -w`). Hunks
    /// from such a diff can't be staged: they don't match the real file.
    pub fn file_diff_opts(&self, change: &FileChange, staged: bool, ignore_ws: bool) -> Result<String> {
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
        if ignore_ws {
            args.push("-w");
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
        let out = self.run(&["stash", "list", "--format=%gd%x1f%H%x1f%ct%x1f%s"])?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.splitn(4, '\x1f').collect();
                (f.len() == 4).then(|| {
                    let (branch, title) = stash_title(f[3]);
                    Stash { name: f[0].into(), id: f[1].into(), time: f[2].parse().unwrap_or(0), message: f[3].into(), title, branch }
                })
            })
            .collect())
    }

    pub fn last_commit_message(&self) -> Result<String> {
        Ok(self.run(&["log", "-1", "--format=%B"])?.trim_end().to_string())
    }

    /// Commits (on any branch, remote branch or tag) with an added or removed
    /// line containing `text` (exact, case-sensitive): `git log -G` with the
    /// text escaped. Newest first.
    pub fn commits_changing(&self, text: &str) -> Result<Vec<String>> {
        if text.is_empty() {
            return Ok(vec![]);
        }
        let g = format!("-G{}", ere_escape(text));
        let out = self.run(&["log", "--format=%H", "--no-ext-diff", "--extended-regexp", &g, "--branches", "--remotes", "--tags", "HEAD", "--"])?;
        Ok(out.lines().map(String::from).collect())
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

    // ------------------------------------------------------------- tags

    /// Annotated tag (signed automatically if `tag.gpgSign` is configured).
    pub fn create_annotated_tag(&self, name: &str, rev: &str, message: &str) -> Result<()> {
        self.run(&["tag", "-a", name, "-m", message, rev]).map(drop)
    }

    pub fn delete_tag(&self, name: &str) -> Result<()> {
        self.run(&["tag", "-d", name]).map(drop)
    }

    pub fn push_tag(&self, remote: &str, name: &str) -> Result<String> {
        let r = format!("refs/tags/{name}");
        self.run(&["push", remote, &r])
    }

    pub fn push_all_tags(&self, remote: &str) -> Result<String> {
        self.run(&["push", remote, "--tags"])
    }

    pub fn delete_remote_tag(&self, remote: &str, name: &str) -> Result<String> {
        let r = format!("refs/tags/{name}");
        self.run(&["push", remote, "--delete", &r])
    }

    // ---------------------------------------------------------- branches

    pub fn rebase_onto(&self, upstream: &str) -> Result<String> {
        self.run(&["rebase", upstream])
    }

    pub fn set_upstream(&self, branch: &str, upstream: &str) -> Result<()> {
        self.run(&["branch", "--set-upstream-to", upstream, branch]).map(drop)
    }

    pub fn push_branch(&self, remote: &str, branch: &str) -> Result<String> {
        self.run(&["push", "-u", remote, branch])
    }

    /// Update a branch that isn't checked out from its upstream, fast-forward
    /// only (`git fetch <remote> <remote-branch>:<local>`; refused otherwise).
    pub fn fast_forward(&self, remote: &str, remote_branch: &str, local: &str) -> Result<String> {
        let spec = format!("refs/heads/{remote_branch}:refs/heads/{local}");
        self.run(&["fetch", remote, &spec])
    }

    /// Push a branch (checked out or not) to its upstream branch.
    pub fn push_to_upstream(&self, remote: &str, local: &str, remote_branch: &str) -> Result<String> {
        let spec = format!("refs/heads/{local}:refs/heads/{remote_branch}");
        self.run(&["push", remote, &spec])
    }

    pub fn delete_remote_branch(&self, remote: &str, branch: &str) -> Result<String> {
        self.run(&["push", remote, "--delete", branch])
    }

    pub fn pull_rebase(&self) -> Result<String> {
        self.run(&["pull", "--rebase"])
    }

    pub fn fetch_remote(&self, remote: &str) -> Result<String> {
        self.run(&["fetch", "--prune", remote])
    }

    // ----------------------------------------------------------- remotes

    pub fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        self.run(&["remote", "add", name, url]).map(drop)
    }

    pub fn rename_remote(&self, old: &str, new: &str) -> Result<()> {
        self.run(&["remote", "rename", old, new]).map(drop)
    }

    pub fn remove_remote(&self, name: &str) -> Result<()> {
        self.run(&["remote", "remove", name]).map(drop)
    }

    pub fn set_remote_url(&self, name: &str, url: &str) -> Result<()> {
        self.run(&["remote", "set-url", name, url]).map(drop)
    }

    // --------------------------------------------------------- worktrees

    pub fn worktrees(&self) -> Result<Vec<Worktree>> {
        let out = self.run(&["worktree", "list", "--porcelain"])?;
        let mut v = Vec::new();
        for block in out.split("\n\n") {
            let mut w: Option<Worktree> = None;
            for line in block.lines() {
                if let Some(p) = line.strip_prefix("worktree ") {
                    w = Some(Worktree {
                        path: PathBuf::from(p),
                        head: None,
                        branch: None,
                        bare: false,
                        locked: false,
                        prunable: false,
                        main: v.is_empty(),
                    });
                } else if let Some(w) = w.as_mut() {
                    if let Some(h) = line.strip_prefix("HEAD ") {
                        w.head = Some(h.to_string());
                    } else if let Some(b) = line.strip_prefix("branch ") {
                        w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
                    } else if line == "bare" {
                        w.bare = true;
                    } else if line.starts_with("locked") {
                        w.locked = true;
                    } else if line.starts_with("prunable") {
                        w.prunable = true;
                    }
                }
            }
            v.extend(w);
        }
        Ok(v)
    }

    /// Add a worktree at `path` for an existing branch, or for a new branch
    /// `new_branch` created from `start`.
    pub fn add_worktree(&self, path: &str, branch: &str, new_branch: bool, start: Option<&str>) -> Result<String> {
        let mut a = vec!["worktree", "add"];
        if new_branch {
            a.extend_from_slice(&["-b", branch, path]);
            if let Some(s) = start {
                a.push(s);
            }
        } else {
            a.extend_from_slice(&[path, branch]);
        }
        self.run(&a)
    }

    pub fn remove_worktree(&self, path: &str, force: bool) -> Result<()> {
        let mut a = vec!["worktree", "remove"];
        if force {
            a.push("--force");
        }
        a.push(path);
        self.run(&a).map(drop)
    }

    pub fn prune_worktrees(&self) -> Result<String> {
        self.run(&["worktree", "prune", "-v"])
    }

    // -------------------------------------------------------- submodules

    pub fn submodule_update(&self) -> Result<String> {
        self.run(&["submodule", "update", "--init", "--recursive"])
    }

    pub fn submodule_sync(&self) -> Result<String> {
        self.run(&["submodule", "sync", "--recursive"])
    }

    // -------------------------------------- operation in progress (merge…)

    /// `continue`/`abort`/`skip` the in-progress merge, rebase, cherry-pick or revert.
    pub fn operation(&self, op: &str, verb: &str) -> Result<String> {
        match (op, verb) {
            // a merge is "continued" by committing (default message)
            ("merge", "continue") => self.run(&["commit", "--no-edit"]),
            ("merge", "abort") => self.run(&["merge", "--abort"]),
            ("rebase" | "cherry-pick" | "revert", "continue" | "abort" | "skip") => {
                let flag = format!("--{verb}");
                self.run(&[op, &flag])
            }
            _ => bail!("cannot {verb} {op}"),
        }
    }

    pub fn create_tag(&self, name: &str, rev: &str) -> Result<()> {
        self.run(&["tag", name, rev]).map(drop)
    }

    pub fn stash_push(&self, message: Option<&str>) -> Result<()> {
        self.stash_with(&StashOptions { message: message.unwrap_or_default().to_string(), untracked: true, ..Default::default() })
    }

    /// `git stash push` with these options.
    pub fn stash_with(&self, o: &StashOptions) -> Result<()> {
        let mut a: Vec<String> = vec!["stash".into(), "push".into()];
        if o.staged_only {
            a.push("--staged".into());
        } else {
            if o.untracked {
                a.push("--include-untracked".into());
            }
            if o.keep_staged {
                a.push("--keep-index".into());
            }
        }
        let m = o.message.trim();
        if !m.is_empty() {
            a.extend(["-m".into(), m.to_string()]);
        }
        if !o.paths.is_empty() && !o.staged_only {
            // git records the whole staging area in a stash of some files, so
            // staged changes to other files would ride along: build it exactly
            let staged = self.run(&["diff", "--cached", "--name-only", "-z"])?;
            if staged.split('\0').any(|f| !f.is_empty() && !o.paths.iter().any(|p| p == f)) {
                return self.stash_paths_exact(o);
            }
            a.push("--".into());
            a.extend(o.paths.iter().map(|p| format!(":(literal){p}")));
        }
        let out = self.run(&a.iter().map(String::as_str).collect::<Vec<_>>())?;
        if out.contains("No local changes to save") {
            bail!("nothing to stash");
        }
        Ok(())
    }

    /// A stash of just `o.paths` (their staged and unstaged changes, and the
    /// untracked ones if `o.untracked`), made like `git stash` makes one, then
    /// those files put back as they are in HEAD. Other files, staged or not,
    /// are left alone and kept out of the stash.
    fn stash_paths_exact(&self, o: &StashOptions) -> Result<()> {
        let dir = self.common_dir.join("margin");
        std::fs::create_dir_all(&dir)?;
        let idx = |n: &str| dir.join(format!("stash-index-{n}-{}", std::process::id()));
        let (i1, i2, i3) = (idx("i"), idx("w"), idx("u"));
        let cleanup = || {
            for f in [&i1, &i2, &i3] {
                let _ = std::fs::remove_file(f);
            }
        };
        let result = (|| -> Result<()> {
            let head = self.run(&["rev-parse", "--verify", "HEAD"])?.trim().to_string();
            let branch = self.run(&["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
            let untracked: Vec<String> = self.run(&["ls-files", "--others", "--exclude-standard", "-z", "--"])?.split('\0').filter(|f| !f.is_empty()).map(String::from).collect();
            let (new, tracked): (Vec<&String>, Vec<&String>) = o.paths.iter().partition(|p| untracked.contains(p));
            let lit = |p: &str| format!(":(literal){p}");
            let with = |f: &std::path::Path, args: &[&str]| self.run_env(args, &[("GIT_INDEX_FILE", f.to_str().unwrap_or_default())]);
            // the staging area: HEAD plus the picked files' staged versions
            with(&i1, &["read-tree", &head])?;
            for p in &tracked {
                let entry = self.run(&["ls-files", "-s", "--", &lit(p)])?;
                match entry.lines().next().and_then(|l| l.split_once('\t')).map(|(m, _)| m.split_whitespace().collect::<Vec<_>>()) {
                    Some(f) if f.len() >= 2 => {
                        with(&i1, &["update-index", "--add", "--cacheinfo", &format!("{},{},{p}", f[0], f[1])])?;
                    }
                    _ => {
                        with(&i1, &["update-index", "--force-remove", "--", p])?;
                    }
                }
            }
            let i_tree = with(&i1, &["write-tree"])?.trim().to_string();
            // the working tree: that, plus the picked files as they are now
            with(&i2, &["read-tree", &i_tree])?;
            for p in &tracked {
                if self.workdir.join(p.as_str()).exists() {
                    with(&i2, &["update-index", "--add", "--", p])?;
                } else {
                    with(&i2, &["update-index", "--force-remove", "--", p])?;
                }
            }
            let w_tree = with(&i2, &["write-tree"])?.trim().to_string();
            let short = self.run(&["log", "-1", "--format=%h %s", &head])?.trim().to_string();
            let m = o.message.trim();
            let msg = if m.is_empty() { format!("WIP on {branch}: {short}") } else { format!("On {branch}: {m}") };
            let i_commit = self.run(&["commit-tree", &i_tree, "-p", &head, "-m", &format!("index on {branch}: {short}")])?.trim().to_string();
            let mut parents = vec!["-p".to_string(), head.clone(), "-p".into(), i_commit];
            let new: Vec<&String> = if o.untracked { new } else { vec![] };
            if !new.is_empty() {
                with(&i3, &["read-tree", "--empty"])?;
                let mut a = vec!["update-index", "--add", "--"];
                a.extend(new.iter().map(|s| s.as_str()));
                with(&i3, &a)?;
                let u_tree = with(&i3, &["write-tree"])?.trim().to_string();
                let u = self.run(&["commit-tree", &u_tree, "-m", &format!("untracked files on {branch}: {short}")])?.trim().to_string();
                parents.extend(["-p".to_string(), u]);
            }
            let mut a: Vec<&str> = vec!["commit-tree", &w_tree];
            a.extend(parents.iter().map(String::as_str));
            a.extend(["-m", &msg]);
            let w = self.run(&a)?.trim().to_string();
            self.run(&["stash", "store", "-m", &msg, &w])?;
            // put the picked files back as they are in HEAD
            for p in &tracked {
                if self.run(&["cat-file", "-e", &format!("{head}:{p}")]).is_ok() {
                    self.run(&["restore", "--source", &head, "--staged", "--worktree", "--", &lit(p)])?;
                } else {
                    self.run(&["rm", "-q", "-f", "--cached", "--", &lit(p)])?;
                    let _ = std::fs::remove_file(self.workdir.join(p.as_str()));
                }
            }
            for p in &new {
                let _ = std::fs::remove_file(self.workdir.join(p.as_str()));
            }
            Ok(())
        })();
        cleanup();
        result
    }

    /// The changes a stash holds, as one patch: tracked changes (against the
    /// commit it was made on) and the untracked files it saved.
    pub fn stash_diff(&self, id: &str, ignore_ws: bool) -> Result<String> {
        check_rev(id)?;
        let base = format!("{id}^1");
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-M"];
        if ignore_ws {
            args.push("-w");
        }
        args.extend([base.as_str(), id]);
        let mut out = self.run(&args)?;
        let untracked = format!("{id}^3");
        if self.run(&["rev-parse", "--verify", "-q", &untracked]).is_ok() {
            out += &self.run(&["diff-tree", "--no-commit-id", "-p", "--no-color", "--no-ext-diff", "--root", "-r", &untracked])?;
        }
        Ok(out)
    }

    /// Check out a new branch at the commit the stash was made on, with the
    /// stash applied (and dropped if it applies cleanly).
    pub fn stash_branch(&self, branch: &str, name: &str) -> Result<()> {
        check_rev(branch)?;
        self.run(&["stash", "branch", branch, name]).map(drop)
    }

    /// Apply, restoring what was staged as staged (`--index`).
    pub fn stash_apply_index(&self, name: &str) -> Result<()> {
        self.run(&["stash", "apply", "--index", name]).map(drop)
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
    let mut c = crate::platform::command(git_binary());
    c.arg("-C")
        .arg(dir)
        // Paths as they are (한글.md, not "\355\225\234…"), in every language.
        .args(["-c", "core.quotepath=false"])
        // Never block on an invisible prompt.
        .env("GIT_TERMINAL_PROMPT", "0")
        // Don't fight the user's terminal for index.lock on read-only calls.
        .env("GIT_OPTIONAL_LOCKS", "0")
        // Stable, parseable output.
        .env("LC_ALL", "C")
        // No terminal to edit in: accept git's default message wherever it
        // would open an editor (rebase --continue, merge --continue, …).
        .env("GIT_EDITOR", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// `MARGIN_GIT` lets enterprises pin a specific git (e.g. a vendored build).
/// A revision given to git as an argument: refuse ones git would read as an option.
/// `s` as an extended regular expression matching it literally.
fn ere_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\.^$|?*+()[]{}".contains(c) {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

pub(crate) fn check_rev(rev: &str) -> Result<()> {
    if rev.is_empty() || rev.starts_with('-') || rev.contains(char::is_whitespace) {
        bail!("not a branch or commit: {rev:?}");
    }
    Ok(())
}

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
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error(format!("Margin needs git, and it isn't installed or isn't on PATH. Install it from https://git-scm.com/downloads{} (or set MARGIN_GIT to its full path).", if cfg!(windows) { " (Git for Windows)" } else { "" }))
        } else {
            Error(format!("failed to run git: {e}"))
        }
    })?;
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
    fn track_parsing() {
        let r = |t: &str| RefInfo {
            kind: RefKind::Local,
            name: "x".into(),
            target: String::new(),
            upstream: Some("origin/x".into()),
            upstream_remote: None,
            upstream_branch: None,
            track: t.into(),
            is_head: false,
        };
        assert_eq!(r("ahead 1, behind 1849").ahead_behind(), (1, 1849));
        assert_eq!(r("ahead 3").ahead_behind(), (3, 0));
        assert_eq!(r("behind 7").ahead_behind(), (0, 7));
        assert_eq!(r("").ahead_behind(), (0, 0));
        assert!(r("gone").upstream_gone() && r("gone").ahead_behind() == (0, 0));
    }

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

#[cfg(test)]
mod worktree_tests {
    use super::*;

    #[test]
    fn worktree_add_list_remove() {
        let d = std::env::temp_dir().join(format!("margin-wt-{}-{}", std::process::id(), crate::time::now_unix()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("repo")).unwrap();
        let r = d.join("repo");
        let g = |args: &[&str]| {
            assert!(std::process::Command::new("git").arg("-C").arg(&r)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
                .args(args).status().unwrap().success());
        };
        g(&["init", "-q"]);
        g(&["commit", "-q", "--allow-empty", "-m", "x"]);
        let repo = Repo::open(&r).unwrap();
        let wt = d.join("wt-feature");
        repo.add_worktree(wt.to_str().unwrap(), "feature/x", true, Some("main")).unwrap();
        let list = repo.worktrees().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].main && list[0].branch.as_deref() == Some("main"));
        assert_eq!(list[1].branch.as_deref(), Some("feature/x"));
        // The worktree is its own Repo sharing the same common dir (and notes).
        let other = Repo::open(&wt).unwrap();
        assert_eq!(other.common_dir, repo.common_dir);
        repo.remove_worktree(wt.to_str().unwrap(), false).unwrap();
        assert_eq!(repo.worktrees().unwrap().len(), 1);
        assert!(repo.operation("merge", "frobnicate").is_err());
    }
}

#[cfg(test)]
mod compare_tests {
    use super::*;

    /// main: a → c, feature (from a): b1 → b2. The comparison shows only what
    /// feature brings in (not c), and only its two commits.
    #[test]
    fn compare_branches_like_a_pull_request() {
        let d = std::env::temp_dir().join(format!("margin-cmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let g = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&d)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"])
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        let put = |f: &str, t: &str| std::fs::write(d.join(f), t).unwrap();
        g(&["init", "-q"]);
        put("a.txt", "one\n");
        g(&["add", "."]);
        g(&["commit", "-qm", "a"]);
        g(&["switch", "-qc", "feature"]);
        put("b.txt", "feature\n");
        g(&["add", "."]);
        g(&["commit", "-qm", "b1"]);
        put("a.txt", "one\ntwo\n");
        g(&["commit", "-qam", "b2"]);
        g(&["switch", "-q", "main"]);
        put("c.txt", "main only\n");
        g(&["add", "."]);
        g(&["commit", "-qm", "c"]);

        let repo = Repo::open(&d).unwrap();
        let diff = repo.range_diff("main", "feature", false).unwrap();
        assert!(diff.contains("b.txt") && diff.contains("+two"), "{diff}");
        assert!(!diff.contains("c.txt"), "main's own change isn't part of the comparison");
        let commits = repo.range_commits("main", "feature", 50).unwrap();
        assert_eq!(commits.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["b2", "b1"]);
        assert_eq!(repo.default_base().as_deref(), Some("main"));
        assert!(repo.range_diff("--output=x", "feature", false).is_err(), "options aren't revisions");

        // a pull request on a remote (GitHub keeps them as refs/pull/<n>/head)
        let remote = d.with_extension("remote.git");
        let _ = std::fs::remove_dir_all(&remote);
        let st = std::process::Command::new("git").args(["init", "-q", "--bare"]).arg(&remote).status().unwrap();
        assert!(st.success());
        g(&["remote", "add", "origin", &remote.display().to_string()]);
        g(&["push", "-q", "origin", "feature:refs/pull/7/head"]);
        let want = repo.resolve("feature").unwrap();
        assert_eq!(repo.fetch_pr("origin", 7).unwrap(), want);
        assert!(repo.run(&["show-ref", "--verify", "refs/heads/pr/7"]).is_err(), "no branch created");
        let _ = std::fs::remove_dir_all(&remote);
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod stash_tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(stash_title("On main: try knots"), ("main".into(), "try knots".into()));
        assert_eq!(stash_title("WIP on feature/x: abc1234 Add units"), ("feature/x".into(), "WIP: Add units".into()));
        assert_eq!(stash_title("odd"), (String::new(), "odd".into()));
    }

    /// Stash a picked file with an untracked one, then only staged changes;
    /// the stash's diff includes its untracked files; a branch from a stash.
    #[test]
    fn stash_files_staged_and_untracked() {
        let d = std::env::temp_dir().join(format!("margin-stash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let g = |args: &[&str]| {
            let o = std::process::Command::new("git").arg("-C").arg(&d).args(["-c", "init.defaultBranch=main"]).args(args).output().unwrap();
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        };
        let put = |f: &str, t: &str| std::fs::write(d.join(f), t).unwrap();
        g(&["init", "-q"]);
        g(&["config", "user.name", "t"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "commit.gpgsign", "false"]);
        g(&["config", "core.autocrlf", "false"]); // (Windows runners may convert line endings)
        put("a.txt", "a\n");
        put("b.txt", "b\n");
        g(&["add", "."]);
        g(&["commit", "-qm", "base"]);
        let repo = Repo::open(&d).unwrap();

        // only a.txt and the new file; b.txt stays
        put("a.txt", "a changed\n");
        put("b.txt", "b changed\n");
        put("new.txt", "new\n");
        repo.stash_with(&StashOptions { message: "a and new".into(), untracked: true, paths: vec!["a.txt".into(), "new.txt".into()], ..Default::default() }).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "a\n");
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "b changed\n");
        assert!(!d.join("new.txt").exists());
        let s = repo.stashes().unwrap();
        assert_eq!((s[0].title.as_str(), s[0].branch.as_str()), ("a and new", "main"));
        let diff = repo.stash_diff(&s[0].id, false).unwrap();
        assert!(diff.contains("+a changed") && diff.contains("new.txt") && diff.contains("+new") && !diff.contains("b changed"), "{diff}");
        assert!(diff.lines().all(|l| l.len() != 40 || !l.chars().all(|c| c.is_ascii_hexdigit())), "no commit id lines: {diff}");

        // some files while another is staged: that one stays out of the stash (and staged)
        put("c.txt", "c\n");
        g(&["add", "c.txt"]);
        put("a.txt", "a picked\n");
        put("other-new.txt", "x\n");
        repo.stash_with(&StashOptions { message: "just a".into(), untracked: true, paths: vec!["a.txt".into(), "other-new.txt".into()], ..Default::default() }).unwrap();
        let st = repo.stashes().unwrap();
        assert_eq!(st[0].title, "just a");
        let diff = repo.stash_diff(&st[0].id, false).unwrap();
        assert!(diff.contains("+a picked") && diff.contains("other-new.txt") && !diff.contains("c.txt") && !diff.contains("b changed"), "{diff}");
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "a\n");
        assert!(!d.join("other-new.txt").exists());
        assert!(repo.run(&["diff", "--cached", "--name-only"]).unwrap().contains("c.txt"), "c.txt still staged");
        // it pops back like any stash
        repo.stash_pop("stash@{0}").unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "a picked\n");
        assert!(d.join("other-new.txt").exists());
        g(&["checkout", "-q", "--", "a.txt"]);
        g(&["rm", "-q", "-f", "--cached", "c.txt"]);
        std::fs::remove_file(d.join("c.txt")).unwrap();
        std::fs::remove_file(d.join("other-new.txt")).unwrap();

        // only what's staged
        g(&["add", "b.txt"]);
        put("a.txt", "a again\n");
        repo.stash_with(&StashOptions { message: "staged b".into(), staged_only: true, ..Default::default() }).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "b\n");
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "a again\n");
        assert_eq!(repo.stashes().unwrap().len(), 2);

        // nothing to stash
        g(&["checkout", "-q", "--", "a.txt"]);
        assert!(repo.stash_with(&StashOptions::default()).is_err());

        // a branch from the older stash: applied there, and dropped
        repo.stash_branch("from-stash", "stash@{1}").unwrap();
        assert_eq!(repo.run(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap().trim(), "from-stash");
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "a changed\n");
        assert!(d.join("new.txt").exists());
        assert_eq!(repo.stashes().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod code_search_tests {
    use super::*;

    #[test]
    fn commits_changing_a_text() {
        assert_eq!(ere_escape("a.b(c)*"), r"a\.b\(c\)\*");
        let d = std::env::temp_dir().join(format!("margin-codesearch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let g = |args: &[&str]| {
            let o = std::process::Command::new("git").arg("-C").arg(&d).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        let put = |t: &str| std::fs::write(d.join("a.txt"), t).unwrap();
        g(&["init", "-q"]);
        put("LEAD = 30\n");
        g(&["add", "."]);
        g(&["commit", "-qm", "add"]);
        let add = g(&["rev-parse", "HEAD"]);
        put("LEAD = 45\n");
        g(&["commit", "-qam", "change the value"]);
        let change = g(&["rev-parse", "HEAD"]);
        put("LEAD = 45\nother(x)\n");
        g(&["commit", "-qam", "unrelated"]);
        let other = g(&["rev-parse", "HEAD"]);
        let repo = Repo::open(&d).unwrap();
        // a line containing it was added or removed: the edit counts too
        assert_eq!(repo.commits_changing("LEAD =").unwrap(), [change.clone(), add.clone()]);
        // regex characters are plain text; case matters
        assert_eq!(repo.commits_changing("other(x)").unwrap(), [other]);
        assert!(repo.commits_changing("lead =").unwrap().is_empty());
        assert!(repo.commits_changing("").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
