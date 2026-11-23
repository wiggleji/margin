//! Interactive rebase without an editor: the commits after a base, a plan
//! for each (pick, reword, edit, squash, fixup, drop) in the order you want,
//! then `git rebase -i` runs with that plan as its todo list. Messages you
//! write (reword, squash) are applied with `git commit --amend -F` steps, so
//! git never opens an editor. A rebase that stops (edit, conflicts) is
//! continued or aborted like any other.

use crate::git::{Repo, check_rev};
use crate::{Result, bail};
use std::path::{Path, PathBuf};

/// A commit that can be rebased.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub subject: String,
    /// The whole message.
    pub message: String,
    pub author: String,
    pub time: i64,
    /// Already on a remote branch: rewriting it needs a force push.
    pub pushed: bool,
}

/// What to do with a commit.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Pick,
    /// Keep the changes, with this message.
    Reword(String),
    /// Stop after it, to amend it or add commits.
    Edit,
    /// Fold into the commit before it (in rebase order); with the message
    /// for the combined commit.
    Squash(String),
    /// Fold into the commit before it, keeping that commit's message.
    Fixup,
    Drop,
}

/// One line of the plan, oldest first (the order the commits are replayed).
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub id: String,
    pub action: Action,
}

/// The commits `base..HEAD`, oldest first. Refuses a range with merge
/// commits (rebasing would flatten them) and a base that isn't behind HEAD.
pub fn commits(repo: &Repo, base: &str) -> Result<Vec<Item>> {
    check_rev(base)?;
    if repo.run(&["merge-base", "--is-ancestor", base, "HEAD"]).is_err() {
        bail!("{base} is not an earlier commit of the current branch");
    }
    let range = format!("{base}..HEAD");
    if !repo.run(&["rev-list", "--merges", "--max-count=1", &range])?.trim().is_empty() {
        bail!("these commits include a merge; interactive rebase would flatten it, so Margin doesn't offer it here");
    }
    let out = repo.run(&["log", "--reverse", "--format=%H%x1f%an%x1f%at%x1f%s%x1f%B%x1e", &range])?;
    // commits not on any remote branch; the rest are pushed
    let unpushed = repo.run(&["rev-list", &range, "--not", "--remotes"])?;
    let unpushed: std::collections::HashSet<&str> = unpushed.lines().collect();
    Ok(out
        .split('\x1e')
        .filter_map(|r| {
            let f: Vec<&str> = r.trim_start_matches('\n').splitn(5, '\x1f').collect();
            (f.len() == 5).then(|| Item {
                id: f[0].to_string(),
                author: f[1].to_string(),
                time: f[2].parse().unwrap_or(0),
                subject: f[3].to_string(),
                message: f[4].trim_end().to_string(),
                pushed: !unpushed.contains(f[0]),
            })
        })
        .collect())
}

/// Check a plan: squash / fixup need a commit before them to fold into.
pub fn check(steps: &[Step]) -> Result<()> {
    let mut kept = false;
    for s in steps {
        match s.action {
            Action::Squash(_) | Action::Fixup if !kept => bail!("the first commit can't be squashed or fixed up: there's nothing before it to fold into"),
            Action::Drop => {}
            _ => kept = true,
        }
    }
    if steps.iter().all(|s| s.action == Action::Drop) {
        bail!("every commit is dropped: use Reset instead");
    }
    Ok(())
}

/// Quote a path for git's shell (it runs `exec` lines and the sequence
/// editor with sh, also on Windows, where it understands `C:/…`).
fn sh_quote(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The todo list for `steps`; messages to write are saved in `dir`.
fn todo(steps: &[Step], dir: &Path) -> Result<String> {
    let mut out = String::new();
    // the message waiting for the end of a squash group
    let mut pending: Option<PathBuf> = None;
    let flush = |out: &mut String, pending: &mut Option<PathBuf>| {
        if let Some(f) = pending.take() {
            out.push_str(&format!("exec git commit --amend --allow-empty --cleanup=strip -F {}\n", sh_quote(&f)));
        }
    };
    for (i, s) in steps.iter().enumerate() {
        let msg_file = |m: &str| -> Result<PathBuf> {
            let f = dir.join(format!("msg-{i}.txt"));
            std::fs::write(&f, m)?;
            Ok(f)
        };
        match &s.action {
            Action::Squash(m) => {
                out.push_str(&format!("fixup {}\n", s.id));
                // the group's message: the last squash's (it has them all)
                pending = Some(msg_file(m)?);
                continue;
            }
            Action::Fixup => {
                out.push_str(&format!("fixup {}\n", s.id));
                continue;
            }
            _ => {}
        }
        // a new commit starts: finish the group before it
        flush(&mut out, &mut pending);
        match &s.action {
            Action::Pick => out.push_str(&format!("pick {}\n", s.id)),
            Action::Edit => out.push_str(&format!("edit {}\n", s.id)),
            Action::Drop => out.push_str(&format!("drop {}\n", s.id)),
            Action::Reword(m) => {
                out.push_str(&format!("pick {}\n", s.id));
                pending = Some(msg_file(m)?);
            }
            Action::Squash(_) | Action::Fixup => unreachable!(),
        }
    }
    flush(&mut out, &mut pending);
    Ok(out)
}

impl Repo {
    /// Rebase the commits after `base` by `steps` (oldest first). Changes not
    /// committed yet are stashed and put back (`--autostash`). Stops like
    /// `git rebase -i` does: at an `edit`, or on conflicts.
    pub fn rebase_interactive(&self, base: &str, steps: &[Step]) -> Result<String> {
        check_rev(base)?;
        check(steps)?;
        // every commit of the range, each once: nothing gets lost by accident
        let mut have: Vec<String> = commits(self, base)?.into_iter().map(|c| c.id).collect();
        let mut want: Vec<String> = steps.iter().map(|s| s.id.clone()).collect();
        have.sort();
        want.sort();
        if have != want {
            bail!("the branch changed since the plan was made: open it again");
        }
        let dir = self.common_dir.join("margin").join("rebase");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let todo_file = dir.join("todo");
        std::fs::write(&todo_file, todo(steps, &dir)?)?;
        let editor = format!("cp {}", sh_quote(&todo_file));
        self.run_env(&["rebase", "-i", "--autostash", base], &[("GIT_SEQUENCE_EDITOR", &editor)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, action: Action) -> Step {
        Step { id: id.into(), action }
    }

    #[test]
    fn todo_lines() {
        let d = std::env::temp_dir().join(format!("margin-todo-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let t = todo(&[step("a", Action::Pick), step("b", Action::Squash("ab".into())), step("c", Action::Fixup), step("d", Action::Reword("D".into())), step("e", Action::Drop), step("f", Action::Edit)], &d).unwrap();
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[..3], ["pick a", "fixup b", "fixup c"]);
        assert!(lines[3].starts_with("exec git commit --amend") && lines[3].ends_with("msg-1.txt'"), "{}", lines[3]);
        assert_eq!(lines[4], "pick d");
        assert!(lines[5].ends_with("msg-3.txt'"));
        assert_eq!(lines[6..], ["drop e", "edit f"]);
        assert_eq!(std::fs::read_to_string(d.join("msg-1.txt")).unwrap(), "ab");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn checks() {
        assert!(check(&[step("a", Action::Fixup)]).is_err());
        assert!(check(&[step("a", Action::Drop), step("b", Action::Squash("x".into()))]).is_err());
        assert!(check(&[step("a", Action::Drop)]).is_err());
        assert!(check(&[step("a", Action::Pick), step("b", Action::Fixup)]).is_ok());
        assert_eq!(sh_quote(Path::new("/tmp/it's")), r"'/tmp/it'\''s'");
    }

    #[test]
    fn rebases_a_real_branch() {
        let d = std::env::temp_dir().join(format!("margin-rebase-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let git = |args: &[&str]| {
            let o = std::process::Command::new("git").current_dir(&d).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"]).args(args).output().unwrap();
            assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "t"]);
        git(&["config", "user.email", "t@t"]);
        let commit = |name: &str, msg: &str| {
            std::fs::write(d.join(name), msg).unwrap();
            git(&["add", "."]);
            git(&["commit", "-qm", msg]);
        };
        commit("base.txt", "base");
        commit("a.txt", "add a");
        commit("b.txt", "add b");
        commit("b2.txt", "more b");
        commit("c.txt", "add c");
        commit("junk.txt", "junk");
        let repo = Repo::open(&d).unwrap();
        let base = git(&["rev-parse", "HEAD~5"]).trim().to_string();
        let items = commits(&repo, &base).unwrap();
        assert_eq!(items.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["add a", "add b", "more b", "add c", "junk"]);
        assert!(items.iter().all(|c| !c.pushed));
        let id = |s: &str| items.iter().find(|c| c.subject == s).unwrap().id.clone();
        // c first (reworded), then b with "more b" squashed in, a, junk dropped
        let plan = [
            step(&id("add c"), Action::Reword("Add c\n\nWith a body.".into())),
            step(&id("add b"), Action::Pick),
            step(&id("more b"), Action::Squash("Add b, all of it".into())),
            step(&id("add a"), Action::Pick),
            step(&id("junk"), Action::Drop),
        ];
        // a changed index is kept (autostash)
        std::fs::write(d.join("base.txt"), "local edit").unwrap();
        repo.rebase_interactive(&base, &plan).unwrap();
        let log = git(&["log", "--format=%s", &format!("{base}..HEAD")]);
        assert_eq!(log.lines().collect::<Vec<_>>(), ["add a", "Add b, all of it", "Add c"]);
        assert_eq!(git(&["log", "-1", "--skip=2", "--format=%b"]).trim(), "With a body.");
        assert!(!d.join("junk.txt").exists() && d.join("b2.txt").exists());
        assert_eq!(std::fs::read_to_string(d.join("base.txt")).unwrap(), "local edit");
        assert!(repo.pending_operation().is_none());

        // edit stops; the plan must match the branch
        let items = commits(&repo, &base).unwrap();
        assert!(repo.rebase_interactive(&base, &[step(&items[0].id, Action::Pick)]).is_err());
        let plan: Vec<Step> = items.iter().enumerate().map(|(i, c)| step(&c.id, if i == 1 { Action::Edit } else { Action::Pick })).collect();
        git(&["checkout", "-q", "--", "base.txt"]);
        repo.rebase_interactive(&base, &plan).unwrap();
        assert_eq!(repo.pending_operation(), Some("rebase"));
        repo.operation("rebase", "continue").unwrap();
        assert!(repo.pending_operation().is_none());
        assert!(commits(&repo, "HEAD~1").is_ok());
        assert!(commits(&repo, "--all").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
