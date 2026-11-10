use crate::diff_view::DiffView;
use crate::jobs::{Jobs, Msg, Snapshot};
use crate::notes_panel::NoteEditor;
use crate::theme;
use eframe::egui;
use margin_core::config::{self, Config, State};
use margin_core::diff;
use margin_core::github::{GitHub, PrDetail, PrSummary};
use margin_core::git::{CommitDetail, FileChange, Repo, ResetMode};
use margin_core::graph::{self, GraphRow};
use margin_core::notes::{NoteStore, NoteTarget};
use margin_core::tasks::{self, Task, TaskProvider};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const PAGE: usize = 2000;

#[derive(Clone, PartialEq, Eq)]
pub enum Sel {
    Working,
    Commit(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HunkOp {
    Stage,
    Unstage,
    Discard,
}

/// Everything the UI can ask for. Views push actions; `MarginApp::handle`
/// executes them after drawing (keeps borrowing simple).
#[derive(Clone)]
pub enum Action {
    Select(Sel),
    SelectFile { path: String, staged: bool },
    Stage(Vec<String>),
    Unstage(Vec<String>),
    Discard(Vec<String>),
    Hunk { file: usize, hunk: usize, op: HunkOp, staged_view: bool },
    Commit,
    Checkout(String),
    CheckoutRemote(String),
    CheckoutDetached(String),
    CreateBranch { name: String, start: Option<String>, switch: bool },
    DeleteBranch { name: String, force: bool },
    Merge(String),
    CherryPick(String),
    Revert(String),
    Reset(String, ResetMode),
    Fetch,
    Pull,
    Push { force: bool },
    StashPush,
    StashPop(String),
    StashApply(String),
    StashDrop(String),
    Refresh,
    LoadMore,
    OpenRepo(PathBuf),
    CloseRepo,
    Confirm { msg: String, action: Box<Action> },
    NewBranchDialog { start: Option<String>, name: String },
    LinkTaskDialog { branch: String },
    LinkTask { branch: String, key: Option<String> },
    FocusBranch(String),
    FocusTask(String),
    ShowNote(crate::notes_panel::NoteTab),
    FetchTask(String),
    FetchMyTasks,
    /// Reload PR status for all local branches (`force` ignores the 60 s throttle).
    FetchPrs { force: bool },
    FetchPrDetail(String),
    OpenUrl(String),
    Copy(String),
    JumpTo(String),
}

pub enum Dialog {
    Confirm { msg: String, action: Action },
    NewBranch { name: String, start: Option<String>, switch: bool },
    LinkTask { branch: String, key: String },
}

pub struct RepoView {
    pub repo: Repo,
    pub notes: NoteStore,
    pub snap: Option<Box<Snapshot>>,
    pub graph: Vec<GraphRow>,
    pub row_of: HashMap<String, usize>,
    pub sel: Sel,
    pub selected_file: Option<(String, bool)>,
    pub work_diff: DiffView,
    pub commit_detail: Option<CommitDetail>,
    pub commit_diff: DiffView,
    pub generation: u64,
    pub commit_msg: String,
    pub amend: bool,
    pub filter: String,
    pub filtered: Option<(String, Vec<usize>)>,
    /// Existing notes → first line (title).
    pub note_index: HashMap<NoteTarget, String>,
    /// Commit id → local branches pointing at it (current branch first).
    pub tips: HashMap<String, Vec<String>>,
    pub github: Option<Arc<GitHub>>,
    pub github_error: Option<String>,
    /// Local branch → its latest PR.
    pub prs: HashMap<String, PrSummary>,
    pub pr_detail: HashMap<String, Result<PrDetail, String>>,
    pub pr_requested: std::collections::HashSet<String>,
    pub prs_fetched_at: Option<f64>,
    pub notes_stamp: u64,
    /// Set by the watcher thread when the notes dir changes.
    pub notes_watch: Arc<AtomicBool>,
    pub editor: NoteEditor,
    pub focus_branch: Option<String>,
    pub focus_task: Option<String>,
    pub scroll_to_row: Option<usize>,
    pub loading: bool,
    pub limit: usize,
}

impl RepoView {
    pub fn current_branch(&self) -> Option<&str> {
        self.snap.as_ref()?.status.branch.as_deref()
    }
    /// The branch whose note/PR the panel shows: an explicitly focused branch,
    /// else the branch whose *tip* is the selected commit, else HEAD's branch.
    /// Because branch notes are keyed by name, they stay on the branch tip
    /// through rebase / amend / new commits.
    pub fn note_branch(&self) -> Option<String> {
        if let Some(b) = &self.focus_branch {
            return Some(b.clone());
        }
        if let Sel::Commit(id) = &self.sel {
            if let Some(b) = self.tips.get(id).and_then(|v| v.first()) {
                return Some(b.clone());
            }
        }
        self.current_branch().map(String::from)
    }
    pub fn note_title(&self, t: &NoteTarget) -> Option<&str> {
        self.note_index.get(t).map(String::as_str)
    }
    pub fn note_task(&self) -> Option<String> {
        if let Some(t) = &self.focus_task {
            return Some(t.clone());
        }
        let b = self.note_branch()?;
        self.notes.task_for_branch(&b)
    }
    pub fn has_note(&self, t: &NoteTarget) -> bool {
        self.note_index.contains_key(t)
    }
    pub fn change(&self, path: &str) -> Option<&FileChange> {
        self.snap.as_ref()?.status.files.iter().find(|f| f.path == path)
    }
}

pub struct MarginApp {
    pub cfg: Config,
    pub state: State,
    pub jobs: Jobs,
    pub rv: Option<RepoView>,
    pub tasks: Option<Arc<dyn TaskProvider>>,
    pub task_cache: HashMap<String, Result<Task, String>>,
    pub my_tasks: Option<Result<Vec<Task>, String>>,
    pub status_line: Option<(String, bool)>,
    pub busy: Option<String>,
    pub dialog: Option<Dialog>,
    pub open_path: String,
    pub was_focused: bool,
    pub author: String,
    pub pending_copy: Option<String>,
    ctx: egui::Context,
}

impl MarginApp {
    pub fn new(ctx: &egui::Context, repo: Option<PathBuf>) -> Self {
        let cfg = config::load_config();
        theme::apply(ctx, cfg.font_size.unwrap_or(13.0));
        let tasks = tasks::provider_from_config(&cfg).map(Arc::from);
        let mut app = MarginApp {
            author: cfg.note_author.clone().unwrap_or_default(),
            state: config::load_state(),
            cfg: cfg.clone(),
            jobs: Jobs::new(ctx),
            rv: None,
            tasks,
            task_cache: HashMap::new(),
            my_tasks: None,
            status_line: None,
            busy: None,
            dialog: None,
            open_path: String::new(),
            was_focused: true,
            pending_copy: None,
            ctx: ctx.clone(),
        };
        if let Some(p) = repo {
            if Repo::open(&p).is_ok() {
                app.handle(Action::OpenRepo(p));
            }
        }
        app
    }

    fn err(&mut self, e: impl ToString) {
        self.status_line = Some((e.to_string(), true));
    }

    fn info(&mut self, s: impl ToString) {
        self.status_line = Some((s.to_string(), false));
    }

    pub fn refresh(&mut self) {
        let Some(rv) = &mut self.rv else { return };
        rv.loading = true;
        let repo = rv.repo.clone();
        let limit = rv.limit;
        self.jobs.spawn(move || {
            // status (slow in monorepos), log and refs run concurrently.
            let r = std::thread::scope(|s| {
                let st = s.spawn(|| repo.status());
                let refs = s.spawn(|| repo.refs());
                let log = repo.log(limit, 0);
                let status = st.join().unwrap()?;
                let refs = refs.join().unwrap()?;
                let commits = log?;
                Ok::<_, margin_core::Error>(Box::new(Snapshot {
                    status,
                    commits,
                    refs,
                    stashes: repo.stashes().unwrap_or_default(),
                    pending_op: repo.pending_operation(),
                    limit,
                }))
            });
            Msg::Refreshed(r.map_err(|e| e.0))
        });
    }

    fn load_work_diff(&mut self) {
        let Some(rv) = &mut self.rv else { return };
        let Some((path, staged)) = rv.selected_file.clone() else {
            rv.work_diff.clear();
            return;
        };
        let Some(change) = rv.change(&path).cloned() else {
            rv.selected_file = None;
            rv.work_diff.clear();
            return;
        };
        rv.generation += 1;
        rv.work_diff.loading = true;
        let generation = rv.generation;
        let repo = rv.repo.clone();
        self.jobs.spawn(move || Msg::WorkDiff {
            generation,
            result: repo.file_diff(&change, staged).map(|t| diff::parse(&t)).map_err(|e| e.0),
        });
    }

    fn load_commit(&mut self, id: String) {
        let Some(rv) = &mut self.rv else { return };
        rv.generation += 1;
        rv.commit_diff.loading = true;
        rv.commit_diff.files.clear();
        let generation = rv.generation;
        let repo = rv.repo.clone();
        self.jobs.spawn(move || {
            let r = (|| {
                let d = repo.commit_detail(&id)?;
                let (files, trunc) = diff::parse(&repo.commit_diff(&id)?);
                Ok::<_, margin_core::Error>((d, files, trunc))
            })();
            Msg::CommitLoaded { generation, result: r.map_err(|e| e.0) }
        });
    }

    fn op(&mut self, label: &str, f: impl FnOnce(&Repo) -> margin_core::Result<String> + Send + 'static) {
        let Some(rv) = &self.rv else { return };
        if let Some(b) = &self.busy {
            self.err(format!("Busy: {b}…"));
            return;
        }
        self.busy = Some(label.to_string());
        let repo = rv.repo.clone();
        let label = label.to_string();
        self.jobs.spawn(move || Msg::OpDone { result: f(&repo).map_err(|e| e.0), label, refresh: true });
    }

    /// Poll the notes dir off the UI thread (1 Hz, a few stat calls) and wake
    /// the UI only when something changed — zero repaints while idle.
    /// The thread exits when the repo is closed (it holds the last Arc).
    fn start_notes_watcher(&mut self) {
        let Some(rv) = &self.rv else { return };
        let flag = rv.notes_watch.clone();
        let notes = rv.notes.clone();
        let ctx = self.ctx.clone();
        let mut last = rv.notes_stamp;
        std::thread::spawn(move || {
            while Arc::strong_count(&flag) > 1 {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let s = notes.stamp();
                if s != last {
                    last = s;
                    flag.store(true, Ordering::Relaxed);
                    ctx.request_repaint();
                }
            }
        });
    }

    pub fn reload_notes_index(&mut self) {
        if let Some(rv) = &mut self.rv {
            rv.note_index = rv.notes.list().map(|l| l.into_iter().map(|m| (m.target, m.title)).collect()).unwrap_or_default();
            rv.notes_stamp = rv.notes.stamp();
        }
    }

    pub fn handle(&mut self, a: Action) {
        use Action::*;
        match a {
            OpenRepo(p) => match Repo::open(&p) {
                Ok(repo) => {
                    let notes = match NoteStore::for_repo(&repo) {
                        Ok(n) => n,
                        Err(e) => return self.err(e),
                    };
                    if self.author.is_empty() {
                        self.author = repo.run(&["config", "user.name"]).map(|s| s.trim().to_string()).unwrap_or_else(|_| "me".into());
                    }
                    let path = repo.workdir.display().to_string();
                    self.state.recent_repos.retain(|r| r != &path);
                    self.state.recent_repos.insert(0, path);
                    self.state.recent_repos.truncate(15);
                    let _ = config::save_state(&self.state);
                    self.rv = Some(RepoView {
                        repo,
                        notes,
                        snap: None,
                        graph: vec![],
                        row_of: HashMap::new(),
                        sel: Sel::Working,
                        selected_file: None,
                        work_diff: DiffView::default(),
                        commit_detail: None,
                        commit_diff: DiffView::default(),
                        generation: 0,
                        commit_msg: String::new(),
                        amend: false,
                        filter: String::new(),
                        filtered: None,
                        note_index: HashMap::new(),
                        tips: HashMap::new(),
                        github: None,
                        github_error: None,
                        prs: HashMap::new(),
                        pr_detail: HashMap::new(),
                        pr_requested: Default::default(),
                        prs_fetched_at: None,
                        notes_stamp: 0,
                        notes_watch: Arc::new(AtomicBool::new(false)),
                        editor: NoteEditor::default(),
                        focus_branch: None,
                        focus_task: None,
                        scroll_to_row: None,
                        loading: true,
                        limit: PAGE,
                    });
                    self.reload_notes_index();
                    self.start_notes_watcher();
                    self.refresh();
                    self.connect_github();
                }
                Err(e) => self.err(format!("{}: {e}", p.display())),
            },
            CloseRepo => {
                if let Some(rv) = &mut self.rv {
                    rv.editor.save_if_dirty(&rv.notes);
                }
                self.rv = None;
            }
            Refresh => self.refresh(),
            LoadMore => {
                if let Some(rv) = &mut self.rv {
                    rv.limit += PAGE;
                }
                self.refresh();
            }
            Select(sel) => {
                let Some(rv) = &mut self.rv else { return };
                if rv.sel == sel {
                    return;
                }
                rv.sel = sel.clone();
                rv.focus_task = None;
                rv.focus_branch = None;
                match sel {
                    Sel::Working => {
                        rv.commit_detail = None;
                        rv.editor.tab = crate::notes_panel::NoteTab::Branch;
                        self.load_work_diff();
                    }
                    Sel::Commit(id) => {
                        // A branch tip without its own commit note opens on the
                        // branch note — that's where the branch's notes "live".
                        let own = rv.has_note(&NoteTarget::Commit(id.clone()));
                        let tip_note = rv.tips.contains_key(&id)
                            && rv.note_branch().is_some_and(|b| rv.has_note(&NoteTarget::Branch(b)));
                        rv.editor.tab = if !own && tip_note {
                            crate::notes_panel::NoteTab::Branch
                        } else {
                            crate::notes_panel::NoteTab::Commit
                        };
                        self.load_commit(id);
                    }
                }
            }
            JumpTo(id) => {
                let Some(rv) = &mut self.rv else { return };
                if let Some(&row) = rv.row_of.get(&id) {
                    rv.scroll_to_row = Some(row + 1);
                    rv.filter.clear();
                }
                self.handle(Select(Sel::Commit(id)));
            }
            SelectFile { path, staged } => {
                if let Some(rv) = &mut self.rv {
                    rv.selected_file = Some((path, staged));
                }
                self.load_work_diff();
            }
            Stage(paths) => self.op("Stage", move |r| {
                r.stage(&paths.iter().map(String::as_str).collect::<Vec<_>>()).map(|_| String::new())
            }),
            Unstage(paths) => self.op("Unstage", move |r| {
                r.unstage(&paths.iter().map(String::as_str).collect::<Vec<_>>()).map(|_| String::new())
            }),
            Discard(paths) => {
                let Some(rv) = &self.rv else { return };
                let changes: Vec<FileChange> = paths.iter().filter_map(|p| rv.change(p).cloned()).collect();
                self.op("Discard", move |r| r.discard(&changes.iter().collect::<Vec<_>>()).map(|_| String::new()));
            }
            Hunk { file, hunk, op, staged_view } => {
                let Some(rv) = &self.rv else { return };
                let Some(patch) = rv.work_diff.files.get(file).and_then(|f| f.hunk_patch(hunk)) else { return };
                let _ = staged_view;
                self.op("Hunk", move |r| {
                    match op {
                        HunkOp::Stage => r.apply_patch(&patch, true, false),
                        HunkOp::Unstage => r.apply_patch(&patch, true, true),
                        HunkOp::Discard => r.apply_patch(&patch, false, true),
                    }
                    .map(|_| String::new())
                });
            }
            Commit => {
                let Some(rv) = &self.rv else { return };
                let msg = rv.commit_msg.trim().to_string();
                if msg.is_empty() {
                    return self.err("Commit message is empty");
                }
                let amend = rv.amend;
                self.op("Commit", move |r| r.commit(&msg, amend));
            }
            Checkout(b) => self.op("Checkout", move |r| r.checkout(&b).map(|_| String::new())),
            CheckoutRemote(b) => self.op("Checkout", move |r| r.checkout_remote(&b).map(|_| String::new())),
            CheckoutDetached(id) => self.op("Checkout", move |r| r.checkout_detached(&id).map(|_| String::new())),
            CreateBranch { name, start, switch } => self.op("Create branch", move |r| {
                r.create_branch(&name, start.as_deref(), switch).map(|_| String::new())
            }),
            DeleteBranch { name, force } => self.op("Delete branch", move |r| r.delete_branch(&name, force).map(|_| String::new())),
            Merge(b) => self.op("Merge", move |r| r.merge(&b)),
            CherryPick(id) => self.op("Cherry-pick", move |r| r.cherry_pick(&id)),
            Revert(id) => self.op("Revert", move |r| r.revert(&id)),
            Reset(id, mode) => self.op("Reset", move |r| r.reset(&id, mode).map(|_| String::new())),
            Fetch => self.op("Fetch", |r| r.fetch()),
            Pull => self.op("Pull", |r| r.pull()),
            Push { force } => {
                let Some(rv) = &self.rv else { return };
                let needs_upstream = rv.snap.as_ref().is_some_and(|s| s.status.upstream.is_none());
                self.op("Push", move |r| r.push(needs_upstream, force));
            }
            StashPush => self.op("Stash", |r| r.stash_push(None).map(|_| String::new())),
            StashPop(n) => self.op("Stash pop", move |r| r.stash_pop(&n).map(|_| String::new())),
            StashApply(n) => self.op("Stash apply", move |r| r.stash_apply(&n).map(|_| String::new())),
            StashDrop(n) => self.op("Stash drop", move |r| r.stash_drop(&n).map(|_| String::new())),
            Confirm { msg, action } => self.dialog = Some(Dialog::Confirm { msg, action: *action }),
            NewBranchDialog { start, name } => self.dialog = Some(Dialog::NewBranch { name, start, switch: true }),
            LinkTaskDialog { branch } => {
                let key = self.rv.as_ref().and_then(|rv| rv.notes.task_for_branch(&branch)).unwrap_or_default();
                self.dialog = Some(Dialog::LinkTask { branch, key });
            }
            LinkTask { branch, key } => {
                let Some(rv) = &self.rv else { return };
                match rv.notes.link_task(&branch, key.as_deref()) {
                    Ok(()) => self.info(format!("Linked {branch} → {}", key.as_deref().unwrap_or("(inferred)"))),
                    Err(e) => self.err(e),
                }
            }
            FocusBranch(b) => {
                if let Some(rv) = &mut self.rv {
                    rv.focus_branch = Some(b);
                    rv.focus_task = None;
                    rv.editor.tab = crate::notes_panel::NoteTab::Branch;
                }
            }
            FocusTask(k) => {
                if let Some(rv) = &mut self.rv {
                    rv.focus_task = Some(k.clone());
                    rv.editor.tab = crate::notes_panel::NoteTab::Task;
                }
                if !self.task_cache.contains_key(&k) {
                    self.handle(FetchTask(k));
                }
            }
            ShowNote(tab) => {
                if let Some(rv) = &mut self.rv {
                    rv.editor.tab = tab;
                }
            }
            FetchTask(key) => {
                let Some(p) = self.tasks.clone() else { return };
                self.jobs.spawn(move || Msg::Task { result: p.get(&key).map_err(|e| e.0), key });
            }
            FetchPrs { force } => {
                let now = self.ctx.input(|i| i.time);
                let Some(rv) = &mut self.rv else { return };
                let Some(gh) = rv.github.clone() else { return };
                let Some(snap) = &rv.snap else { return };
                if !force && rv.prs_fetched_at.is_some_and(|t| now - t < 60.0) {
                    return;
                }
                rv.prs_fetched_at = Some(now);
                // Local branch → name of its head on GitHub (upstream may differ).
                let heads: Vec<(String, String)> = snap
                    .refs
                    .iter()
                    .filter(|r| r.kind == margin_core::git::RefKind::Local)
                    .take(40) // most recently committed first
                    .map(|r| (r.name.clone(), r.upstream_branch.clone().unwrap_or_else(|| r.name.clone())))
                    .collect();
                let workdir = rv.repo.workdir.clone();
                self.jobs.spawn(move || {
                    let names: Vec<String> = heads.iter().map(|(_, h)| h.clone()).collect();
                    let result = gh.pr_summaries(&names).map(|by_head| {
                        heads.iter().filter_map(|(local, head)| by_head.get(head).map(|p| (local.clone(), p.clone()))).collect()
                    });
                    Msg::Prs { workdir, result: result.map_err(|e| e.0) }
                });
            }
            FetchPrDetail(branch) => {
                let Some(rv) = &mut self.rv else { return };
                let Some(gh) = rv.github.clone() else { return };
                let head = rv
                    .snap
                    .as_ref()
                    .and_then(|s| s.refs.iter().find(|r| r.kind == margin_core::git::RefKind::Local && r.name == branch))
                    .and_then(|r| r.upstream_branch.clone())
                    .unwrap_or_else(|| branch.clone());
                rv.pr_requested.insert(branch.clone());
                let workdir = rv.repo.workdir.clone();
                self.jobs.spawn(move || Msg::PrDetail {
                    workdir,
                    result: gh.pr_detail(&head).map_err(|e| e.0),
                    branch,
                });
            }
            FetchMyTasks => {
                let Some(p) = self.tasks.clone() else { return };
                self.jobs.spawn(move || Msg::MyTasks(p.my_open(30).map_err(|e| e.0)));
            }
            OpenUrl(url) => open_url(&url, self),
            Copy(s) => {
                self.info(format!("Copied {s}"));
                self.pending_copy = Some(s);
            }
        }
    }

    /// Resolve the GitHub remote + token off the UI thread (`gh auth token`
    /// is a subprocess), then load PR status.
    fn connect_github(&mut self) {
        let Some(rv) = &self.rv else { return };
        let repo = rv.repo.clone();
        let cfg = self.cfg.clone();
        let workdir = repo.workdir.clone();
        self.jobs.spawn(move || Msg::GithubReady {
            workdir,
            result: GitHub::for_repo(&repo, &cfg, None).map(|o| o.map(Arc::new)).map_err(|e| e.0),
        });
    }

    fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::GithubReady { workdir, result } => {
                let Some(rv) = self.rv.as_mut().filter(|rv| rv.repo.workdir == workdir) else { return };
                match result {
                    Ok(gh) => rv.github = gh,
                    Err(e) => rv.github_error = Some(e),
                }
                self.handle(Action::FetchPrs { force: true });
            }
            Msg::Prs { workdir, result } => {
                let Some(rv) = self.rv.as_mut().filter(|rv| rv.repo.workdir == workdir) else { return };
                match result {
                    Ok(prs) => {
                        rv.prs = prs;
                        rv.github_error = None;
                        // Details are reloaded lazily for whatever is on screen.
                        rv.pr_detail.clear();
                        rv.pr_requested.clear();
                    }
                    Err(e) => rv.github_error = Some(e),
                }
            }
            Msg::PrDetail { workdir, branch, result } => {
                let Some(rv) = self.rv.as_mut().filter(|rv| rv.repo.workdir == workdir) else { return };
                rv.pr_detail.insert(branch, result.and_then(|d| d.ok_or_else(|| "No pull request for this branch".to_string())));
            }
            Msg::Refreshed(r) => {
                let Some(rv) = &mut self.rv else { return };
                rv.loading = false;
                match r {
                    Ok(snap) => {
                        let (g, _w) = graph::layout(&snap.commits);
                        rv.graph = g;
                        rv.row_of = snap.commits.iter().enumerate().map(|(i, c)| (c.id.clone(), i)).collect();
                        rv.tips.clear();
                        for r in snap.refs.iter().filter(|r| r.kind == margin_core::git::RefKind::Local) {
                            let v = rv.tips.entry(r.target.clone()).or_default();
                            if r.is_head { v.insert(0, r.name.clone()) } else { v.push(r.name.clone()) }
                        }
                        rv.filtered = None;
                        rv.snap = Some(snap);
                        if rv.amend && rv.commit_msg.is_empty() {
                            rv.commit_msg = rv.repo.last_commit_message().unwrap_or_default();
                        }
                        if rv.sel == Sel::Working {
                            self.load_work_diff();
                        }
                        self.handle(Action::FetchPrs { force: false });
                    }
                    Err(e) => self.err(e),
                }
            }
            Msg::WorkDiff { generation, result } => {
                let Some(rv) = &mut self.rv else { return };
                if generation != rv.generation {
                    return;
                }
                match result {
                    Ok((f, t)) => rv.work_diff.set(f, t),
                    Err(e) => rv.work_diff.set_error(e),
                }
            }
            Msg::CommitLoaded { generation, result } => {
                let Some(rv) = &mut self.rv else { return };
                if generation != rv.generation {
                    return;
                }
                match result {
                    Ok((d, f, t)) => {
                        rv.commit_detail = Some(d);
                        rv.commit_diff.set(f, t);
                    }
                    Err(e) => rv.commit_diff.set_error(e),
                }
            }
            Msg::OpDone { label, result, refresh } => {
                self.busy = None;
                match result {
                    Ok(out) => {
                        if label == "Commit" {
                            if let Some(rv) = &mut self.rv {
                                rv.commit_msg.clear();
                                rv.amend = false;
                            }
                        }
                        let first = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
                        self.info(if first.is_empty() { format!("{label}: done") } else { format!("{label}: {first}") });
                    }
                    Err(e) => self.err(format!("{label} failed: {e}")),
                }
                if refresh {
                    self.refresh();
                }
                if matches!(label.as_str(), "Push" | "Fetch" | "Pull") {
                    if let Some(rv) = &mut self.rv {
                        rv.prs_fetched_at = None; // reload PR status after the refresh
                    }
                }
            }
            Msg::Task { key, result } => {
                if let Err(e) = &result {
                    self.err(format!("{key}: {e}"));
                }
                self.task_cache.insert(key, result);
            }
            Msg::MyTasks(r) => self.my_tasks = Some(r),
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.dialog else { return };
        let mut run: Option<Action> = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("dialog")).show(ctx, |ui| {
            ui.set_width(380.0);
            match d {
                Dialog::Confirm { msg, action } => {
                    ui.label(msg.as_str());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            run = Some(action.clone());
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::NewBranch { name, start, switch } => {
                    ui.heading("New branch");
                    if let Some(s) = start {
                        ui.weak(format!("from {}", &s[..s.len().min(12)]));
                    }
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("feature/PROJ-123-short-name").desired_width(f32::INFINITY));
                    r.request_focus();
                    ui.checkbox(switch, "Check out after creating");
                    ui.horizontal(|ui| {
                        let ok = !name.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("Create")).clicked() || (ok && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                            run = Some(Action::CreateBranch { name: name.trim().to_string(), start: start.clone(), switch: *switch });
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::LinkTask { branch, key } => {
                    ui.heading("Link task");
                    ui.weak(format!("branch {branch}"));
                    ui.add(egui::TextEdit::singleline(key).hint_text("PROJ-123 (empty = infer from branch name)").desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            let k = key.trim();
                            run = Some(Action::LinkTask { branch: branch.clone(), key: (!k.is_empty()).then(|| k.to_string()) });
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        if modal.should_close() {
            close = true;
        }
        if close {
            self.dialog = None;
        }
        if let Some(a) = run {
            self.handle(a);
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui) -> Vec<Action> {
        let mut actions = vec![];
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.heading("Margin");
                ui.weak("git + notes + tasks");
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.open_path).hint_text("/path/to/repo").desired_width(360.0));
                    if ui.button("Open").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        actions.push(Action::OpenRepo(PathBuf::from(self.open_path.trim())));
                    }
                    #[cfg(target_os = "macos")]
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            actions.push(Action::OpenRepo(p));
                        }
                    }
                });
                ui.add_space(16.0);
                for r in self.state.recent_repos.clone() {
                    if ui.link(&r).clicked() {
                        actions.push(Action::OpenRepo(PathBuf::from(r)));
                    }
                }
            });
        });
        actions
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let Some(rv) = &mut self.rv else { return };
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                if ui.button("◀").on_hover_text("Close repository").clicked() {
                    actions.push(Action::CloseRepo);
                }
                ui.strong(rv.repo.name());
                if let Some(s) = &rv.snap {
                    let b = s.status.branch.clone().unwrap_or_else(|| "(detached)".into());
                    let mut t = format!("  {b}");
                    if s.status.ahead > 0 {
                        t += &format!("  ↑{}", s.status.ahead);
                    }
                    if s.status.behind > 0 {
                        t += &format!("  ↓{}", s.status.behind);
                    }
                    ui.label(egui::RichText::new(t).color(theme::ACCENT));
                    if let Some(op) = s.pending_op {
                        ui.colored_label(theme::ERROR, format!("  {op} in progress"));
                    }
                }
                ui.separator();
                let idle = self.busy.is_none();
                ui.add_enabled_ui(idle, |ui| {
                    if ui.button("Fetch").clicked() {
                        actions.push(Action::Fetch);
                    }
                    if ui.button("Pull").clicked() {
                        actions.push(Action::Pull);
                    }
                    if ui.button("Push").clicked() {
                        actions.push(Action::Push { force: false });
                    }
                    if ui.button("Stash").clicked() {
                        actions.push(Action::StashPush);
                    }
                });
                if let Some(b) = &self.busy {
                    ui.spinner();
                    ui.weak(format!("{b}…"));
                } else if rv.loading || self.jobs.in_flight > 0 {
                    ui.spinner();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⟳").on_hover_text("Refresh (⌘R)").clicked() {
                        actions.push(Action::Refresh);
                    }
                    ui.add(egui::TextEdit::singleline(&mut rv.filter).hint_text("Filter commits…").desired_width(220.0));
                });
            });
            ui.add_space(2.0);
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("statusbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                match &self.status_line {
                    Some((m, err)) => {
                        let m = m.lines().next().unwrap_or("").to_string();
                        let resp = ui.add(egui::Label::new(egui::RichText::new(&m).color(if *err { theme::ERROR } else { theme::TEXT_DIM })).sense(egui::Sense::click()));
                        if let Some((full, _)) = &self.status_line {
                            if full.lines().count() > 1 {
                                resp.clone().on_hover_text(full);
                            }
                        }
                        if resp.clicked() {
                            self.status_line = None;
                        }
                    }
                    None => {
                        ui.weak("Ready");
                    }
                }
                if let Some(rv) = &self.rv {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(format!("{} notes", rv.note_index.len()));
                    });
                }
            });
        });
    }
}

impl eframe::App for MarginApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        for m in self.jobs.poll() {
            self.on_msg(m);
        }
        if let Some(s) = self.pending_copy.take() {
            ctx.copy_text(s);
        }

        // Refresh when the window regains focus (like Sublime Merge) and
        // pick up note edits made by agents (see start_notes_watcher).
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if focused && !self.was_focused {
            self.refresh();
        }
        self.was_focused = focused;
        let notes_changed = self.rv.as_ref().is_some_and(|rv| rv.notes_watch.swap(false, Ordering::Relaxed));
        if notes_changed {
            self.reload_notes_index();
            if let Some(rv) = &mut self.rv {
                rv.editor.on_external_change(&rv.notes);
            }
        }

        // Global shortcuts
        let (refresh, save) = ctx.input_mut(|i| {
            (i.consume_key(egui::Modifiers::COMMAND, egui::Key::R), i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
        });
        if refresh {
            self.refresh();
        }
        if save {
            if let Some(rv) = &mut self.rv {
                rv.editor.save(&rv.notes);
                self.reload_notes_index();
            }
        }

        let mut actions = Vec::new();
        if self.rv.is_none() {
            actions = self.welcome(ui);
        } else {
            self.toolbar(ui, &mut actions);
            self.status_bar(ui);
            let tasks_enabled = self.tasks.is_some();
            let rv = self.rv.as_mut().unwrap();
            egui::Panel::left("sidebar").resizable(true).default_size(220.0).show(ui, |ui| {
                crate::sidebar::show(ui, rv, tasks_enabled, self.my_tasks.as_ref(), &mut actions);
            });
            egui::Panel::right("details").resizable(true).default_size(560.0).min_size(360.0).show(ui, |ui| {
                egui::Panel::bottom("notes").resizable(true).default_size(300.0).min_size(120.0).show(ui, |ui| {
                    // Fill the panel so it keeps its size instead of shrinking to content.
                    ui.set_min_height(ui.available_height());
                    let task_url = |k: &str| tasks::task_url(&self.cfg, self.tasks.as_deref(), k);
                    crate::notes_panel::show(ui, rv, &self.task_cache, tasks_enabled, &task_url, &self.author, &mut actions);
                });
                egui::CentralPanel::default().show(ui, |ui| {
                    crate::details::show(ui, rv, &mut actions);
                });
            });
            egui::CentralPanel::default().show(ui, |ui| {
                crate::commit_list::show(ui, rv, &mut actions);
            });
        }
        self.dialogs(&ctx);
        for a in actions {
            self.handle(a);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(rv) = &mut self.rv {
            rv.editor.save_if_dirty(&rv.notes);
        }
    }
}

fn open_url(url: &str, app: &mut MarginApp) {
    if let Some(target) = url.strip_prefix("margin:") {
        // Internal links: margin:commit:<sha>, margin:task:KEY, margin:branch:name
        match NoteTarget::parse(target) {
            Ok(NoteTarget::Commit(c)) => {
                let full = app.rv.as_ref().and_then(|rv| rv.repo.resolve(&c).ok()).unwrap_or(c);
                app.handle(Action::JumpTo(full));
            }
            Ok(NoteTarget::Task(k)) => app.handle(Action::FocusTask(k)),
            Ok(NoteTarget::Branch(b)) => app.handle(Action::FocusBranch(b)),
            Ok(NoteTarget::Repo) => app.handle(Action::ShowNote(crate::notes_panel::NoteTab::Repo)),
            Err(e) => app.err(e),
        }
        return;
    }
    if !margin_core::markdown::link_allowed(url) || !(url.starts_with("http") || url.starts_with("mailto:")) {
        return app.err(format!("Refusing to open {url}"));
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(url).spawn();
    if let Err(e) = r {
        app.err(format!("open {url}: {e}"));
    }
}
