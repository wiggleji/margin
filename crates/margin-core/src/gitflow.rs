//! Git Flow, implemented natively with plain git commands — no `git-flow`
//! extension needed on the machine. Uses the same `gitflow.*` config keys as
//! git-flow (AVH), so repos initialised with `git flow init` just work and
//! vice versa.
//!
//! Finish steps are idempotent: each merge is skipped if already done and the
//! tag is skipped if it exists. After resolving a merge conflict and
//! committing, run Finish again and it continues where it stopped.

use crate::git::{RefKind, Repo};
use crate::{Error, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Feature,
    Bugfix,
    Release,
    Hotfix,
    Support,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Feature, Kind::Bugfix, Kind::Release, Kind::Hotfix, Kind::Support];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Feature => "Feature",
            Kind::Bugfix => "Bugfix",
            Kind::Release => "Release",
            Kind::Hotfix => "Hotfix",
            Kind::Support => "Support",
        }
    }
    /// Whether `finish` applies (support branches are long-lived).
    pub fn finishable(self) -> bool {
        self != Kind::Support
    }
    /// Release/hotfix names are versions and get tagged on finish.
    pub fn tags(self) -> bool {
        matches!(self, Kind::Release | Kind::Hotfix)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowConfig {
    pub main: String,
    pub develop: String,
    pub feature: String,
    pub bugfix: String,
    pub release: String,
    pub hotfix: String,
    pub support: String,
    pub versiontag: String,
}

impl FlowConfig {
    pub fn prefix(&self, k: Kind) -> &str {
        match k {
            Kind::Feature => &self.feature,
            Kind::Bugfix => &self.bugfix,
            Kind::Release => &self.release,
            Kind::Hotfix => &self.hotfix,
            Kind::Support => &self.support,
        }
    }
    pub fn branch(&self, k: Kind, name: &str) -> String {
        format!("{}{name}", self.prefix(k))
    }
}

fn get(repo: &Repo, key: &str) -> Option<String> {
    repo.run(&["config", "--get", key]).ok().map(|s| s.trim().to_string())
}

fn set(repo: &Repo, key: &str, val: &str) -> Result<()> {
    repo.run(&["config", key, val]).map(drop)
}

/// The repo's git-flow configuration, or `None` if not initialised.
/// One `git config --get-regexp` call (the palette reads this on every open).
pub fn config(repo: &Repo) -> Option<FlowConfig> {
    let out = repo.run(&["config", "--get-regexp", "^gitflow\\."]).ok()?;
    let map: std::collections::HashMap<&str, &str> =
        out.lines().filter_map(|l| l.split_once(' ').or(Some((l, "")))).collect();
    let develop = map.get("gitflow.branch.develop")?.to_string();
    let main = map.get("gitflow.branch.master").or_else(|| map.get("gitflow.branch.main"))?.to_string();
    let p = |k: &str, d: &str| map.get(format!("gitflow.prefix.{k}").as_str()).map(|v| v.to_string()).unwrap_or_else(|| d.to_string());
    Some(FlowConfig {
        main,
        develop,
        feature: p("feature", "feature/"),
        bugfix: p("bugfix", "bugfix/"),
        release: p("release", "release/"),
        hotfix: p("hotfix", "hotfix/"),
        support: p("support", "support/"),
        versiontag: p("versiontag", ""),
    })
}

fn local_branches(repo: &Repo) -> Result<Vec<String>> {
    Ok(repo.refs()?.into_iter().filter(|r| r.kind == RefKind::Local).map(|r| r.name).collect())
}

/// Initialise with default prefixes; creates `develop` from `main` if missing.
pub fn init(repo: &Repo, main: &str, develop: &str) -> Result<String> {
    let locals = local_branches(repo)?;
    if !locals.iter().any(|b| b == main) {
        bail!("production branch '{main}' does not exist");
    }
    set(repo, "gitflow.branch.master", main)?;
    set(repo, "gitflow.branch.develop", develop)?;
    for (k, d) in [("feature", "feature/"), ("bugfix", "bugfix/"), ("release", "release/"), ("hotfix", "hotfix/"), ("support", "support/"), ("versiontag", "")] {
        let key = format!("gitflow.prefix.{k}");
        if get(repo, &key).is_none() {
            set(repo, &key, d)?;
        }
    }
    let mut log = format!("git flow initialised: production '{main}', development '{develop}'");
    if !locals.iter().any(|b| b == develop) {
        repo.create_branch(develop, Some(main), false)?;
        log += &format!("; created '{develop}' from '{main}'");
    }
    Ok(log)
}

/// Existing branches of a kind, as names without the prefix.
pub fn branches(repo: &Repo, cfg: &FlowConfig, kind: Kind) -> Result<Vec<String>> {
    let p = cfg.prefix(kind);
    Ok(local_branches(repo)?.into_iter().filter_map(|b| b.strip_prefix(p).map(String::from)).collect())
}

pub fn start(repo: &Repo, cfg: &FlowConfig, kind: Kind, name: &str, base: Option<&str>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("name is required");
    }
    let branch = cfg.branch(kind, name);
    let base = base.map(String::from).unwrap_or_else(|| match kind {
        Kind::Feature | Kind::Bugfix | Kind::Release => cfg.develop.clone(),
        Kind::Hotfix | Kind::Support => cfg.main.clone(),
    });
    repo.create_branch(&branch, Some(&base), true)?;
    Ok(format!("Started {branch} from {base}"))
}

/// Push the branch and set its upstream.
pub fn publish(repo: &Repo, cfg: &FlowConfig, kind: Kind, name: &str, remote: &str) -> Result<String> {
    repo.push_branch(remote, &cfg.branch(kind, name))
}

fn is_ancestor(repo: &Repo, a: &str, b: &str) -> bool {
    repo.run(&["merge-base", "--is-ancestor", a, b]).is_ok()
}

fn merge_into(repo: &Repo, target: &str, branch: &str, log: &mut Vec<String>) -> Result<()> {
    if is_ancestor(repo, branch, target) {
        log.push(format!("{branch} already merged into {target}"));
        return Ok(());
    }
    repo.checkout(target)?;
    let msg = format!("Merge branch '{branch}' into {target}");
    repo.run(&["merge", "--no-ff", "-m", &msg, branch]).map_err(|e| {
        Error(format!(
            "merging {branch} into {target} stopped: {e}\nResolve the conflicts and commit, then run Finish again — it continues from here."
        ))
    })?;
    log.push(format!("Merged {branch} into {target}"));
    Ok(())
}

/// Finish a branch. Release/hotfix: merge into main, tag, merge back into
/// develop (or into an open release branch, for hotfixes), delete the branch.
pub fn finish(repo: &Repo, cfg: &FlowConfig, kind: Kind, name: &str, tag_message: Option<&str>) -> Result<String> {
    if !kind.finishable() {
        bail!("{} branches are not finished", kind.label().to_lowercase());
    }
    let branch = cfg.branch(kind, name);
    let locals = local_branches(repo)?;
    if !locals.contains(&branch) {
        bail!("branch '{branch}' does not exist");
    }
    let mut log = Vec::new();
    let mut targets = Vec::new();
    match kind {
        Kind::Feature | Kind::Bugfix => {
            merge_into(repo, &cfg.develop, &branch, &mut log)?;
            targets.push(cfg.develop.clone());
        }
        Kind::Release | Kind::Hotfix => {
            merge_into(repo, &cfg.main, &branch, &mut log)?;
            let tag = format!("{}{name}", cfg.versiontag);
            let exists = repo.run(&["rev-parse", "--verify", "-q", &format!("refs/tags/{tag}")]).is_ok();
            if exists {
                log.push(format!("tag {tag} already exists"));
            } else {
                let default = format!("{} {name}", kind.label());
                repo.create_annotated_tag(&tag, &cfg.main, tag_message.filter(|m| !m.trim().is_empty()).unwrap_or(&default))?;
                log.push(format!("Tagged {tag}"));
            }
            // A hotfix goes into the open release branch if there is one (git-flow behaviour).
            let back = if kind == Kind::Hotfix {
                locals.iter().find(|b| b.starts_with(&cfg.release)).cloned().unwrap_or_else(|| cfg.develop.clone())
            } else {
                cfg.develop.clone()
            };
            merge_into(repo, &back, &branch, &mut log)?;
            targets.extend([cfg.main.clone(), back]);
        }
        Kind::Support => unreachable!(),
    }
    // End on develop (as git-flow does) and delete the finished branch. `-d`
    // only checks the current branch, so verify every target ourselves and
    // then delete with -D (a hotfix may be in main + release, not develop).
    repo.checkout(&cfg.develop)?;
    if let Some(t) = targets.iter().find(|t| !is_ancestor(repo, &branch, t)) {
        bail!("{branch} is not fully merged into {t}; not deleting it");
    }
    repo.delete_branch(&branch, true)?;
    log.push(format!("Deleted {branch}"));
    Ok(log.join("; "))
}

#[cfg(test)]
mod tests {
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
    fn full_flow() {
        let d = std::env::temp_dir().join(format!("margin-flow-{}-{}", std::process::id(), crate::time::now_unix()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let w = |f: &str, c: &str| std::fs::write(d.join(f), c).unwrap();
        git(&d, &["init", "-q"]);
        git(&d, &["config", "user.name", "t"]);
        git(&d, &["config", "user.email", "t@t"]);
        w("app.txt", "v0\n");
        git(&d, &["add", "."]);
        git(&d, &["commit", "-qm", "init"]);
        let repo = Repo::open(&d).unwrap();

        assert!(config(&repo).is_none());
        init(&repo, "main", "develop").unwrap();
        let cfg = config(&repo).unwrap();
        assert_eq!((cfg.main.as_str(), cfg.develop.as_str(), cfg.feature.as_str()), ("main", "develop", "feature/"));

        // feature with a conflict against develop → finish stops, resolve, finish again
        start(&repo, &cfg, Kind::Feature, "login", None).unwrap();
        assert_eq!(git(&d, &["branch", "--show-current"]), "feature/login");
        w("app.txt", "feature\n");
        git(&d, &["commit", "-qam", "login"]);
        git(&d, &["switch", "-q", "develop"]);
        w("app.txt", "develop\n");
        git(&d, &["commit", "-qam", "dev change"]);
        assert_eq!(branches(&repo, &cfg, Kind::Feature).unwrap(), vec!["login"]);
        let err = finish(&repo, &cfg, Kind::Feature, "login", None).unwrap_err();
        assert!(err.0.contains("run Finish again"), "{err}");
        w("app.txt", "both\n");
        git(&d, &["commit", "-qam", "resolve"]);
        let log = finish(&repo, &cfg, Kind::Feature, "login", None).unwrap();
        assert!(log.contains("already merged") && log.contains("Deleted feature/login"), "{log}");
        assert!(branches(&repo, &cfg, Kind::Feature).unwrap().is_empty());

        // release → merged into main + develop, tagged
        start(&repo, &cfg, Kind::Release, "1.0", None).unwrap();
        w("version.txt", "1.0\n");
        git(&d, &["add", "."]);
        git(&d, &["commit", "-qm", "bump"]);
        finish(&repo, &cfg, Kind::Release, "1.0", Some("First release")).unwrap();
        assert_eq!(git(&d, &["tag", "-l", "-n1", "1.0"]).split_whitespace().skip(1).collect::<Vec<_>>().join(" "), "First release");
        assert_eq!(git(&d, &["show", "main:version.txt"]), "1.0");
        assert_eq!(git(&d, &["show", "develop:version.txt"]), "1.0");
        assert_eq!(git(&d, &["branch", "--show-current"]), "develop");

        // hotfix while a release is open → goes into the release branch, not develop
        start(&repo, &cfg, Kind::Release, "1.1", None).unwrap();
        start(&repo, &cfg, Kind::Hotfix, "1.0.1", None).unwrap();
        w("fix.txt", "fix\n");
        git(&d, &["add", "."]);
        git(&d, &["commit", "-qm", "fix"]);
        finish(&repo, &cfg, Kind::Hotfix, "1.0.1", None).unwrap();
        assert_eq!(git(&d, &["show", "main:fix.txt"]), "fix");
        assert_eq!(git(&d, &["show", "release/1.1:fix.txt"]), "fix");
        assert!(Command::new("git").arg("-C").arg(&d).args(["cat-file", "-e", "develop:fix.txt"]).status().map(|s| !s.success()).unwrap());
        assert_eq!(git(&d, &["tag", "-l", "1.0.1"]), "1.0.1");

        // support: starts from main, can't be finished
        start(&repo, &cfg, Kind::Support, "1.x", None).unwrap();
        assert!(finish(&repo, &cfg, Kind::Support, "1.x", None).is_err());
    }
}
