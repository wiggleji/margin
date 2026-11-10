//! Detached notes and branch-rename following.
//!
//! * A **commit note** is bound to one SHA on purpose. After a rebase/amend
//!   (or a branch delete) that commit may no longer be on any ref: its note is
//!   "detached" and listed so it can be re-attached, merged or deleted.
//! * A **branch note** is bound to a branch name. When the branch is renamed
//!   (in Margin or with `git branch -m` in a terminal) the note moves to the
//!   new name automatically; renames are read from the reflog. If the branch
//!   was deleted, the note becomes detached.
//! * **Auto re-attach**: a detached commit note whose change still exists as
//!   a *different* commit (rebase, cherry-pick, message-only amend) is moved
//!   to it when `git patch-id` finds exactly one match. If the change itself
//!   was edited (amend with new content, squash), it stays detached.

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
    /// Commits with the same patch-id when there was more than one (ambiguous:
    /// not moved automatically, offered as suggestions).
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reattached {
    /// Journal id — pass to `journal::undo`.
    pub id: String,
    pub from: String,
    pub to: String,
    /// "patch-id"
    pub how: &'static str,
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
            candidates: vec![],
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

/// Candidates scanned by patch-id when no same-author-date commit matched.
const FALLBACK_SCAN: usize = 200;

/// Move detached commit notes to their rewritten commit when `git patch-id`
/// identifies exactly one. Re-attached notes are removed from `detached`;
/// ambiguous ones get `candidates`.
pub fn auto_reattach(repo: &Repo, store: &NoteStore, detached: &mut Vec<DetachedNote>) -> Result<Vec<Reattached>> {
    let mut done = Vec::new();
    for d in detached.iter_mut() {
        let NoteTarget::Commit(old) = &d.note else { continue };
        if d.reason != Reason::CommitUnreachable {
            continue; // the old object must exist to compute its patch-id
        }
        // Never re-apply a move the user undid.
        let matches: Vec<String> = find_rewrites(repo, old)?
            .into_iter()
            .filter(|m| !crate::journal::is_rejected(store, &d.target, &format!("commit:{m}")))
            .collect();
        match matches.as_slice() {
            [new] => {
                let to = NoteTarget::Commit(new.clone());
                // Another process (GUI vs MCP) may have moved it already: skip, don't fail the scan.
                if let Ok(id) = crate::journal::move_recorded(store, &d.note, &to) {
                    done.push(Reattached { id, from: d.target.clone(), to: to.to_string(), how: "patch-id" });
                }
            }
            [] => {}
            many => d.candidates = many.to_vec(),
        }
    }
    let moved: HashSet<&str> = done.iter().map(|r| r.from.as_str()).collect();
    detached.retain(|d| !moved.contains(d.target.as_str()));
    Ok(done)
}

/// Commits on current refs carrying the same change as `old`.
pub fn find_rewrites(repo: &Repo, old: &str) -> Result<Vec<String>> {
    let Some(pid) = repo.patch_ids(&[old.to_string()])?.remove(old) else {
        return Ok(vec![]); // merge or empty commit: no patch-id
    };
    let cands = repo.rewrite_candidates(old, 5000)?;
    let stamp = repo.author_stamp(old);
    // 1) Rebase/amend/cherry-pick keep author email + date: check those first (usually 1).
    let (same, rest): (Vec<_>, Vec<_>) = cands.into_iter().partition(|(_, e, t)| stamp.as_ref().is_some_and(|(se, st)| se == e && st == t));
    let matching = |ids: Vec<String>| -> Result<Vec<String>> {
        let pids = repo.patch_ids(&ids)?;
        Ok(ids.into_iter().filter(|i| pids.get(i) == Some(&pid)).collect())
    };
    let m = matching(same.into_iter().map(|c| c.0).collect())?;
    if !m.is_empty() {
        return Ok(m);
    }
    // 2) Author date was reset (e.g. `rebase --reset-author-date`): scan recent commits.
    matching(rest.into_iter().take(FALLBACK_SCAN).map(|c| c.0).collect())
}

#[cfg(test)]
mod reattach_tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    #[test]
    fn patch_id_reattach() {
        let d = std::env::temp_dir().join(format!("margin-reattach-{}-{}", std::process::id(), crate::time::now_unix()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let w = |f: &str, c: &str| std::fs::write(d.join(f), c).unwrap();
        git(&d, &["init", "-q"]);
        w("a.txt", "1\n");
        git(&d, &["add", "."]);
        git(&d, &["commit", "-qm", "base"]);
        git(&d, &["switch", "-qc", "feat"]);
        w("a.txt", "1\n2\n");
        git(&d, &["commit", "-qam", "add 2"]);
        let c1 = git(&d, &["rev-parse", "HEAD"]);
        w("b.txt", "x\n");
        git(&d, &["add", "."]);
        git(&d, &["commit", "-qm", "add b"]);
        let c2 = git(&d, &["rev-parse", "HEAD"]);
        git(&d, &["switch", "-q", "main"]);
        w("c.txt", "main work\n");
        git(&d, &["add", "."]);
        git(&d, &["commit", "-qm", "main work"]);

        let repo = Repo::open(&d).unwrap();
        let store = NoteStore::for_repo(&repo).unwrap();
        store.write(&NoteTarget::Commit(c1.clone()), "note on add 2", None).unwrap();
        store.write(&NoteTarget::Commit(c2.clone()), "note on add b", None).unwrap();

        // Rebase feat onto main → both commits rewritten with identical changes.
        git(&d, &["switch", "-q", "feat"]);
        git(&d, &["rebase", "-q", "main"]);
        // Then amend the tip WITH a content change → its patch-id differs.
        w("b.txt", "x\ny\n");
        git(&d, &["commit", "-qa", "--amend", "-m", "add b (edited)"]);
        let new_c1 = git(&d, &["rev-parse", "HEAD~1"]);

        let mut det = find(&repo, &store, &HashSet::new()).unwrap();
        assert_eq!(det.len(), 2);
        let moved = auto_reattach(&repo, &store, &mut det).unwrap();
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert_eq!(moved[0].to, format!("commit:{new_c1}"));
        assert_eq!(store.read(&NoteTarget::Commit(new_c1)).unwrap().unwrap().content, "note on add 2");
        // The edited commit's note stays detached for a human to decide.
        assert_eq!(det.len(), 1);
        assert_eq!(det[0].target, format!("commit:{c2}"));

        // Undo: note goes back to the old commit, destination removed,
        // and the next scan does not re-apply the same move.
        let active = crate::journal::active(&store);
        assert_eq!(active.len(), 1);
        let out = crate::journal::undo(&store, &moved[0].id).unwrap();
        assert!(!out.kept_edited_destination);
        assert_eq!(store.read(&NoteTarget::Commit(c1.clone())).unwrap().unwrap().content, "note on add 2");
        assert!(store.read(&NoteTarget::Commit(git(&d, &["rev-parse", "HEAD~1"]))).unwrap().is_none());
        let mut det = find(&repo, &store, &HashSet::new()).unwrap();
        assert!(auto_reattach(&repo, &store, &mut det).unwrap().is_empty(), "undone pair must not be re-applied");
        assert!(det.iter().any(|x| x.target == format!("commit:{c1}")));
        assert!(crate::journal::undo(&store, &moved[0].id).is_err(), "double undo is refused");
        store.delete(&NoteTarget::Commit(c1.clone())).unwrap();

        // Message-only amend keeps the patch-id → re-attached.
        store.write(&NoteTarget::Commit(git(&d, &["rev-parse", "HEAD"])), "tip note", None).unwrap();
        git(&d, &["commit", "-q", "--amend", "-m", "add b (final wording)"]);
        let tip = git(&d, &["rev-parse", "HEAD"]);
        let mut det = find(&repo, &store, &HashSet::new()).unwrap();
        let moved = auto_reattach(&repo, &store, &mut det).unwrap();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].to, format!("commit:{tip}"));
        // Undo after the user edited the re-attached note: edits are kept,
        // the original is restored on the old commit.
        store.write(&NoteTarget::Commit(tip.clone()), "tip note + my edit", None).unwrap();
        let out = crate::journal::undo(&store, &moved[0].id).unwrap();
        assert!(out.kept_edited_destination);
        assert_eq!(store.read(&NoteTarget::Commit(tip.clone())).unwrap().unwrap().content, "tip note + my edit");
        assert_eq!(store.read(&NoteTarget::Commit(moved[0].from.trim_start_matches("commit:").into())).unwrap().unwrap().content, "tip note");

        // Ambiguous: same change cherry-picked onto two branches → suggestions, no move.
        git(&d, &["switch", "-qc", "solo", "main"]);
        w("d.txt", "dup\n");
        git(&d, &["add", "."]);
        git(&d, &["commit", "-qm", "dup change"]);
        let dup = git(&d, &["rev-parse", "HEAD"]);
        store.write(&NoteTarget::Commit(dup.clone()), "dup note", None).unwrap();
        git(&d, &["switch", "-qc", "x1", "main"]);
        git(&d, &["cherry-pick", "-x", &dup]);
        // Give x2 a different base so its copy is a different commit (two
        // cherry-picks onto the same parent in the same second are identical).
        git(&d, &["switch", "-qc", "x2", "main"]);
        w("e.txt", "other\n");
        git(&d, &["add", "."]);
        git(&d, &["commit", "-qm", "unrelated"]);
        git(&d, &["cherry-pick", "-x", &dup]);
        git(&d, &["branch", "-D", "solo"]);
        let mut det = find(&repo, &store, &HashSet::new()).unwrap();
        let moved = auto_reattach(&repo, &store, &mut det).unwrap();
        assert!(moved.is_empty());
        let amb = det.iter().find(|x| x.target == format!("commit:{dup}")).unwrap();
        assert_eq!(amb.candidates.len(), 2);
    }
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
        // The amend above only changed the message of an empty commit (no patch):
        // nothing to match, stays detached.
        let mut det2 = det.clone();
        assert!(auto_reattach(&repo, &store, &mut det2).unwrap().is_empty());
        // A commit note on a reachable commit is not detached.
        let base = repo.resolve("main").unwrap();
        store.write(&NoteTarget::Commit(base), "fine", None).unwrap();
        assert_eq!(find(&repo, &store, &HashSet::new()).unwrap().len(), 2);
    }
}
