//! Which branches the history graph shows (like Sublime Merge's "hide branch").
//! Stored per repository in `<git-common-dir>/margin/view.json`, so all
//! worktrees of a repository share it. Hidden branches stay in the sidebar;
//! only their commits leave the graph (unless another visible ref or HEAD
//! reaches them).

use crate::Result;
use crate::git::{Commit, Repo};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryFilter {
    /// Hidden local branches (`feature/x`).
    #[serde(default)]
    pub hidden_branches: Vec<String>,
    /// Hidden remote branches (`origin/feature/x`).
    #[serde(default)]
    pub hidden_remotes: Vec<String>,
    /// Hide every remote branch.
    #[serde(default)]
    pub hide_all_remotes: bool,
    /// Hide tags (commits only reachable from a tag).
    #[serde(default)]
    pub hide_tags: bool,
    /// Show only this local branch (and HEAD).
    #[serde(default)]
    pub only: Option<String>,
}

impl HistoryFilter {
    pub fn is_empty(&self) -> bool {
        *self == HistoryFilter::default()
    }

    /// Is this branch's history hidden? `remote`: `origin/x` style name.
    pub fn hides(&self, name: &str, remote: bool) -> bool {
        if let Some(o) = &self.only {
            return remote || name != o;
        }
        if remote {
            self.hide_all_remotes || self.hidden_remotes.iter().any(|h| h == name)
        } else {
            self.hidden_branches.iter().any(|h| h == name)
        }
    }

    /// Toggle one branch. Leaving "only" mode keeps the rest visible.
    pub fn toggle(&mut self, name: &str, remote: bool) {
        self.only = None;
        let list = if remote { &mut self.hidden_remotes } else { &mut self.hidden_branches };
        if let Some(i) = list.iter().position(|h| h == name) {
            list.remove(i);
        } else {
            list.push(name.to_string());
            list.sort();
        }
    }

    /// Names that no longer exist are dropped (e.g. after a branch is deleted).
    pub fn prune(&mut self, locals: &[&str], remotes: &[&str]) -> bool {
        let before = self.clone();
        self.hidden_branches.retain(|b| locals.contains(&b.as_str()));
        self.hidden_remotes.retain(|b| remotes.contains(&b.as_str()));
        if self.only.as_deref().is_some_and(|o| !locals.contains(&o)) {
            self.only = None;
        }
        *self != before
    }

    /// Revision arguments for `git log` (HEAD is always included).
    fn log_args(&self) -> Vec<String> {
        let mut a = vec![];
        if let Some(o) = &self.only {
            a.push(format!("refs/heads/{o}"));
            return a;
        }
        // `--exclude` applies to the next --branches / --remotes only.
        a.extend(self.hidden_branches.iter().map(|b| format!("--exclude={b}")));
        a.push("--branches".into());
        if !self.hide_all_remotes {
            a.extend(self.hidden_remotes.iter().map(|b| format!("--exclude={b}")));
            a.push("--remotes".into());
        }
        if !self.hide_tags {
            a.push("--tags".into());
        }
        a
    }
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

/// `Repo::log` restricted by the filter.
pub fn log(repo: &Repo, f: &HistoryFilter, limit: usize, skip: usize) -> Result<Vec<Commit>> {
    repo.log_with(&f.log_args(), limit, skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_branches_leave_the_graph() {
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
        g(&["checkout", "-q", "main"]);
        g(&["commit", "-q", "--allow-empty", "-m", "on main"]);
        let repo = Repo::open(&d).unwrap();
        let subjects = |f: &HistoryFilter| log(&repo, f, 50, 0).unwrap().into_iter().map(|c| c.subject).collect::<Vec<_>>();

        let mut f = HistoryFilter::default();
        assert!(subjects(&f).contains(&"on side".to_string()));
        f.toggle("side", false);
        assert!(f.hides("side", false));
        assert!(!subjects(&f).contains(&"on side".to_string()));
        assert!(subjects(&f).contains(&"on main".to_string()));
        // "only side": side's commits plus HEAD (main)
        f.only = Some("side".into());
        let s = subjects(&f);
        assert!(s.contains(&"on side".to_string()) && s.contains(&"on main".to_string()));
        // saved per repository
        save(&repo, &f).unwrap();
        assert_eq!(load(&repo), f);
        // deleted branches are pruned
        assert!(f.prune(&["main"], &[]));
        assert!(f.is_empty());
    }
}
