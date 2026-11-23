//! Every command shown in the palette, built from the current repo state.

use crate::app::{Action, MarginApp, RepoView, Sel};
use crate::notes_panel::NoteTab;
use crate::palette::{Command, Group, Item, Run, Step};
use margin_core::git::{RefKind, Repo, ResetMode};
use margin_core::gitflow::{self, Kind};
use std::path::PathBuf;
use std::sync::Arc;

const CMD: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

fn c(group: Group, title: &str, hint: &str, keywords: &str, steps: Vec<Step>, run: Run) -> Command {
    Command { group, title: title.into(), hint: hint.into(), keywords: keywords.into(), steps, run }
}

fn pick(prompt: &str, items: Vec<Item>) -> Step {
    Step::Pick { prompt: prompt.into(), items }
}
fn input(prompt: &str, initial: &str) -> Step {
    Step::Input { prompt: prompt.into(), initial: initial.into() }
}
fn confirm(msg: impl Into<String>) -> Step {
    Step::Confirm(msg.into())
}

/// Run a git operation in the background (refreshes afterwards).
fn git<F>(label: &str, f: F) -> Run
where
    F: Fn(&Repo, &[String]) -> margin_core::Result<String> + Send + Sync + 'static,
{
    let label = label.to_string();
    let f = Arc::new(f);
    Box::new(move |vals: &[String]| {
        let vals = vals.to_vec();
        let f = f.clone();
        vec![Action::RunGit { label: label.clone(), f: Arc::new(move |r: &Repo| f(r, &vals)) }]
    })
}

fn act(f: impl Fn(&[String]) -> Vec<Action> + 'static) -> Run {
    Box::new(f)
}

fn ok<T>(r: margin_core::Result<T>) -> margin_core::Result<String> {
    r.map(|_| String::new())
}

pub fn build(app: &MarginApp) -> Vec<Command> {
    let mut v = Vec::new();
    repository(app, &mut v);
    let Some(rv) = &app.rv else { return v };
    let Some(snap) = &rv.snap else { return v };

    let locals: Vec<&margin_core::git::RefInfo> = snap.refs.iter().filter(|r| r.kind == RefKind::Local).collect();
    let current = snap.status.branch.clone();
    let local_items = |exclude_head: bool| -> Vec<Item> {
        locals
            .iter()
            .filter(|r| !(exclude_head && r.is_head))
            .map(|r| Item::new(r.name.clone(), if r.is_head { "current".to_string() } else { r.track.clone() }, r.name.clone()))
            .collect()
    };
    let remote_branch_items: Vec<Item> =
        snap.refs.iter().filter(|r| r.kind == RefKind::Remote).map(|r| Item::simple(r.name.clone())).collect();
    let all_branch_items: Vec<Item> = local_items(false).into_iter().chain(remote_branch_items.clone()).collect();
    let tag_items: Vec<Item> = snap.refs.iter().filter(|r| r.kind == RefKind::Tag).map(|r| Item::simple(r.name.clone())).collect();
    let remotes = rv.repo.remotes();
    let remote_items: Vec<Item> =
        remotes.iter().map(|r| Item::new(r.clone(), rv.repo.remote_url(r).unwrap_or_default(), r.clone())).collect();
    let default_remote = if remotes.iter().any(|r| r == "origin") { "origin".to_string() } else { remotes.first().cloned().unwrap_or_default() };
    let stash_items: Vec<Item> = snap.stashes.iter().map(|s| Item::new(s.message.clone(), s.name.clone(), s.name.clone())).collect();

    operation(rv, &mut v);
    remote(&remote_items, &remote_branch_items, &mut v);
    commit(rv, &mut v);
    branch(rv, current.as_deref(), &local_items, &all_branch_items, &remote_branch_items, &remote_items, &mut v);
    tag(&tag_items, &remote_items, &default_remote, &mut v);
    stash(&stash_items, &mut v);
    worktree(rv, &locals, &mut v);
    git_flow(rv, &default_remote, &mut v);
    if rv.repo.workdir.join(".gitmodules").exists() {
        v.push(c(Group::Submodule, "Update submodules", "init + recursive", "submodule update init", vec![], git("Submodule update", |r, _| r.submodule_update())));
        v.push(c(Group::Submodule, "Sync submodule URLs", "", "submodule sync", vec![], git("Submodule sync", |r, _| r.submodule_sync())));
    }
    github(rv, current.as_deref(), &mut v);
    notes(rv, current.as_deref(), &mut v);
    history_filter(rv, current.as_deref(), &local_items(false), &mut v);
    navigate(&all_branch_items, &tag_items, &mut v);
    v
}

fn repository(app: &MarginApp, v: &mut Vec<Command>) {
    let name_of = |p: &str| PathBuf::from(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Change directory: open tabs first, then recent. (Worktrees have their
    // own commands and sidebar section.)
    let tabs = app.tabs();
    let mut items: Vec<Item> = tabs
        .iter()
        .map(|(p, n, active)| Item::new(n.clone(), if *active { "current tab".to_string() } else { format!("open tab · {}", p.display()) }, p.display().to_string()))
        .collect();
    for p in app.recent_closed(&tabs) {
        items.push(Item::new(name_of(&p), format!("recent · {}", crate::app::tilde(std::path::Path::new(&p))), p.clone()));
    }
    let (key, back_key) = if cfg!(target_os = "macos") { ("⌃Tab", "⌃⇧Tab") } else { ("Ctrl+Tab", "Ctrl+Shift+Tab") };
    v.push(c(Group::Repository, "Change directory…", "", "switch tab repository project open recent", vec![pick("Directory", items)], act(|x| vec![Action::OpenRepo(PathBuf::from(&x[0]))])));
    if tabs.len() > 1 {
        v.push(c(Group::Repository, "Next tab", key, "directory switch", vec![], act(|_| vec![Action::NextTab])));
        v.push(c(Group::Repository, "Previous tab", back_key, "directory switch", vec![], act(|_| vec![Action::PrevTab])));
    }
    let key = if cfg!(target_os = "macos") { "⌘⇧↩" } else { "Ctrl+Shift+Enter" };
    let t = if app.focus_diff { "Show the branches and history again" } else { "Focus on the diff" };
    v.push(c(Group::Navigate, t, key, "full width review zen hide sidebar history layout", vec![], act(|_| vec![Action::ToggleFocusDiff])));
    // View: each panel and sidebar section, as a toggle
    for p in &crate::view::PANES {
        let on = crate::view::shown(&app.state, p.id);
        let title = format!("{} {}", if on { "Hide" } else { "Show" }, p.label.to_lowercase());
        let id = p.id.to_string();
        v.push(c(Group::Navigate, &title, &crate::view::shortcut(p.digit), "view toggle panel layout show hide", vec![], act(move |_| vec![Action::ToggleView(id.clone())])));
    }
    for (sec, label) in crate::view::SECTIONS {
        let id = crate::view::section_id(sec);
        let on = crate::view::shown(&app.state, &id);
        let title = format!("{} {} in the sidebar", if on { "Hide" } else { "Show" }, label.to_lowercase());
        v.push(c(Group::Navigate, &title, "sidebar section", "view toggle sidebar section show hide", vec![], act(move |_| vec![Action::ToggleView(id.clone())])));
    }
    let dock = if app.state.notes_under_history { "Move notes under the commit details" } else { "Move notes under the branches and history" };
    v.push(c(Group::Notes, dock, "", "panel position dock layout left right", vec![], act(|_| vec![Action::ToggleNotesDock])));
    v.push(c(Group::Notes, "Notes sync settings…", "iCloud · Dropbox · Drive", "cloud sync icloud dropbox google drive", vec![], act(|_| vec![Action::OpenSettings])));
    if app.rv.as_ref().is_some_and(|rv| matches!(rv.sync_state, margin_core::sync::RepoSync::Active { .. })) {
        v.push(c(Group::Notes, "Sync notes now", "", "cloud sync", vec![], act(|_| vec![Action::SyncNow])));
    }
    if cfg!(target_os = "macos") {
        v.push(c(Group::Repository, "Open folder…", &format!("{CMD}O"), "repository directory new tab browse", vec![], act(|_| vec![Action::OpenFolderDialog])));
    }
    v.push(c(Group::Repository, "Open repository by path…", "new tab", "path folder directory", vec![input("Path to a git repository", "")], act(|x| vec![Action::OpenRepo(PathBuf::from(&x[0]))])));
    v.push(c(Group::Repository, "Check for updates…", &format!("Margin {}", env!("CARGO_PKG_VERSION")), "update upgrade version new release", vec![], act(|_| vec![Action::CheckUpdate { manual: true }])));
    v.push(c(Group::Repository, "Install 'margin' command…", "margin . in a terminal", "cli shell terminal command line path open", vec![], act(|_| vec![Action::InstallCli])));
    let Some(rv) = &app.rv else { return };
    let path = rv.repo.workdir.clone();
    v.push(c(Group::Repository, "Settings…", "", "preferences options auto fetch", vec![], act(|_| vec![Action::OpenSettings])));
    v.push(c(Group::Repository, "Repository settings…", "this repository", "tasks github issues jira ticket notes sync branches history per repo", vec![], act(|_| vec![Action::OpenRepoSettings])));
    v.push(c(Group::Remote, "Toggle auto-fetch", "", "background fetch interval", vec![], act(|_| vec![Action::ToggleAutoFetch])));
    v.push(c(Group::Repository, "Refresh", &format!("{CMD}R"), "reload status", vec![], act(|_| vec![Action::Refresh])));
    let p = path.clone();
    v.push(c(Group::Repository, "Reveal in Finder", "", "open folder explorer", vec![], act(move |_| vec![Action::OpenPath { path: p.clone(), terminal: false }])));
    let p = path.clone();
    v.push(c(Group::Repository, "Open in Terminal", "", "shell console", vec![], act(move |_| vec![Action::OpenPath { path: p.clone(), terminal: true }])));
    let p = path.display().to_string();
    v.push(c(Group::Repository, "Copy repository path", "", "clipboard", vec![], act(move |_| vec![Action::Copy(p.clone())])));
    v.push(c(Group::Repository, "Close tab", "", "close repository directory", vec![], act(|_| vec![Action::CloseRepo])));
}

fn operation(rv: &RepoView, v: &mut Vec<Command>) {
    let conflicts = rv.snap.as_ref().map_or(0, |s| s.status.files.iter().filter(|f| f.conflicted).count());
    if conflicts > 0 {
        let hint = if conflicts == 1 { "1 file".to_string() } else { format!("{conflicts} files") };
        v.push(c(Group::Operation, "Resolve conflicts…", &hint, "conflict merge resolve editor ours theirs", vec![], act(|_| vec![Action::OpenConflicts(None)])));
    }
    let Some(op) = rv.snap.as_ref().and_then(|s| s.pending_op) else {
        v.push(c(Group::Branch, "Interactive rebase…", "since the default branch", "rebase squash fixup reword reorder drop edit commits", vec![], act(|_| vec![Action::OpenRebase(None)])));
        return;
    };
    let verbs: &[&str] = if op == "merge" { &["continue", "abort"] } else { &["continue", "skip", "abort"] };
    for &verb in verbs {
        let title = format!("{} {op}", capitalize(verb));
        let steps = if verb == "abort" { vec![confirm(format!("Abort the {op} and return to the state before it started?"))] } else { vec![] };
        let label = title.clone();
        v.push(c(Group::Operation, &title, "", "conflict resolve", steps, git(&label, move |r, _| r.operation(op, verb))));
    }
}

fn remote(remote_items: &[Item], remote_branches: &[Item], v: &mut Vec<Command>) {
    v.push(c(Group::Remote, "Fetch", "all remotes, prune", "update download", vec![], act(|_| vec![Action::Fetch])));
    v.push(c(Group::Remote, "Fetch remote…", "", "download", vec![pick("Remote", remote_items.to_vec())], git("Fetch", |r, x| r.fetch_remote(&x[0]))));
    v.push(c(Group::Remote, "Pull", "", "update merge", vec![], git("Pull", |r, _| r.pull())));
    v.push(c(Group::Remote, "Pull with rebase", "", "update", vec![], git("Pull --rebase", |r, _| r.pull_rebase())));
    v.push(c(Group::Remote, "Push", "", "upload publish", vec![], act(|_| vec![Action::Push { force: false }])));
    v.push(c(
        Group::Remote,
        "Force push (with lease)…",
        "",
        "overwrite upload",
        vec![confirm("Force-push the current branch? Remote commits not in your branch are discarded (refused if the remote moved since your last fetch).")],
        act(|_| vec![Action::Push { force: true }]),
    ));
    v.push(c(Group::Remote, "Add remote…", "", "new origin upstream", vec![input("Remote name", "upstream"), input("URL", "")], git("Add remote", |r, x| ok(r.add_remote(&x[0], &x[1])))));
    v.push(c(Group::Remote, "Rename remote…", "", "", vec![pick("Remote", remote_items.to_vec()), input("New name", "")], git("Rename remote", |r, x| ok(r.rename_remote(&x[0], &x[1])))));
    v.push(c(Group::Remote, "Set remote URL…", "", "change origin", vec![pick("Remote", remote_items.to_vec()), input("New URL", "")], git("Set remote URL", |r, x| ok(r.set_remote_url(&x[0], &x[1])))));
    v.push(c(
        Group::Remote,
        "Remove remote…",
        "",
        "delete",
        vec![pick("Remote", remote_items.to_vec()), confirm("Remove this remote and its remote-tracking branches?")],
        git("Remove remote", |r, x| ok(r.remove_remote(&x[0]))),
    ));
    v.push(c(
        Group::Remote,
        "Delete remote branch…",
        "",
        "push delete",
        vec![pick("Remote branch", remote_branches.to_vec()), confirm("Delete this branch on the remote server?")],
        git("Delete remote branch", |r, x| {
            let (remote, branch) = x[0].split_once('/').unwrap_or(("origin", &x[0]));
            r.delete_remote_branch(remote, branch)
        }),
    ));
}

fn commit(rv: &RepoView, v: &mut Vec<Command>) {
    let st = rv.snap.as_ref().map(|s| &s.status);
    let unstaged: Vec<String> = st.map(|s| s.files.iter().filter(|f| f.is_unstaged()).map(|f| f.path.clone()).collect()).unwrap_or_default();
    let staged: Vec<String> = st.map(|s| s.files.iter().filter(|f| f.is_staged()).map(|f| f.path.clone()).collect()).unwrap_or_default();
    v.push(c(Group::Commit, "Commit…", &format!("{CMD}↵ in message box"), "save", vec![input("Commit message", &rv.commit_msg)], act(|x| vec![Action::CommitWithMessage(x[0].clone())])));
    v.push(c(Group::Commit, "Amend last commit…", "", "edit message fix", vec![], act(|_| vec![Action::StartAmend])));
    if !unstaged.is_empty() {
        let u = unstaged.clone();
        v.push(c(Group::Commit, "Stage all changes", &format!("{} files", u.len()), "add", vec![], act(move |_| vec![Action::Stage(u.clone())])));
        let u = unstaged.clone();
        v.push(c(
            Group::Commit,
            "Discard all unstaged changes…",
            "",
            "reset clean revert",
            vec![confirm(format!("Discard ALL {} unstaged changes? Untracked files are deleted. This cannot be undone.", u.len()))],
            act(move |_| vec![Action::Discard(u.clone())]),
        ));
    }
    if !staged.is_empty() {
        v.push(c(Group::Commit, "Unstage all", &format!("{} files", staged.len()), "reset", vec![], act(move |_| vec![Action::Unstage(staged.clone())])));
    }
    v.push(c(Group::Commit, "Revert commit…", "", "undo", vec![input("Commit to revert", "HEAD"), confirm("Create a new commit that reverts it?")], git("Revert", |r, x| r.revert(&x[0]))));
    v.push(c(Group::Commit, "Cherry-pick commit…", "", "copy apply", vec![input("Commit (SHA or rev)", "")], git("Cherry-pick", |r, x| r.cherry_pick(&x[0]))));
    let modes = vec![
        Item::new("Soft", "keep index and working tree", "soft"),
        Item::new("Mixed", "keep working tree, reset index", "mixed"),
        Item::new("Hard", "discard everything", "hard"),
    ];
    v.push(c(
        Group::Commit,
        "Reset current branch…",
        "",
        "undo move head",
        vec![pick("Mode", modes), input("Reset to", "HEAD~1"), confirm("Reset the current branch?")],
        git("Reset", |r, x| {
            let mode = match x[0].as_str() {
                "soft" => ResetMode::Soft,
                "hard" => ResetMode::Hard,
                _ => ResetMode::Mixed,
            };
            ok(r.reset(&x[1], mode))
        }),
    ));
}

#[allow(clippy::too_many_arguments)]
fn branch(
    rv: &RepoView,
    current: Option<&str>,
    locals: &dyn Fn(bool) -> Vec<Item>,
    all: &[Item],
    remote_branches: &[Item],
    remote_items: &[Item],
    v: &mut Vec<Command>,
) {
    v.push(c(
        Group::Branch,
        "New branch…",
        "",
        "create checkout",
        vec![input("Branch name", ""), input("Start point", "HEAD")],
        act(|x| vec![Action::CreateBranch { name: x[0].clone(), start: Some(x[1].clone()), switch: true }]),
    ));
    v.push(c(Group::Branch, "Checkout branch…", "", "switch", vec![pick("Branch", locals(true))], act(|x| vec![Action::Checkout(x[0].clone())])));
    v.push(c(Group::Branch, "Checkout remote branch…", "tracking", "switch track", vec![pick("Remote branch", remote_branches.to_vec())], act(|x| vec![Action::CheckoutRemote(x[0].clone())])));
    v.push(c(Group::Branch, "Merge branch into current…", "", "combine", vec![pick("Merge which branch", all.to_vec())], act(|x| vec![Action::Merge(x[0].clone())])));
    v.push(c(
        Group::Branch,
        "Compare branches…",
        "review mode",
        "diff review compare changes pull request files changed",
        vec![pick("Compare (the branch with the changes)", all.to_vec()), pick("With base (where the changes would go)", all.to_vec())],
        act(|x| vec![Action::OpenReview { base: Some(x[1].clone()), head: x[0].clone(), pr: None }]),
    ));
    v.push(c(
        Group::Branch,
        "Rebase current branch onto…",
        "",
        "replay",
        vec![pick("Onto", all.to_vec()), confirm("Rebase the current branch? Its commits get new SHAs (commit notes follow by patch-id).")],
        git("Rebase", |r, x| r.rebase_onto(&x[0])),
    ));
    v.push(c(Group::Branch, "Rename branch…", "note follows", "move", vec![pick("Branch", locals(false)), input("New name", "")], act(|x| vec![Action::RenameBranch { old: x[0].clone(), new: x[1].clone() }])));
    v.push(c(
        Group::Branch,
        "Delete branch…",
        "",
        "remove",
        vec![pick("Branch", locals(true)), confirm("Delete this branch? (Refused if unmerged. Its note goes to Detached notes.)")],
        act(|x| vec![Action::DeleteBranch { name: x[0].clone(), force: false }]),
    ));
    v.push(c(
        Group::Branch,
        "Force delete branch…",
        "",
        "remove unmerged",
        vec![pick("Branch", locals(true)), confirm("Force-delete this branch? Unmerged commits will be lost.")],
        act(|x| vec![Action::DeleteBranch { name: x[0].clone(), force: true }]),
    ));
    if let Some(cur) = current {
        let cur = cur.to_string();
        v.push(c(
            Group::Branch,
            "Set upstream of current branch…",
            &cur.clone(),
            "track",
            vec![pick("Upstream", remote_branches.to_vec())],
            git("Set upstream", move |r, x| ok(r.set_upstream(&cur, &x[0]))),
        ));
    }
    v.push(c(
        Group::Branch,
        "Push branch to remote…",
        "",
        "publish upload",
        vec![pick("Branch", locals(false)), pick("Remote", remote_items.to_vec())],
        git("Push branch", |r, x| r.push_branch(&x[1], &x[0])),
    ));
    if let Some(cur) = current {
        let cur = cur.to_string();
        let _ = rv;
        v.push(c(Group::Branch, "Link task to current branch…", "", "jira ticket", vec![], act(move |_| vec![Action::LinkTaskDialog { branch: cur.clone() }])));
    }
}

fn tag(tags: &[Item], remotes: &[Item], default_remote: &str, v: &mut Vec<Command>) {
    v.push(c(Group::Tag, "Create tag…", "lightweight", "new", vec![input("Tag name", ""), input("On commit", "HEAD")], git("Create tag", |r, x| ok(r.create_tag(&x[0], &x[1])))));
    v.push(c(
        Group::Tag,
        "Create annotated tag…",
        "with message",
        "new release",
        vec![input("Tag name", ""), input("Message", ""), input("On commit", "HEAD")],
        git("Create tag", |r, x| ok(r.create_annotated_tag(&x[0], &x[2], &x[1]))),
    ));
    v.push(c(Group::Tag, "Checkout tag…", "detached", "switch", vec![pick("Tag", tags.to_vec())], act(|x| vec![Action::CheckoutDetached(x[0].clone())])));
    v.push(c(Group::Tag, "Delete tag…", "local", "remove", vec![pick("Tag", tags.to_vec()), confirm("Delete this local tag?")], git("Delete tag", |r, x| ok(r.delete_tag(&x[0])))));
    v.push(c(Group::Tag, "Push tag…", "", "publish upload", vec![pick("Tag", tags.to_vec()), pick("Remote", remotes.to_vec())], git("Push tag", |r, x| r.push_tag(&x[1], &x[0]))));
    let dr = default_remote.to_string();
    v.push(c(Group::Tag, "Push all tags", &dr.clone(), "publish upload", vec![], git("Push tags", move |r, _| r.push_all_tags(&dr))));
    v.push(c(
        Group::Tag,
        "Delete remote tag…",
        "",
        "remove push",
        vec![input("Tag name", ""), pick("Remote", remotes.to_vec()), confirm("Delete this tag on the remote server?")],
        git("Delete remote tag", |r, x| r.delete_remote_tag(&x[1], &x[0])),
    ));
}

fn stash(stashes: &[Item], v: &mut Vec<Command>) {
    v.push(c(Group::Stash, "Stash changes", "incl. untracked", "save shelve", vec![], act(|_| vec![Action::StashPush])));
    v.push(c(Group::Stash, "Stash with message…", "", "save shelve", vec![input("Message", "")], git("Stash", |r, x| ok(r.stash_push(Some(&x[0]))))));
    if !stashes.is_empty() {
        v.push(c(Group::Stash, "Pop latest stash", &stashes[0].label, "unshelve", vec![], act(|_| vec![Action::StashPop("stash@{0}".into())])));
        v.push(c(Group::Stash, "Apply stash…", "keep it", "unshelve", vec![pick("Stash", stashes.to_vec())], act(|x| vec![Action::StashApply(x[0].clone())])));
        v.push(c(Group::Stash, "Pop stash…", "", "unshelve", vec![pick("Stash", stashes.to_vec())], act(|x| vec![Action::StashPop(x[0].clone())])));
        v.push(c(Group::Stash, "Drop stash…", "", "delete", vec![pick("Stash", stashes.to_vec()), confirm("Drop this stash? It cannot be recovered easily.")], act(|x| vec![Action::StashDrop(x[0].clone())])));
    }
}

fn worktree(rv: &RepoView, locals: &[&margin_core::git::RefInfo], v: &mut Vec<Command>) {
    let wts = rv.repo.worktrees().unwrap_or_default();
    let here = margin_core::platform::canonical(&rv.repo.workdir);
    let others: Vec<Item> = wts
        .iter()
        .filter(|w| margin_core::platform::canonical(&w.path) != here && !w.bare)
        .map(|w| Item::new(w.path.display().to_string(), w.branch.clone().unwrap_or_else(|| "detached".into()), w.path.display().to_string()))
        .collect();
    let removable: Vec<Item> = wts
        .iter()
        .filter(|w| !w.main)
        .map(|w| Item::new(w.path.display().to_string(), w.branch.clone().unwrap_or_else(|| "detached".into()), w.path.display().to_string()))
        .collect();
    let checked_out: Vec<&str> = wts.iter().filter_map(|w| w.branch.as_deref()).collect();
    let free: Vec<Item> = locals.iter().filter(|r| !checked_out.contains(&r.name.as_str())).map(|r| Item::simple(r.name.clone())).collect();
    // Default location: a sibling folder, e.g. ../myrepo-feature-x
    let parent = rv.repo.workdir.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let base = format!("{}/{}-", parent.display(), rv.repo.name());

    if !others.is_empty() {
        v.push(c(Group::Worktree, "Open worktree…", "", "switch window", vec![pick("Worktree", others)], act(|x| vec![Action::OpenRepo(PathBuf::from(&x[0]))])));
    }
    v.push(c(
        Group::Worktree,
        "Add worktree for branch…",
        "",
        "new checkout folder",
        vec![pick("Branch (not checked out elsewhere)", free), input("Folder", &base)],
        git("Add worktree", |r, x| r.add_worktree(&fix_path(&x[1], &x[0]), &x[0], false, None)),
    ));
    v.push(c(
        Group::Worktree,
        "Add worktree with new branch…",
        "",
        "new checkout folder",
        vec![input("New branch name", ""), input("Start point", "HEAD"), input("Folder", &base)],
        git("Add worktree", |r, x| r.add_worktree(&fix_path(&x[2], &x[0]), &x[0], true, Some(&x[1]))),
    ));
    if !removable.is_empty() {
        v.push(c(
            Group::Worktree,
            "Remove worktree…",
            "",
            "delete folder",
            vec![pick("Worktree", removable.clone()), confirm("Remove this worktree folder? (Refused if it has uncommitted changes.)")],
            git("Remove worktree", |r, x| ok(r.remove_worktree(&x[0], false))),
        ));
        v.push(c(
            Group::Worktree,
            "Force remove worktree…",
            "",
            "delete folder",
            vec![pick("Worktree", removable), confirm("Force-remove this worktree? Uncommitted changes in it are lost.")],
            git("Remove worktree", |r, x| ok(r.remove_worktree(&x[0], true))),
        ));
    }
    v.push(c(Group::Worktree, "Prune worktrees", "drop stale entries", "clean", vec![], git("Prune worktrees", |r, _| r.prune_worktrees())));
}

/// If the folder input still ends with the "<repo>-" stub, append the branch slug.
fn fix_path(path: &str, branch: &str) -> String {
    if path.ends_with('-') {
        format!("{path}{}", branch.replace(['/', ' '], "-"))
    } else {
        path.to_string()
    }
}

fn git_flow(rv: &RepoView, default_remote: &str, v: &mut Vec<Command>) {
    let Some(cfg) = gitflow::config(&rv.repo) else {
        let locals: Vec<String> = rv.snap.as_ref().map(|s| s.refs.iter().filter(|r| r.kind == RefKind::Local).map(|r| r.name.clone()).collect()).unwrap_or_default();
        let main = ["main", "master"].into_iter().find(|m| locals.iter().any(|l| l == m)).unwrap_or("main").to_string();
        v.push(c(
            Group::GitFlow,
            "Initialize git flow…",
            "",
            "init setup",
            vec![input("Production branch", &main), input("Development branch", "develop")],
            git("Git flow init", |r, x| gitflow::init(r, &x[0], &x[1])),
        ));
        return;
    };
    let cfg = Arc::new(cfg);
    for kind in Kind::ALL {
        let k = kind.label();
        let existing: Vec<Item> = gitflow::branches(&rv.repo, &cfg, kind).unwrap_or_default().into_iter().map(Item::simple).collect();
        let (name_prompt, start_steps) = match kind {
            Kind::Release | Kind::Hotfix => ("Version", vec![input("Version", "")]),
            Kind::Support => ("Version", vec![input("Version", ""), input("Base (tag or commit)", &cfg.main)]),
            _ => ("Name", vec![input("Name", "")]),
        };
        let _ = name_prompt;
        let cf = cfg.clone();
        v.push(c(
            Group::GitFlow,
            &format!("{k}: Start…"),
            &format!("from {}", if matches!(kind, Kind::Hotfix | Kind::Support) { &cfg.main } else { &cfg.develop }),
            "git flow start new",
            start_steps,
            git(&format!("Git flow {k} start"), move |r, x| gitflow::start(r, &cf, kind, &x[0], x.get(1).map(String::as_str))),
        ));
        if existing.is_empty() {
            continue;
        }
        if kind.finishable() {
            let mut steps = vec![pick(&format!("{k} to finish"), existing.clone())];
            if kind.tags() {
                steps.push(input("Tag message (empty: default)", ""));
            }
            steps.push(confirm(match kind {
                Kind::Release | Kind::Hotfix => format!("Merge into {} and {}, tag it, and delete the branch?", cfg.main, cfg.develop),
                _ => format!("Merge into {} and delete the branch?", cfg.develop),
            }));
            let cf = cfg.clone();
            v.push(c(
                Group::GitFlow,
                &format!("{k}: Finish…"),
                "",
                "git flow finish merge",
                steps,
                git(&format!("Git flow {k} finish"), move |r, x| {
                    let msg = if kind.tags() { Some(x[1].as_str()) } else { None };
                    gitflow::finish(r, &cf, kind, &x[0], msg)
                }),
            ));
        }
        let cf = cfg.clone();
        let remote = default_remote.to_string();
        v.push(c(
            Group::GitFlow,
            &format!("{k}: Publish…"),
            &format!("push to {remote}"),
            "git flow publish push",
            vec![pick(k, existing)],
            git(&format!("Git flow {k} publish"), move |r, x| gitflow::publish(r, &cf, kind, &x[0], &remote)),
        ));
    }
}

fn github(rv: &RepoView, current: Option<&str>, v: &mut Vec<Command>) {
    let Some(gh) = &rv.github else { return };
    let web = if gh.repo.host == "github.com" { "https://github.com".to_string() } else { format!("https://{}", gh.repo.host) };
    let repo_url = format!("{web}/{}/{}", gh.repo.owner, gh.repo.name);
    if let Some(cur) = current {
        if let Some(pr) = rv.prs.get(cur) {
            let u = pr.url.clone();
            v.push(c(Group::GitHub, "Open pull request", &format!("#{}", pr.number), "pr browser", vec![], act(move |_| vec![Action::OpenUrl(u.clone())])));
        } else {
            let u = format!("{repo_url}/compare/{cur}?expand=1");
            v.push(c(Group::GitHub, "Create pull request", cur, "pr new browser", vec![], act(move |_| vec![Action::OpenUrl(u.clone())])));
        }
    }
    // Review mode for any open pull request
    let prs: Vec<Item> = match &rv.open_prs {
        Some(Ok(list)) => list
            .iter()
            .map(|p| {
                let mark = if p.review_requested { "review requested · " } else if p.mine { "yours · " } else { "" };
                Item::new(format!("#{} {}", p.number, p.title), format!("{mark}{} · {} → {}", p.author, p.head, p.base), p.number.to_string())
            })
            .collect(),
        _ => vec![],
    };
    if prs.is_empty() {
        v.push(c(Group::GitHub, "Review pull request…", "load open pull requests", "pr review code review", vec![], act(|_| vec![Action::FetchOpenPrs])));
    } else {
        v.push(c(
            Group::GitHub,
            "Review pull request…",
            &format!("{} open", prs.len()),
            "pr review code review files changed approve",
            vec![pick("Pull request", prs)],
            act(|x| x[0].parse::<u64>().map(|n| vec![Action::OpenReview { base: None, head: String::new(), pr: Some(n) }]).unwrap_or_default()),
        ));
    }
    if let Some(cur) = current
        && let Some(pr) = rv.prs.get(cur).filter(|p| p.state == "OPEN")
    {
        let n = pr.number;
        v.push(c(Group::GitHub, "Review this branch's pull request", &format!("#{n}"), "pr review code review", vec![], act(move |_| vec![Action::OpenReview { base: None, head: String::new(), pr: Some(n) }])));
    }
    v.push(c(Group::GitHub, "Refresh pull request status", "", "pr ci reload", vec![], act(|_| vec![Action::FetchPrs { force: true }])));
    v.push(c(Group::GitHub, "Open repository on GitHub", "", "browser web", vec![], act(move |_| vec![Action::OpenUrl(repo_url.clone())])));
}

fn notes(rv: &RepoView, current: Option<&str>, v: &mut Vec<Command>) {
    // (repository and task notes are no longer created; existing ones still open)
    if rv.has_note(&margin_core::notes::NoteTarget::Repo) {
        v.push(c(Group::Notes, "Open repository notes", "", "note", vec![], act(|_| vec![Action::OpenNote(margin_core::notes::NoteTarget::Repo)])));
    }
    if current.is_some() {
        v.push(c(Group::Notes, "Open branch notes", "", "note", vec![], act(|_| vec![Action::Select(Sel::Working), Action::ShowNote(NoteTab::Branch)])));
    }
    if let Some(t) = rv.note_task().map(margin_core::notes::NoteTarget::Task).filter(|t| rv.has_note(t)) {
        v.push(c(Group::Notes, "Open task notes", "", "note jira", vec![], act(move |_| vec![Action::OpenNote(t.clone())])));
    }
    if !rv.detached.is_empty() {
        let items: Vec<Item> = rv
            .detached
            .iter()
            .map(|d| Item::new(crate::notes_panel::short_target(&d.note), d.title.clone(), d.target.clone()))
            .collect();
        v.push(c(
            Group::Notes,
            "Open detached note…",
            &format!("{}", rv.detached.len()),
            "orphan",
            vec![pick("Detached note", items)],
            act(|x| margin_core::notes::NoteTarget::parse(&x[0]).map(|t| vec![Action::OpenNote(t)]).unwrap_or_default()),
        ));
    }
}

/// Which branches the history graph shows (see also the eye on each branch).
fn history_filter(rv: &RepoView, current: Option<&str>, branches: &[Item], v: &mut Vec<Command>) {
    use margin_core::history::HistoryFilter;
    let f = rv.graph_filter.clone();
    let set = |f: HistoryFilter| act(move |_| vec![Action::SetGraphFilter(f.clone())]);
    v.push(c(Group::Navigate, "Show all branches in history", "", "unhide graph filter eye", vec![], set(HistoryFilter::reset(true, f.hide_tags))));
    v.push(c(Group::Navigate, "Show only major branches in history", "main, develop, current", "graph filter eye default", vec![], set(HistoryFilter::reset(false, f.hide_tags))));
    if let Some(cur) = current {
        let mut g = f.clone();
        g.only = Some(cur.to_string());
        v.push(c(Group::Navigate, "Show only the current branch in history", cur, "solo graph filter", vec![], set(g)));
    }
    let refs = rv.snap.as_ref().map(|s| s.refs.clone()).unwrap_or_default();
    let base = f.clone();
    v.push(c(
        Group::Navigate,
        "Show or hide a branch in history…",
        "",
        "graph filter eye toggle",
        vec![pick("Branch", branches.to_vec())],
        act(move |x| {
            let mut g = base.clone();
            let up = margin_core::history::head_upstream(&refs);
            match refs.iter().find(|r| r.kind == margin_core::git::RefKind::Local && r.name == x[0]) {
                Some(r) => {
                    let shown = g.shows(r, up.as_deref());
                    g.set(r, !shown);
                    vec![Action::SetGraphFilter(g)]
                }
                None => vec![],
            }
        }),
    ));
    let mut g = f.clone();
    g.hide_tags = !g.hide_tags;
    let t = if f.hide_tags { "Show tags in history" } else { "Hide tags in history" };
    v.push(c(Group::Navigate, t, "", "graph filter", vec![], set(g)));
}

fn navigate(branches: &[Item], tags: &[Item], v: &mut Vec<Command>) {
    v.push(c(Group::Navigate, "Working directory", "", "changes status", vec![], act(|_| vec![Action::Select(Sel::Working)])));
    v.push(c(Group::Navigate, "Go to branch…", "", "jump show", vec![pick("Branch", branches.to_vec())], act(|x| vec![Action::GotoRev(x[0].clone())])));
    v.push(c(Group::Navigate, "Go to tag…", "", "jump show", vec![pick("Tag", tags.to_vec())], act(|x| vec![Action::GotoRev(x[0].clone())])));
    v.push(c(Group::Navigate, "Go to commit…", "", "jump sha", vec![input("SHA or revision", "")], act(|x| vec![Action::GotoRev(x[0].clone())])));
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
