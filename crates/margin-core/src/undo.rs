//! Undo for git operations: what an operation changed in the local refs
//! (branches, tags, the stash list, which branch is checked out), and
//! putting it back.
//!
//! Margin records the refs before and after each operation. Undo restores
//! the "before" side, but only while the repository is exactly as the
//! operation left it, so it never throws away anything done since. Remote
//! refs aren't touched: a pull's fetched commits stay in `origin/…`.
//! Nothing is lost by an undo either: the commits it steps back from stay
//! in the reflog (and a redo puts them back).

use crate::git::{Repo, check_rev};
use crate::{Error, Result};
use std::collections::BTreeMap;

/// Which branch is checked out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    Detached(String),
}

/// The local refs at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefState {
    pub head: Head,
    /// `refs/heads/…` and `refs/tags/…` → object id.
    pub refs: BTreeMap<String, String>,
    /// The stash list, newest first: (commit id, message).
    pub stash: Vec<(String, String)>,
}

/// How undo moves the checked-out branch back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoMode {
    /// `reset --soft`: the commit's changes go back to the staging area
    /// (undoing a commit or an amend, or a soft reset).
    Soft,
    /// `reset --mixed`: the index follows, the working tree is kept (undoing a mixed reset).
    Mixed,
    /// `reset --keep`: index and files follow, refusing to overwrite
    /// uncommitted changes (merge, pull, cherry-pick, revert, rebase, hard reset).
    Keep,
}

/// One operation that can be undone.
#[derive(Clone, Debug)]
pub struct UndoPoint {
    /// What was done ("Commit", "Merge", "Delete branch").
    pub label: String,
    pub before: RefState,
    pub after: RefState,
    pub mode: UndoMode,
}

const ZERO: &str = "0000000000000000000000000000000000000000";

impl Repo {
    /// The local refs now.
    pub fn ref_state(&self) -> Result<RefState> {
        let head = match self.run(&["symbolic-ref", "-q", "HEAD"]) {
            Ok(s) if !s.trim().is_empty() => Head::Branch(s.trim().trim_start_matches("refs/heads/").to_string()),
            _ => Head::Detached(self.run(&["rev-parse", "-q", "--verify", "HEAD"]).unwrap_or_default().trim().to_string()),
        };
        let out = self.run(&["for-each-ref", "--format=%(refname)%00%(objectname)", "refs/heads", "refs/tags"])?;
        let refs = out.lines().filter_map(|l| l.split_once('\0')).map(|(r, o)| (r.to_string(), o.to_string())).collect();
        // (no stash yet: `refs/stash` doesn't exist, and the log fails)
        let stash = self
            .run(&["log", "-g", "--format=%H%x00%gs", "refs/stash", "--"])
            .map(|o| o.lines().filter_map(|l| l.split_once('\0')).map(|(h, s)| (h.to_string(), s.to_string())).collect())
            .unwrap_or_default();
        Ok(RefState { head, refs, stash })
    }

    /// Undo `p`: put the refs back as they were before it. Refuses (changing
    /// nothing) when the repository has changed since, or an operation is
    /// in progress. Returns a short description of what was restored.
    pub fn undo(&self, p: &UndoPoint) -> Result<String> {
        self.restore(&p.after, &p.before, p.mode, &p.label)
    }

    /// Redo `p` after an undo: forward from its "before" to its "after".
    pub fn redo(&self, p: &UndoPoint) -> Result<String> {
        self.restore(&p.before, &p.after, p.mode, &p.label)
    }

    fn restore(&self, from: &RefState, to: &RefState, mode: UndoMode, label: &str) -> Result<String> {
        if let Some(op) = self.pending_operation() {
            return Err(Error(format!("A {op} is in progress; finish or abort it first")));
        }
        let now = self.ref_state()?;
        if now != *from {
            return Err(Error(format!("The repository has changed since “{label}”, so undoing it could lose that work. Nothing was changed.")));
        }
        let cur = match &from.head {
            Head::Branch(b) => Some(format!("refs/heads/{b}")),
            Head::Detached(_) => None,
        };
        let changed: Vec<&String> = from.refs.keys().chain(to.refs.keys()).filter(|r| from.refs.get(*r) != to.refs.get(*r)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let set_ref = |r: &str| -> Result<()> {
            let old = from.refs.get(r).map_or(ZERO, String::as_str);
            match to.refs.get(r) {
                Some(new) => self.run(&["update-ref", "-m", &format!("margin: undo {label}"), r, new, old]).map(drop),
                None => self.run(&["update-ref", "-d", r, old]).map(drop),
            }
        };
        // 1. refs other than the checked-out branch: straight back
        for r in changed.iter().filter(|r| Some(r.as_str()) != cur.as_deref()) {
            set_ref(r)?;
        }
        // 2. the checked-out branch: moved back with reset, or switched away from
        if from.head != to.head {
            match &to.head {
                Head::Branch(b) => self.run(&["switch", "-q", b])?,
                Head::Detached(id) => {
                    check_rev(id)?;
                    self.run(&["switch", "-q", "--detach", id])?
                }
            };
            if let Some(c) = cur.as_deref().filter(|c| changed.iter().any(|r| r.as_str() == *c)) {
                set_ref(c)?;
            }
        } else if let Some(c) = cur.as_deref().filter(|c| changed.iter().any(|r| r.as_str() == *c)) {
            let Some(target) = to.refs.get(c) else {
                return Err(Error("The checked-out branch didn't exist before".into()));
            };
            let m = match mode {
                UndoMode::Soft => "--soft",
                UndoMode::Mixed => "--mixed",
                UndoMode::Keep => "--keep",
            };
            self.run(&["reset", "-q", m, target])?;
        }
        // 3. the stash list: entries that shouldn't be there go (a redo of a
        //    drop), dropped ones come back (oldest first, so the list ends
        //    up in its old order)
        for e in from.stash.iter().filter(|e| !to.stash.contains(e)) {
            let list = self.ref_state()?.stash;
            if let Some(i) = list.iter().position(|x| x == e) {
                self.run(&["stash", "drop", "-q", &format!("stash@{{{i}}}")])?;
            }
        }
        for (id, msg) in to.stash.iter().rev().filter(|e| !from.stash.contains(e)) {
            check_rev(id)?;
            self.run(&["stash", "store", "-m", msg, id])?;
        }
        Ok(describe(from, to))
    }
}

impl UndoPoint {
    /// A point for an operation, if it changed anything undo can restore.
    pub fn new(label: &str, before: RefState, after: RefState, mode: UndoMode) -> Option<UndoPoint> {
        // (new stash entries aren't undone: the stash pop / drop is the way back)
        let comparable = |s: &RefState| (s.head.clone(), s.refs.clone(), s.stash.iter().filter(|e| before.stash.contains(e)).cloned().collect::<Vec<_>>());
        (comparable(&before) != comparable(&after) && after.stash.len() <= before.stash.len()).then(|| UndoPoint { label: label.to_string(), before, after, mode })
    }

    /// What an undo would do, in a sentence.
    pub fn describe_undo(&self) -> String {
        describe(&self.after, &self.before)
    }

    /// What a redo would do.
    pub fn describe_redo(&self) -> String {
        describe(&self.before, &self.after)
    }
}

/// "develop back to 1a2b3c4d; feat/x restored" — from `from` to `to`.
fn describe(from: &RefState, to: &RefState) -> String {
    let short = |id: &str| id.chars().take(8).collect::<String>();
    let name = |r: &str| r.trim_start_matches("refs/heads/").trim_start_matches("refs/tags/").to_string();
    let mut parts = vec![];
    if from.head != to.head {
        parts.push(match &to.head {
            Head::Branch(b) => format!("{b} checked out"),
            Head::Detached(id) => format!("{} checked out", short(id)),
        });
    }
    let keys: std::collections::BTreeSet<&String> = from.refs.keys().chain(to.refs.keys()).collect();
    for r in keys {
        let kind = if r.starts_with("refs/tags/") { "tag " } else { "" };
        match (from.refs.get(r), to.refs.get(r)) {
            (Some(a), Some(b)) if a != b => parts.push(format!("{kind}{} at {}", name(r), short(b))),
            (Some(_), None) => parts.push(format!("{kind}{} removed", name(r))),
            (None, Some(_)) => parts.push(format!("{kind}{} restored", name(r))),
            _ => {}
        }
    }
    let back = to.stash.iter().filter(|e| !from.stash.contains(e)).count();
    if back > 0 {
        parts.push(if back == 1 { "stash restored".into() } else { format!("{back} stashes restored") });
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str) -> (std::path::PathBuf, Repo, impl Fn(&[&str]) -> String) {
        let d = std::env::temp_dir().join(format!("margin-undo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let d2 = d.clone();
        let g = move |args: &[&str]| {
            let o = std::process::Command::new("git").arg("-C").arg(&d2).args(["-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        g(&["init", "-q"]);
        g(&["config", "user.name", "Mina"]);
        g(&["config", "user.email", "mina@example.com"]);
        g(&["config", "core.autocrlf", "false"]);
        std::fs::write(d.join("a.txt"), "one\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-qm", "first"]);
        let r = Repo::open(&d).unwrap();
        (d, r, g)
    }

    /// Run `f` as an operation and return its undo point.
    fn op(r: &Repo, label: &str, mode: UndoMode, f: impl FnOnce()) -> UndoPoint {
        let before = r.ref_state().unwrap();
        f();
        UndoPoint::new(label, before, r.ref_state().unwrap(), mode).expect("something changed")
    }

    #[test]
    fn undo_commit_puts_changes_back_in_staging_and_redo_brings_it_back() {
        let (d, r, g) = repo("commit");
        let first = g(&["rev-parse", "HEAD"]);
        std::fs::write(d.join("a.txt"), "two\n").unwrap();
        g(&["add", "."]);
        let p = op(&r, "Commit", UndoMode::Soft, || {
            g(&["commit", "-qm", "second"]);
        });
        let second = g(&["rev-parse", "HEAD"]);
        assert_eq!(p.describe_undo(), format!("main at {}", &first[..8]));
        r.undo(&p).unwrap();
        assert_eq!(g(&["rev-parse", "HEAD"]), first);
        assert_eq!(g(&["status", "--porcelain"]), "M  a.txt"); // staged again
        r.redo(&p).unwrap();
        assert_eq!(g(&["rev-parse", "HEAD"]), second);
        assert_eq!(g(&["status", "--porcelain"]), "");
    }

    #[test]
    fn undo_merge_branch_delete_checkout_and_stash_drop() {
        let (d, r, g) = repo("ops");
        let main = g(&["rev-parse", "HEAD"]);
        g(&["switch", "-qc", "feat/x"]);
        std::fs::write(d.join("b.txt"), "b\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-qm", "feature"]);
        let feat = g(&["rev-parse", "HEAD"]);
        g(&["switch", "-q", "main"]);

        // merge: main moves forward; undo moves it back and the file goes
        let p = op(&r, "Merge", UndoMode::Keep, || {
            g(&["merge", "-q", "--no-ff", "-m", "merge", "feat/x"]);
        });
        r.undo(&p).unwrap();
        assert_eq!(g(&["rev-parse", "HEAD"]), main);
        assert!(!d.join("b.txt").exists());

        // delete a branch: undo restores it at the same commit
        let p = op(&r, "Delete branch", UndoMode::Keep, || {
            g(&["branch", "-qD", "feat/x"]);
        });
        assert_eq!(p.describe_undo(), "feat/x restored");
        r.undo(&p).unwrap();
        assert_eq!(g(&["rev-parse", "feat/x"]), feat);

        // create and switch: undo switches back and removes the new branch
        let p = op(&r, "Create branch", UndoMode::Keep, || {
            g(&["switch", "-qc", "topic"]);
        });
        r.undo(&p).unwrap();
        assert_eq!(g(&["branch", "--show-current"]), "main");
        assert!(g(&["branch", "--list", "topic"]).is_empty());

        // drop a stash: undo brings it back with its message
        std::fs::write(d.join("a.txt"), "wip\n").unwrap();
        g(&["stash", "push", "-qm", "my work"]);
        let p = op(&r, "Stash drop", UndoMode::Keep, || {
            g(&["stash", "drop", "-q"]);
        });
        r.undo(&p).unwrap();
        assert!(g(&["stash", "list"]).contains("my work"));
        r.redo(&p).unwrap();
        assert!(g(&["stash", "list"]).is_empty());
    }

    #[test]
    fn refuses_when_the_repository_changed_since() {
        let (d, r, g) = repo("changed");
        std::fs::write(d.join("a.txt"), "two\n").unwrap();
        g(&["add", "."]);
        let p = op(&r, "Commit", UndoMode::Soft, || {
            g(&["commit", "-qm", "second"]);
        });
        std::fs::write(d.join("a.txt"), "three\n").unwrap();
        g(&["commit", "-qam", "third"]);
        let head = g(&["rev-parse", "HEAD"]);
        assert!(r.undo(&p).is_err());
        assert_eq!(g(&["rev-parse", "HEAD"]), head);
    }

    #[test]
    fn nothing_to_undo_for_a_new_stash_or_no_change() {
        let (d, r, g) = repo("none");
        let before = r.ref_state().unwrap();
        assert!(UndoPoint::new("Fetch", before.clone(), r.ref_state().unwrap(), UndoMode::Keep).is_none());
        std::fs::write(d.join("a.txt"), "wip\n").unwrap();
        g(&["stash", "push", "-q"]);
        assert!(UndoPoint::new("Stash", before, r.ref_state().unwrap(), UndoMode::Keep).is_none());
    }
}
