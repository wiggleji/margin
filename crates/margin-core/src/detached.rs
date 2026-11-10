//! Detached notes and branch-rename following.
//!
//! * A **commit note** is bound to one SHA on purpose. After a rebase/amend
//!   (or a branch delete) that commit may no longer be on any ref: its note is
//!   "detached" and listed so it can be re-attached, merged or deleted.
//! * A **branch note** is bound to a branch name. When the branch is renamed
//!   (in Margin or with `git branch -m` in a terminal) the note moves to the
//!   new name automatically; renames are read from the reflog. If the branch
//!   was deleted, the note becomes detached.

use crate::git::{RefKind, Repo};
use crate::notes::{NoteMeta, NoteStore, NoteTarget};
use crate::Result;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Commit exists but no branch/tag/remote/stash/HEAD contains it
    /// (typically rebased or amended away).
    CommitUnreachable,
    /// Commit object is gone (garbage-collected, or from another clone).
    CommitMissing,
    /// No local branch with this name (deleted).
    BranchMissing,
}

impl Reason {
    pub fn describe(self) -> &'static str {
        match self {
            Reason::CommitUnreachable => "commit is no longer on any branch (rebased, amended or deleted)",
            Reason::CommitMissing => "commit no longer exists in this repository",
            Reason::BranchMissing => "branch no longer exists",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DetachedNote {
    pub target: String,
    #[serde(skip)]
    pub note: NoteTarget,
    pub title: String,
    pub modified: i64,
    pub reason: Reason,
    /// Subject of the old commit, when it still exists.
    pub subject: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Renamed {
    pub from: String,
    pub to: String,
}

/// Move branch notes of renamed branches to the new name. Returns the moves made.
pub fn follow_renames(repo: &Repo, store: &NoteStore) -> Result<Vec<Renamed>> {
    let locals = local_branches(repo)?;
    let orphans: Vec<(String, i64)> = store
        .list()?
        .into_iter()
        .filter_map(|m| match m.target {
            NoteTarget::Branch(b) if !locals.contains(&b) => Some((b, m.modified)),
            _ => None,
        })
        .collect();
    if orphans.is_empty() {
        return Ok(vec![]);
    }
    let names: Vec<String> = locals.iter().cloned().collect();
    // old name → (new name, when). Latest rename wins if a name was reused.
    let mut edges: HashMap<String, (String, i64)> = HashMap::new();
    for (old, new, ts) in repo.branch_renames(&names) {
        if edges.get(&old).is_none_or(|(_, t)| ts >= *t) {
            edges.insert(old, (new, ts));
        }
    }
    let mut moved = Vec::new();
    for (old, note_mtime) in orphans {
        // Only follow a rename that happened after the note was last written
        // (guards against an unrelated, older rename of a reused name).
        // 5 s slack for coarse filesystem timestamps.
        if edges.get(&old).is_none_or(|(_, ts)| *ts + 5 < note_mtime) {
            continue;
        }
        // Follow a→b→c chains to a branch that exists now.
        let mut cur = old.clone();
        let mut hops = 0;
        while let Some((next, _)) = edges.get(&cur) {
            cur = next.clone();
            hops += 1;
            if locals.contains(&cur) || hops > 10 {
                break;
            }
        }
        if cur != old && locals.contains(&cur) {
            store.rename_branch(&old, &cur)?;
            moved.push(Renamed { from: old, to: cur });
        }
    }
    Ok(moved)
}

/// Notes whose commit or branch is gone. `known_reachable` lets the GUI skip
/// commits it already knows are on a branch (avoids `git` calls).
pub fn find(repo: &Repo, store: &NoteStore, known_reachable: &HashSet<String>) -> Result<Vec<DetachedNote>> {
    let locals = local_branches(repo)?;
    let metas = store.list()?;
    let commit_ids: Vec<String> = metas
        .iter()
        .filter_map(|m| match &m.target {
            NoteTarget::Commit(c) if !known_reachable.contains(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let existing = repo.existing_commits(&commit_ids)?;

    let mut out = Vec::new();
    let mut push = |m: &NoteMeta, reason: Reason, subject: Option<String>| {
        out.push(DetachedNote {
            target: m.target.to_string(),
            note: m.target.clone(),
            title: m.title.clone(),
            modified: m.modified,
            reason,
            subject,
        })
    };
    for m in &metas {
        match &m.target {
            NoteTarget::Branch(b) if !locals.contains(b) => push(m, Reason::BranchMissing, None),
            NoteTarget::Commit(c) if !known_reachable.contains(c) => {
                if !existing.contains(c) {
                    push(m, Reason::CommitMissing, None);
                } else if !repo.is_reachable(c) {
                    push(m, Reason::CommitUnreachable, repo.subject_of(c));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn local_branches(repo: &Repo) -> Result<HashSet<String>> {
    Ok(repo.refs()?.into_iter().filter(|r| r.kind == RefKind::Local).map(|r| r.name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn renames_and_detached() {
        let d = std::env::temp_dir().join(format!("margin-detached-{}-{}", std::process::id(), crate::time::now_unix()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q"]);
        git(&d, &["commit", "-q", "--allow-empty", "-m", "base"]);
        git(&d, &["switch", "-q", "-c", "feat/a"]);
        git(&d, &["commit", "-q", "--allow-empty", "-m", "work"]);
        let repo = Repo::open(&d).unwrap();
        let store = NoteStore::for_repo(&repo).unwrap();
        let tip = repo.resolve("HEAD").unwrap();

        store.write(&NoteTarget::Branch("feat/a".into()), "# plan", None).unwrap();
        store.write(&NoteTarget::Commit(tip.clone()), "perf of this commit", None).unwrap();
        store.write(&NoteTarget::Branch("gone".into()), "old", None).unwrap();
        store.link_task("feat/a", Some("PROJ-1")).unwrap();

        // Rename twice in a terminal, then amend: branch note follows, commit note detaches.
        git(&d, &["branch", "-m", "feat/a", "feat/b"]);
        git(&d, &["branch", "-m", "feat/b", "feat/c"]);
        git(&d, &["commit", "-q", "--amend", "--allow-empty", "-m", "work v2"]);

        let moved = follow_renames(&repo, &store).unwrap();
        assert_eq!(moved.len(), 1);
        assert_eq!((moved[0].from.as_str(), moved[0].to.as_str()), ("feat/a", "feat/c"));
        assert_eq!(store.read(&NoteTarget::Branch("feat/c".into())).unwrap().unwrap().content, "# plan");
        assert_eq!(store.task_for_branch("feat/c").as_deref(), Some("PROJ-1"));

        let det = find(&repo, &store, &HashSet::new()).unwrap();
        let mut got: Vec<(String, Reason)> = det.iter().map(|d| (d.target.clone(), d.reason)).collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(got, vec![("branch:gone".into(), Reason::BranchMissing), (format!("commit:{tip}"), Reason::CommitUnreachable)]);
        assert_eq!(det.iter().find(|d| d.reason == Reason::CommitUnreachable).unwrap().subject.as_deref(), Some("work"));
        // A commit note on a reachable commit is not detached.
        let base = repo.resolve("main").unwrap();
        store.write(&NoteTarget::Commit(base), "fine", None).unwrap();
        assert_eq!(find(&repo, &store, &HashSet::new()).unwrap().len(), 2);
    }
}
