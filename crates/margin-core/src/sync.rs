//! Notes sync through a cloud-synced folder (iCloud Drive, Dropbox, Google
//! Drive, OneDrive…) — no provider APIs, just a folder they already sync.
//!
//! Notes always live locally in `.git/margin/notes` (fast, offline, what the
//! GUI and MCP use). Sync *mirrors* that folder two-way with
//! `<sync folder>/Margin/<repo-id>/`:
//!
//! * a small `.sync-state.json` remembers each file's content hash at the
//!   last sync, so for every file we know which side changed since then;
//! * changed on one side → copied over; deleted on one side → deleted;
//! * changed on both sides → merged (the other side's text appended under a
//!   marker) and written to both — nothing is lost;
//! * iCloud placeholders (`.name.md.icloud`, not downloaded yet) are skipped,
//!   so a not-yet-downloaded note is never mistaken for a deletion.
//!
//! Any number of repositories can sync.

use crate::config::Settings;
use crate::git::Repo;
use crate::{Result, bail};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const STATE: &str = ".sync-state.json";

/// Stable id for "the same repository on another computer": derived only
/// from the remote (ssh/https forms agree), never from the local folder name,
/// which differs between clones. Without a remote: the root commit.
pub fn repo_id(repo: &Repo) -> String {
    let remote = repo.remote_url("origin").or_else(|| repo.remotes().first().and_then(|r| repo.remote_url(r)));
    let (name, key) = match remote {
        Some(u) => match crate::github::parse_remote(&u) {
            Some(r) => (r.name.clone(), format!("{}/{}/{}", r.host, r.owner.to_lowercase(), r.name.to_lowercase())),
            None => {
                let k = u.trim().trim_end_matches('/').trim_end_matches(".git").to_lowercase();
                (k.rsplit(['/', ':']).next().unwrap_or("repo").to_string(), k)
            }
        },
        None => (
            "repo".to_string(),
            repo.run(&["rev-list", "--max-parents=0", "HEAD"])
                .ok()
                .and_then(|o| o.lines().last().map(String::from))
                .unwrap_or_else(|| repo.workdir.display().to_string()),
        ),
    };
    format!("{}-{:08x}", slug(&name), fnv(key.as_bytes()) as u32)
}

pub fn remote_dir(sync_folder: &Path, id: &str) -> PathBuf {
    sync_folder.join("Margin").join(id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoSync {
    /// Not enabled for this repo.
    Off,
    /// Enabled, but no sync folder chosen (or it's missing/unmounted).
    NoFolder,
    Active { remote: PathBuf },
}

/// What sync should do for `id` given the settings.
pub fn status_for(id: &str, settings: &Settings) -> RepoSync {
    if !settings.synced_repos.iter().any(|r| r == id) {
        return RepoSync::Off;
    }
    let Some(folder) = settings.sync_folder.as_ref().map(PathBuf::from).filter(|f| f.is_dir()) else {
        return RepoSync::NoFolder;
    };
    RepoSync::Active { remote: remote_dir(&folder, id) }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub pulled: usize,
    pub pushed: usize,
    pub deleted_local: usize,
    pub deleted_remote: usize,
    pub merged: usize,
}

impl Report {
    pub fn local_changed(&self) -> bool {
        self.pulled + self.deleted_local + self.merged > 0
    }
    pub fn is_empty(&self) -> bool {
        *self == Report::default()
    }
}

/// Files that sync: notes and task links. Not the per-computer undo journal.
fn synced_files(dir: &Path) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut files = BTreeSet::new();
    let mut placeholders = BTreeSet::new();
    let mut visit = |sub: &str| {
        let Ok(rd) = fs::read_dir(dir.join(sub)) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = if sub.is_empty() { name.clone() } else { format!("{sub}/{name}") };
            // iCloud: ".foo.md.icloud" = foo.md exists remotely but isn't downloaded
            if let Some(real) = name.strip_prefix('.').and_then(|n| n.strip_suffix(".icloud")) {
                placeholders.insert(if sub.is_empty() { real.to_string() } else { format!("{sub}/{real}") });
                continue;
            }
            let keep = if sub.is_empty() { name == "repo.md" || name == "links.json" } else { name.ends_with(".md") };
            if keep && e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                files.insert(rel);
            }
        }
    };
    for sub in ["", "branch", "commit", "task"] {
        visit(sub);
    }
    (files, placeholders)
}

fn hash_file(p: &Path) -> Option<String> {
    fs::read(p).ok().map(|b| format!("{:016x}", fnv(&b)))
}

fn write_atomic(p: &Path, data: &[u8]) -> Result<()> {
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    crate::notes::atomic_write(p, data)
}

/// What the last sync agreed on, tied to the remote folder it was made with.
#[derive(Debug, Default, Serialize, serde::Deserialize)]
struct SyncState {
    #[serde(default)]
    remote: String,
    /// rel path → content hash at the last sync
    #[serde(default)]
    files: BTreeMap<String, String>,
}

enum Op {
    Same(Option<String>),
    Pull(String),
    Push(String),
    DelLocal,
    DelRemote,
    Merge,
}

/// Above this many deletions in one pass (and more than half the notes on
/// that side), stop instead: almost certainly a wiped/replaced folder.
const MAX_SILENT_DELETES: usize = 3;

/// One two-way mirror pass between the local notes dir and the remote dir.
///
/// Safety: the sync record is tied to the remote folder (a different or
/// empty/wiped folder means "first sync": union, never delete); deletions are
/// moved to `.trash/<time>/`; a pass that would delete most notes on a side
/// is refused without changing anything.
pub fn mirror(local: &Path, remote: &Path) -> Result<Report> {
    if !remote.parent().is_some_and(|p| p.exists() || fs::create_dir_all(p).is_ok()) {
        bail!("sync folder is not available");
    }
    fs::create_dir_all(remote)?;
    let state_path = local.join(STATE);
    let remote_key = remote.display().to_string();
    let mut st: SyncState = fs::read_to_string(&state_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let (lf, _) = synced_files(local);
    let (rf, placeholders) = synced_files(remote);
    // First sync with this folder, or one side was wiped/replaced: no shared
    // history to compare against → merge everything, delete nothing.
    if st.remote != remote_key || (rf.is_empty() && placeholders.is_empty()) || lf.is_empty() {
        st = SyncState { remote: remote_key.clone(), files: BTreeMap::new() };
    }

    // ---- plan
    let all: BTreeSet<String> = lf.iter().chain(rf.iter()).chain(st.files.keys()).cloned().collect();
    let mut plan: Vec<(String, Op)> = Vec::new();
    for rel in all {
        if placeholders.contains(&rel) {
            continue; // not downloaded yet: decide next time
        }
        let lv = hash_file(&local.join(&rel));
        let rv = hash_file(&remote.join(&rel));
        let b = st.files.get(&rel).cloned();
        let op = if lv == rv {
            Op::Same(lv)
        } else if lv == b {
            match rv {
                Some(v) => Op::Pull(v),
                None => Op::DelLocal,
            }
        } else if rv == b {
            match lv {
                Some(v) => Op::Push(v),
                None => Op::DelRemote,
            }
        } else {
            Op::Merge
        };
        plan.push((rel, op));
    }
    let dl = plan.iter().filter(|(_, o)| matches!(o, Op::DelLocal)).count();
    let dr = plan.iter().filter(|(_, o)| matches!(o, Op::DelRemote)).count();
    if dl > MAX_SILENT_DELETES && dl * 2 > lf.len() {
        bail!("sync stopped: it would delete {dl} of {} notes on this computer; nothing was changed (check the sync folder)", lf.len());
    }
    if dr > MAX_SILENT_DELETES && dr * 2 > rf.len() {
        bail!("sync stopped: it would delete {dr} of {} notes in the sync folder; nothing was changed", rf.len());
    }

    // ---- apply
    let stamp = crate::format_utc(crate::time::now_unix()).replace([' ', ':'], "-");
    let trash = |root: &Path, rel: &str| -> Result<()> {
        let to = root.join(".trash").join(&stamp).join(rel);
        if let Some(d) = to.parent() {
            fs::create_dir_all(d)?;
        }
        fs::rename(root.join(rel), to)?;
        Ok(())
    };
    let mut rep = Report::default();
    for (rel, op) in plan {
        let (lp, rp) = (local.join(&rel), remote.join(&rel));
        match op {
            Op::Same(v) => {
                match v {
                    Some(v) => st.files.insert(rel, v),
                    None => st.files.remove(&rel),
                };
            }
            Op::Pull(v) => {
                write_atomic(&lp, &fs::read(&rp)?)?;
                st.files.insert(rel, v);
                rep.pulled += 1;
            }
            Op::Push(v) => {
                write_atomic(&rp, &fs::read(&lp)?)?;
                st.files.insert(rel, v);
                rep.pushed += 1;
            }
            Op::DelLocal => {
                trash(local, &rel)?;
                st.files.remove(&rel);
                rep.deleted_local += 1;
            }
            Op::DelRemote => {
                trash(remote, &rel)?;
                st.files.remove(&rel);
                rep.deleted_remote += 1;
            }
            Op::Merge => {
                // Both changed (or one deleted while the other edited): keep everything.
                let l = fs::read_to_string(&lp).unwrap_or_default();
                let r = fs::read_to_string(&rp).unwrap_or_default();
                let merged = if rel.ends_with(".json") {
                    r // links.json: the remote copy wins; it is small and rebuilt on edit
                } else if l.trim().is_empty() {
                    r
                } else if r.trim().is_empty() {
                    l
                } else {
                    format!(
                        "{}\n\n---\n\n_Conflicting edit synced from another computer ({} UTC):_\n\n{}\n",
                        l.trim_end(),
                        crate::format_utc(crate::time::now_unix()),
                        r.trim()
                    )
                };
                write_atomic(&lp, merged.as_bytes())?;
                write_atomic(&rp, merged.as_bytes())?;
                st.files.insert(rel, format!("{:016x}", fnv(merged.as_bytes())));
                rep.merged += 1;
            }
        }
    }
    let json = serde_json::to_string(&st)?;
    if fs::read_to_string(&state_path).ok().as_deref() != Some(json.as_str()) {
        write_atomic(&state_path, json.as_bytes())?;
    }
    Ok(rep)
}

/// Run one sync pass for a repo if its sync is active (settings).
/// `None` when sync is off or unavailable for it.
pub fn sync_repo_now(repo: &Repo, local_notes: &Path) -> Option<Result<Report>> {
    let settings = crate::config::load_settings();
    match status_for(&repo_id(repo), &settings) {
        RepoSync::Active { remote } => Some(mirror(local_notes, &remote)),
        _ => None,
    }
}

/// Cheap change detector for a notes dir (names + sizes + mtimes).
pub fn dir_stamp(dir: &Path) -> u64 {
    let mut parts = Vec::new();
    for sub in ["", "branch", "commit", "task"] {
        if let Ok(rd) = fs::read_dir(dir.join(sub)) {
            for e in rd.flatten() {
                if let Ok(m) = e.metadata() {
                    parts.push(format!("{sub}/{}:{}:{:?}", e.file_name().to_string_lossy(), m.len(), m.modified().ok()));
                }
            }
        }
    }
    parts.sort();
    fnv(parts.join("\n").as_bytes())
}

/// Cloud folders found on this computer: (label, path to use as sync folder).
pub fn detect_providers() -> Vec<(String, PathBuf)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return vec![] };
    let mut out = Vec::new();
    let mut add = |label: &str, p: PathBuf| {
        if p.is_dir() && !out.iter().any(|(_, q): &(String, PathBuf)| *q == p) {
            out.push((label.to_string(), p));
        }
    };
    add("iCloud Drive", home.join("Library/Mobile Documents/com~apple~CloudDocs"));
    add("Dropbox", home.join("Library/CloudStorage/Dropbox"));
    add("Dropbox", home.join("Dropbox"));
    if let Ok(rd) = fs::read_dir(home.join("Library/CloudStorage")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("GoogleDrive-") {
                add("Google Drive", e.path().join("My Drive"));
            } else if n.starts_with("OneDrive") {
                add("OneDrive", e.path());
            }
        }
    }
    add("Google Drive", home.join("Google Drive"));
    out
}

fn slug(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect()
}

fn fnv(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("margin-sync-{tag}-{}-{}", std::process::id(), crate::time::now_unix()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }
    fn put(d: &Path, rel: &str, s: &str) {
        fs::create_dir_all(d.join(rel).parent().unwrap()).unwrap();
        fs::write(d.join(rel), s).unwrap();
    }
    fn get(d: &Path, rel: &str) -> Option<String> {
        fs::read_to_string(d.join(rel)).ok()
    }

    #[test]
    fn two_computers_one_cloud_folder() {
        let root = tmp("two");
        let (mac1, mac2, cloud) = (root.join("mac1"), root.join("mac2"), root.join("cloud/Margin/app-1234"));
        fs::create_dir_all(&mac1).unwrap();
        fs::create_dir_all(&mac2).unwrap();

        // mac1 writes notes, syncs up
        put(&mac1, "branch/feature%2Fx.md", "# plan");
        put(&mac1, "repo.md", "repo notes");
        put(&mac1, "links.json", "{}");
        put(&mac1, "reattach.json", "{\"local\":true}");
        let r = mirror(&mac1, &cloud).unwrap();
        assert_eq!(r.pushed, 3);
        assert!(get(&cloud, "reattach.json").is_none(), "undo journal stays per-computer");

        // mac2 pulls them
        let r = mirror(&mac2, &cloud).unwrap();
        assert_eq!(r.pulled, 3);
        assert_eq!(get(&mac2, "branch/feature%2Fx.md").as_deref(), Some("# plan"));

        // edit on mac2 → mac1 gets it; delete on mac1 → gone on mac2
        put(&mac2, "branch/feature%2Fx.md", "# plan v2");
        mirror(&mac2, &cloud).unwrap();
        fs::remove_file(mac1.join("repo.md")).unwrap();
        let r = mirror(&mac1, &cloud).unwrap();
        assert_eq!((r.pulled, r.deleted_remote), (1, 1));
        assert_eq!(get(&mac1, "branch/feature%2Fx.md").as_deref(), Some("# plan v2"));
        mirror(&mac2, &cloud).unwrap();
        assert!(get(&mac2, "repo.md").is_none());

        // nothing changed → no-op
        assert!(mirror(&mac1, &cloud).unwrap().is_empty());

        // both edit the same note → merged on both sides, nothing lost
        put(&mac1, "branch/feature%2Fx.md", "# plan v3 (mac1)");
        mirror(&mac1, &cloud).unwrap();
        put(&mac2, "branch/feature%2Fx.md", "# plan v3 (mac2)");
        let r = mirror(&mac2, &cloud).unwrap();
        assert_eq!(r.merged, 1);
        let m = get(&mac2, "branch/feature%2Fx.md").unwrap();
        assert!(m.contains("(mac2)") && m.contains("(mac1)") && m.contains("Conflicting edit"), "{m}");
        mirror(&mac1, &cloud).unwrap();
        assert_eq!(get(&mac1, "branch/feature%2Fx.md"), Some(m));
    }

    #[test]
    fn switching_or_wiping_the_sync_folder_never_deletes_notes() {
        let root = tmp("safety");
        let local = root.join("local");
        fs::create_dir_all(&local).unwrap();
        for i in 0..6 {
            put(&local, &format!("branch/b{i}.md"), &format!("note {i}"));
        }
        let old = root.join("cloud/Margin/old-id");
        mirror(&local, &old).unwrap();

        // 1) the sync folder changes (new id / new provider): first sync → push all
        let new = root.join("cloud2/Margin/new-id");
        let r = mirror(&local, &new).unwrap();
        assert_eq!((r.pushed, r.deleted_local), (6, 0), "{r:?}");

        // 2) the cloud folder gets wiped: push everything back, delete nothing
        fs::remove_dir_all(&new).unwrap();
        let r = mirror(&local, &new).unwrap();
        assert_eq!((r.pushed, r.deleted_local), (6, 0));
        assert_eq!(synced_files(&local).0.len(), 6);

        // 3) most notes vanish remotely while a few stay: refused, nothing changed
        for i in 0..5 {
            fs::remove_file(new.join(format!("branch/b{i}.md"))).unwrap();
        }
        let err = mirror(&local, &new).unwrap_err();
        assert!(err.0.contains("sync stopped"), "{err}");
        assert_eq!(synced_files(&local).0.len(), 6);

        // 4) a normal single deletion still syncs — and lands in .trash
        for i in 0..5 {
            put(&new, &format!("branch/b{i}.md"), &format!("note {i}"));
        }
        mirror(&local, &new).unwrap();
        fs::remove_file(new.join("branch/b0.md")).unwrap();
        let r = mirror(&local, &new).unwrap();
        assert_eq!(r.deleted_local, 1);
        assert!(!local.join("branch/b0.md").exists());
        let trashed: Vec<_> = walk(&local.join(".trash"));
        assert!(trashed.iter().any(|p| p.ends_with("branch/b0.md")), "{trashed:?}");

        // 5) local notes dir wiped (e.g. re-clone): pull everything, delete nothing remotely
        fs::remove_dir_all(&local).unwrap();
        fs::create_dir_all(&local).unwrap();
        let r = mirror(&local, &new).unwrap();
        assert_eq!((r.pulled, r.deleted_remote), (5, 0));
    }

    fn walk(d: &Path) -> Vec<String> {
        let mut v = vec![];
        if let Ok(rd) = fs::read_dir(d) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    v.extend(walk(&e.path()));
                } else {
                    v.push(e.path().display().to_string());
                }
            }
        }
        v
    }

    #[test]
    fn icloud_placeholder_is_not_a_deletion() {
        let root = tmp("icloud");
        let (local, cloud) = (root.join("l"), root.join("c/Margin/x"));
        fs::create_dir_all(&local).unwrap();
        put(&local, "task/PROJ-1.md", "task note");
        mirror(&local, &cloud).unwrap();
        // iCloud evicts the file and leaves a placeholder
        fs::remove_file(cloud.join("task/PROJ-1.md")).unwrap();
        put(&cloud, "task/.PROJ-1.md.icloud", "");
        let r = mirror(&local, &cloud).unwrap();
        assert!(r.is_empty(), "{r:?}");
        assert_eq!(get(&local, "task/PROJ-1.md").as_deref(), Some("task note"));
    }

    #[test]
    fn same_repo_same_id_regardless_of_folder_or_url_form() {
        let root = tmp("id");
        let mk = |dir: &str, url: &str| {
            let d = root.join(dir);
            fs::create_dir_all(&d).unwrap();
            let g = |a: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&d).args(a).status().unwrap().success());
            g(&["init", "-q"]);
            g(&["remote", "add", "origin", url]);
            Repo::open(&d).unwrap()
        };
        let a = mk("work-checkout", "git@github.com:Acme/App.git");
        let b = mk("app", "https://github.com/acme/app");
        assert_eq!(repo_id(&a), repo_id(&b));
        assert!(repo_id(&a).starts_with("app-"));
        let c = mk("app2", "https://github.com/acme/other");
        assert_ne!(repo_id(&a), repo_id(&c));
    }

    #[test]
    fn status() {
        let root = tmp("status");
        let mut s = Settings::default();
        assert_eq!(status_for("a", &s), RepoSync::Off);
        s.synced_repos = vec!["a".into(), "b".into()];
        assert_eq!(status_for("a", &s), RepoSync::NoFolder);
        s.sync_folder = Some(root.display().to_string());
        assert_eq!(status_for("a", &s), RepoSync::Active { remote: root.join("Margin/a") });
        // any number of repositories sync
        assert_eq!(status_for("b", &s), RepoSync::Active { remote: root.join("Margin/b") });
    }
}
