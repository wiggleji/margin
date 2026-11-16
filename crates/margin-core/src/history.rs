//! Which branches the history graph shows (like Sublime Merge's branch eye).
//!
//! By default only the *major* branches are shown: the current branch and its
//! upstream, plus main / master / develop / trunk … (locally and on remotes).
//! Every other branch is hidden until you click its eye; "Show all" flips the
//! default. Choices are stored per repository in
//! `<git-common-dir>/margin/view.json`, shared by all its worktrees.
//! Hidden branches stay in the sidebar; only their commits leave the graph
//! (unless a visible branch or HEAD reaches them).

use crate::Result;
use crate::git::{Commit, RefInfo, RefKind, Repo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryFilter {
    /// Local branches shown (`true`) or hidden by hand, overriding the default.
    #[serde(default)]
    pub branches: BTreeMap<String, bool>,
    /// Remote branches (`origin/x`) shown or hidden by hand.
    #[serde(default)]
    pub remotes: BTreeMap<String, bool>,
    /// Show every branch unless hidden by hand (instead of only major ones).
    #[serde(default)]
    pub show_all: bool,
    /// Leave out commits only reachable from tags.
    #[serde(default)]
    pub hide_tags: bool,
    /// Show only this local branch (and HEAD).
    #[serde(default)]
    pub only: Option<String>,
}

/// Long-lived branch names shown by default.
pub fn is_major(name: &str) -> bool {
    matches!(name, "main" | "master" | "develop" | "development" | "dev" | "trunk" | "staging" | "production" | "prod")
}

impl HistoryFilter {
    /// Is `r` in the graph? `head_upstream`: the current branch's upstream (`origin/x`).
    pub fn shows(&self, r: &RefInfo, head_upstream: Option<&str>) -> bool {
        let remote = r.kind == RefKind::Remote;
        if let Some(o) = &self.only {
            return !remote && (r.name == *o || r.is_head);
        }
        let set = if remote { &self.remotes } else { &self.branches };
        if let Some(&v) = set.get(&r.name) {
            return v;
        }
        if self.show_all || r.is_head {
            return true;
        }
        if remote {
            let short = r.name.split_once('/').map_or(r.name.as_str(), |(_, b)| b);
            head_upstream == Some(r.name.as_str()) || is_major(short)
        } else {
            is_major(&r.name)
        }
    }

    /// Show or hide one branch by hand. Leaves "only" mode.
    pub fn set(&mut self, r: &RefInfo, visible: bool) {
        self.only = None;
        let set = if r.kind == RefKind::Remote { &mut self.remotes } else { &mut self.branches };
        set.insert(r.name.clone(), visible);
    }

    /// Back to the default: major branches only, or every branch.
    pub fn reset(show_all: bool, hide_tags: bool) -> HistoryFilter {
        HistoryFilter { show_all, hide_tags, ..Default::default() }
    }

    /// Forget branches that no longer exist. Returns whether anything changed.
    pub fn prune(&mut self, refs: &[RefInfo]) -> bool {
        let before = self.clone();
        let has = |k: RefKind, n: &str| refs.iter().any(|r| r.kind == k && r.name == n);
        self.branches.retain(|b, _| has(RefKind::Local, b));
        self.remotes.retain(|b, _| has(RefKind::Remote, b));
        if self.only.as_deref().is_some_and(|o| !has(RefKind::Local, o)) {
            self.only = None;
        }
        *self != before
    }

    /// (shown, total) branches, local and remote.
    pub fn counts(&self, refs: &[RefInfo]) -> (usize, usize) {
        let up = head_upstream(refs);
        let branches = refs.iter().filter(|r| matches!(r.kind, RefKind::Local | RefKind::Remote));
        let total = branches.clone().count();
        (branches.filter(|r| self.shows(r, up.as_deref())).count(), total)
    }

    /// Revision arguments for `git log` (HEAD is added by `log_with`).
    fn log_args(&self, refs: &[RefInfo]) -> Vec<String> {
        let up = head_upstream(refs);
        let mut a = vec![];
        for r in refs {
            if !self.shows(r, up.as_deref()) {
                continue;
            }
            match r.kind {
                RefKind::Local => a.push(format!("refs/heads/{}", r.name)),
                RefKind::Remote => a.push(format!("refs/remotes/{}", r.name)),
                _ => {}
            }
        }
        if !self.hide_tags && self.only.is_none() {
            a.push("--tags".into());
        }
        a
    }
}

/// The current branch's upstream (`origin/x`), if any.
pub fn head_upstream(refs: &[RefInfo]) -> Option<String> {
    refs.iter().find(|r| r.is_head && r.kind == RefKind::Local).and_then(|r| r.upstream.clone())
}

fn path(repo: &Repo) -> std::path::PathBuf {
    repo.common_dir.join("margin").join("view.json")
}

pub fn load(repo: &Repo) -> HistoryFilter {
    std::fs::read(path(repo)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(repo: &Repo, f: &HistoryFilter) -> Result<()> {
    let p = path(repo);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    crate::notes::atomic_write(&p, &serde_json::to_vec_pretty(f).unwrap_or_default())
}

/// `Repo::log` restricted to the branches the filter shows (`refs`: from `Repo::refs`).
pub fn log(repo: &Repo, f: &HistoryFilter, refs: &[RefInfo], limit: usize, skip: usize) -> Result<Vec<Commit>> {
    repo.log_with(&f.log_args(refs), limit, skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_major_branches_by_default() {
        let d = std::env::temp_dir().join(format!("margin-hist-{}-{}", std::process::id(), crate::time::now_unix()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let g = |args: &[&str]| {
            assert!(std::process::Command::new("git").arg("-C").arg(&d)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
                .args(args).status().unwrap().success());
        };
        g(&["init", "-q"]);
        g(&["commit", "-q", "--allow-empty", "-m", "base"]);
        g(&["checkout", "-q", "-b", "side"]);
        g(&["commit", "-q", "--allow-empty", "-m", "on side"]);
        g(&["checkout", "-q", "-b", "work", "main"]);
        g(&["commit", "-q", "--allow-empty", "-m", "on work"]);
        let repo = Repo::open(&d).unwrap();
        let refs = repo.refs().unwrap();
        let side = refs.iter().find(|r| r.name == "side").unwrap().clone();
        let subjects = |f: &HistoryFilter| log(&repo, f, &refs, 50, 0).unwrap().into_iter().map(|c| c.subject).collect::<Vec<_>>();

        // default: main (major) and work (current) shown, side hidden
        let mut f = HistoryFilter::default();
        assert!(!f.shows(&side, None));
        let s = subjects(&f);
        assert!(s.contains(&"on work".to_string()) && !s.contains(&"on side".to_string()));
        assert_eq!(f.counts(&refs), (2, 3));
        // the eye shows it
        f.set(&side, true);
        assert!(subjects(&f).contains(&"on side".to_string()));
        // show all / reset
        assert!(HistoryFilter::reset(true, false).shows(&side, None));
        // only side: side + HEAD
        f.only = Some("side".into());
        assert!(subjects(&f).contains(&"on work".to_string()));
        save(&repo, &f).unwrap();
        assert_eq!(load(&repo), f);
        // a deleted branch is forgotten
        g(&["branch", "-q", "-D", "side"]);
        assert!(f.prune(&repo.refs().unwrap()));
        assert!(f.branches.is_empty() && f.only.is_none());
    }
}
