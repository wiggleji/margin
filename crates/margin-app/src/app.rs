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
    /// Pick a folder with the system dialog and open it (macOS); elsewhere
    /// asks for a path in the palette.
    OpenFolderDialog,
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
    /// Reply to a PR review thread (id) on the PR for `branch`.
    ReplyThread { id: String, branch: String, body: String },
    /// Resolve or reopen a PR review thread.
    ResolveThread { id: String, branch: String, resolve: bool },
    /// Start a review thread on a line of the selected commit (PR of `branch`).
    NewThread { branch: String, path: String, line: u64, old_side: bool, body: String },
    RenameBranchDialog(String),
    RenameBranch { old: String, new: String },
    /// Show any note (e.g. a detached one) in the notes panel.
    OpenNote(NoteTarget),
    ReattachDialog(NoteTarget),
    /// Move a note; `to` is `commit:<rev>` or `branch:<name>`.
    MoveNote { from: NoteTarget, to: String },
    DeleteNote(NoteTarget),
    /// Any git operation, run in the background (palette commands).
    RunGit { label: String, f: Arc<dyn Fn(&Repo) -> margin_core::Result<String> + Send + Sync> },
    OpenPath { path: PathBuf, terminal: bool },
    CommitWithMessage(String),
    StartAmend,
    /// Resolve a branch/tag/SHA and jump to it.
    GotoRev(String),
    OpenPalette,
    /// Open the palette with a prefilled search (e.g. "branch").
    OpenPaletteWith(String),
    /// Start a palette command directly by key (`Group/Title`), e.g. from the menu bar.
    RunPaletteCommand(String),
    NavBack,
    NavForward,
    OpenSettings,
    InstallLicense(String),
    /// Enable/disable notes sync for the active repo.
    SetNotesSync(bool),
    SetSyncFolder(Option<String>),
    SyncNow,
    RemoveLicense,
    SwitchTab(usize),
    NextTab,
    PrevTab,
    /// Close the tab at a visual index.
    CloseTab(usize),
    ToggleAutoFetch,
    /// Move the notes panel between the details and the history column.
    ToggleNotesDock,
    /// Change which branches the history graph shows (saved per repository).
    SetGraphFilter(margin_core::history::HistoryFilter),
    /// Reload the shown diff (e.g. after toggling "Ignore whitespace").
    ReloadDiff,
    /// Give the diff the whole window (hide branches, history, left notes) or undo it.
    ToggleFocusDiff,
    /// Revert an automatic re-attachment (journal id).
    UndoReattach(String),
    /// Confirm an automatic re-attachment (stop offering undo).
    KeepReattach(String),
    ScanDetached,
    OpenUrl(String),
    Copy(String),
    JumpTo(String),
}

/// Text size bounds and default (points).
const MIN_FONT: f32 = 11.0;
const MAX_FONT: f32 = 20.0;
const DEFAULT_FONT: f32 = 13.0;
/// The interface size moves in a narrow range so the layout stays put.
const MIN_UI_FONT: f32 = 12.0;
const MAX_UI_FONT: f32 = 15.0;

/// Sections of the Settings window.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Appearance,
    Pro,
    Sync,
    Fetch,
    Tasks,
    Diffs,
}

impl SettingsTab {
    const ALL: [SettingsTab; 6] = [SettingsTab::Appearance, SettingsTab::Pro, SettingsTab::Sync, SettingsTab::Fetch, SettingsTab::Tasks, SettingsTab::Diffs];
    fn label(self) -> &'static str {
        match self {
            SettingsTab::Appearance => "Appearance",
            SettingsTab::Pro => "Margin Pro",
            SettingsTab::Sync => "Notes sync",
            SettingsTab::Fetch => "Fetch",
            SettingsTab::Tasks => "Tasks",
            SettingsTab::Diffs => "Diffs",
        }
    }
    fn intro(self) -> &'static str {
        match self {
            SettingsTab::Appearance => "How Margin looks. Themes apply right away.",
            SettingsTab::Pro => "GitHub PR status, Jira ticket details and My tasks, and notes sync for more than one repository.",
            SettingsTab::Sync => "Keep notes in a folder that iCloud Drive, Dropbox or Google Drive already syncs, so they follow you to other computers.",
            SettingsTab::Fetch => "Keep remote branches up to date in the background.",
            SettingsTab::Tasks => "Where task keys such as PROJ-123 link to.",
            SettingsTab::Diffs => "How diffs open.",
        }
    }
}

/// One setting: a title, its control(s), and a short explanation underneath.
fn setting(ui: &mut egui::Ui, title: &str, help: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(egui::RichText::new(title).strong());
    ui.add_space(4.0);
    add(ui);
    if !help.is_empty() {
        ui.add_space(3.0);
        ui.label(egui::RichText::new(help).small().color(theme::TEXT_DIM()));
    }
    ui.add_space(16.0);
}

/// A font dropdown: the built-in font, then the installed families (for
/// code: fixed-pitch ones first). Has a filter field. Returns true on a pick.
fn font_picker(ui: &mut egui::Ui, id: &str, current: &mut Option<margin_core::config::FontChoice>, code: bool) -> bool {
    let builtin = if code { "Default (Hack)" } else { "Default (Ubuntu)" };
    let shown = current.as_ref().map_or(builtin, |c| c.family.as_str()).to_string();
    let filter_id = ui.scope_id().with((id, "filter"));
    let mut picked = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(260.0)
        .height(340.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            // (a fixed height: the popup keeps the size of its first frame)
            ui.set_min_height(300.0);
            let Some(list) = crate::fonts::families(ui.ctx()) else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Finding installed fonts…");
                });
                return;
            };
            let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
            let r = ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Filter").desired_width(f32::INFINITY));
            if !r.has_focus() && filter.is_empty() {
                r.request_focus();
            }
            ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
            ui.separator();
            let q = filter.to_lowercase();
            if q.is_empty() && ui.selectable_label(current.is_none(), builtin).clicked() {
                picked = Some(None);
            }
            let matches = |f: &&crate::fonts::Family| q.is_empty() || f.name.to_lowercase().contains(&q);
            let groups: Vec<(&str, Vec<&crate::fonts::Family>)> = if code {
                vec![("Fixed width", list.iter().filter(|f| f.mono).filter(matches).collect()), ("Other fonts", list.iter().filter(|f| !f.mono).filter(matches).collect())]
            } else {
                vec![("", list.iter().filter(matches).collect())]
            };
            for (title, fams) in groups {
                if fams.is_empty() {
                    continue;
                }
                if !title.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(title).small().color(theme::TEXT_DIM()));
                }
                for f in fams {
                    let on = current.as_ref().is_some_and(|c| c.family == f.name);
                    if ui.selectable_label(on, &f.name).clicked() {
                        picked = Some(Some(f.choice.clone()));
                    }
                }
            }
        });
    match picked {
        Some(p) => {
            ui.data_mut(|d| d.remove::<String>(filter_id));
            egui::Popup::close_all(ui.ctx());
            *current = p;
            true
        }
        None => false,
    }
}

/// A clickable preview card for a theme.
fn theme_card(ui: &mut egui::Ui, p: &theme::Palette, selected: bool) -> egui::Response {
    let size = egui::vec2(122.0, 96.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    let preview = egui::Rect::from_min_size(rect.min, egui::vec2(size.x, 70.0));
    painter.rect_filled(preview, 6.0, p.bg);
    // a sidebar strip, two text lines, a diff line pair and an accent dot
    let side = egui::Rect::from_min_size(preview.min, egui::vec2(30.0, preview.height()));
    painter.rect_filled(side, egui::CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, p.bg_dark);
    let line = |y: f32, w: f32, _c: egui::Color32| egui::Rect::from_min_size(preview.min + egui::vec2(38.0, y), egui::vec2(w, 4.0));
    painter.rect_filled(line(10.0, 60.0, p.text), 2.0, p.text);
    painter.rect_filled(line(19.0, 42.0, p.text_dim), 2.0, p.text_dim);
    let diff = |y: f32, _bg: egui::Color32| egui::Rect::from_min_size(preview.min + egui::vec2(34.0, y), egui::vec2(size.x - 40.0, 9.0));
    painter.rect_filled(diff(32.0, p.removed_bg), 1.0, p.removed_bg);
    painter.rect_filled(diff(43.0, p.added_bg), 1.0, p.added_bg);
    painter.rect_filled(line(34.5, 34.0, p.removed), 2.0, p.removed);
    painter.rect_filled(line(45.5, 48.0, p.added), 2.0, p.added);
    for (i, k) in [0u16, 1, 2].into_iter().enumerate() {
        painter.circle_filled(side.left_top() + egui::vec2(15.0, 14.0 + i as f32 * 14.0), 3.5, p.lanes[k as usize]);
    }
    painter.circle_filled(preview.right_bottom() + egui::vec2(-10.0, -9.0), 4.0, p.accent);
    let border = if selected { egui::Stroke::new(2.0, theme::ACCENT()) } else if resp.hovered() { egui::Stroke::new(1.0, theme::TEXT_DIM()) } else { egui::Stroke::new(1.0, theme::HOVER()) };
    painter.rect_stroke(preview, 6.0, border, egui::StrokeKind::Outside);
    let name = if selected { egui::RichText::new(p.name).strong() } else { egui::RichText::new(p.name) };
    let galley = painter.layout_no_wrap(name.text().to_string(), egui::TextStyle::Small.resolve(ui.style()), if selected { theme::TEXT() } else { theme::TEXT_DIM() });
    painter.galley(egui::pos2(rect.left() + 2.0, preview.bottom() + 7.0), galley, theme::TEXT());
    resp
}

pub enum Dialog {
    Confirm { msg: String, action: Action },
    NewBranch { name: String, start: Option<String>, switch: bool },
    LinkTask { branch: String, key: String },
    RenameBranch { old: String, new: String },
    Reattach { from: NoteTarget, to_commit: bool, value: String },
}

pub struct RepoView {
    pub repo: Repo,
    pub notes: NoteStore,
    pub snap: Option<Box<Snapshot>>,
    pub graph: Vec<GraphRow>,
    pub row_of: HashMap<String, usize>,
    pub sel: Sel,
    pub work_diff: DiffView,
    pub commit_detail: Option<CommitDetail>,
    /// Branches containing the selected commit (commit details).
    pub commit_branches: Vec<String>,
    pub commit_diff: DiffView,
    pub generation: u64,
    pub commit_msg: String,
    /// Focus the commit message box on the next frame (Repository → Commit…).
    pub focus_commit_msg: bool,
    pub amend: bool,
    pub filter: String,
    /// Search autocomplete: highlighted entry, and dropdown closed with Esc.
    pub search_sel: usize,
    pub search_hidden: bool,
    pub filtered: Option<(String, Vec<usize>)>,
    /// Existing notes → first line (title).
    pub note_index: HashMap<NoteTarget, String>,
    /// Commit id → local branches pointing at it (current branch first).
    pub tips: HashMap<String, Vec<String>>,
    /// Task keys shown on each history row: a key appears only on its latest
    /// (topmost) commit, so a ticket spanning many commits shows once.
    pub row_tasks: HashMap<String, Vec<String>>,
    /// The same for the commits matching the history search.
    pub filtered_tasks: HashMap<String, Vec<String>>,
    pub github: Option<Arc<GitHub>>,
    pub github_error: Option<String>,
    /// Local branch → its latest PR.
    pub prs: HashMap<String, PrSummary>,
    pub pr_detail: HashMap<String, Result<PrDetail, String>>,
    pub pr_requested: std::collections::HashSet<String>,
    pub prs_fetched_at: Option<f64>,
    /// Notes whose commit/branch is gone (see margin_core::detached).
    pub detached: Vec<margin_core::detached::DetachedNote>,
    /// Signature of refs + notes at the last detached scan.
    pub detached_sig: u64,
    /// Note opened explicitly (detached notes), shown in its own tab.
    pub pinned_note: Option<NoteTarget>,
    /// Automatic re-attachments that can still be undone (newest first).
    pub reattached: Vec<margin_core::journal::ReattachEntry>,
    pub notes_stamp: u64,
    /// Set by the watcher thread when the notes dir changes.
    pub notes_watch: Arc<AtomicBool>,
    pub editor: NoteEditor,
    pub focus_branch: Option<String>,
    pub focus_task: Option<String>,
    pub scroll_to_row: Option<usize>,
    /// History viewport (scroll offset, height) from the last frame.
    pub list_view: (f32, f32),
    pub loading: bool,
    pub limit: usize,
    /// Selection history for the toolbar's back/forward (like a browser).
    pub history: Vec<Sel>,
    /// Branches hidden from the history graph (per repository, view.json).
    pub graph_filter: margin_core::history::HistoryFilter,
    pub hist_pos: usize,
    /// Set while navigating back/forward so the move isn't recorded again.
    pub navigating: bool,
    pub opened_at: std::time::Instant,
    /// Last successful fetch (manual, pull, or automatic).
    pub last_fetch: Option<std::time::Instant>,
    pub auto_fetching: bool,
    /// Background work finished for this tab while it was hidden: refresh on activation.
    pub stale: bool,
    /// Loaded once (GitHub connection, notes watcher) on first activation.
    pub activated: bool,
    /// GitHub connection attempted (reset when Pro becomes active).
    pub github_checked: bool,
    /// `origin` looks like a GitHub remote (computed once; for the Pro hint).
    pub looks_github: bool,
    /// Identity of this repo across computers (notes sync).
    pub repo_id: String,
    /// Notes sync state for this repo (set by MarginApp::apply_sync).
    pub sync_state: margin_core::sync::RepoSync,
    /// Shared with the watcher thread, which does the mirroring.
    pub sync: Arc<std::sync::Mutex<SyncShared>>,
}

/// Notes-sync state shared between the UI and a tab's watcher thread.
#[derive(Default)]
pub struct SyncShared {
    /// Cloud-side folder to mirror with, when sync is active.
    pub remote: Option<PathBuf>,
    /// Last pass: when and what happened.
    pub last: Option<(std::time::Instant, Result<margin_core::sync::Report, String>)>,
    /// Run a pass on the next tick even if nothing changed.
    pub force: bool,
}

impl RepoView {
    fn new(repo: Repo, notes: NoteStore) -> RepoView {
        let looks_github = repo.remote_url("origin").is_some_and(|u| u.to_lowercase().contains("github"));
        let repo_id = margin_core::sync::repo_id(&repo);
        let graph_filter = margin_core::history::load(&repo);
        let mut rv = RepoView {
            graph_filter,
            repo,
            notes,
            snap: None,
            graph: vec![],
            row_of: HashMap::new(),
            sel: Sel::Working,
            work_diff: DiffView::default(),
            commit_detail: None,
            commit_branches: vec![],
            commit_diff: DiffView::default(),
            generation: 0,
            commit_msg: String::new(),
            focus_commit_msg: false,
            amend: false,
            filter: String::new(),
            search_sel: 0,
            search_hidden: false,
            filtered: None,
            note_index: HashMap::new(),
            tips: HashMap::new(),
            row_tasks: HashMap::new(),
            filtered_tasks: HashMap::new(),
            github: None,
            github_error: None,
            prs: HashMap::new(),
            pr_detail: HashMap::new(),
            pr_requested: Default::default(),
            prs_fetched_at: None,
            detached: vec![],
            detached_sig: 0,
            pinned_note: None,
            reattached: vec![],
            notes_stamp: 0,
            notes_watch: Arc::new(AtomicBool::new(false)),
            editor: NoteEditor::default(),
            focus_branch: None,
            focus_task: None,
            scroll_to_row: None,
            list_view: (0.0, 0.0),
            loading: true,
            limit: PAGE,
            history: vec![Sel::Working],
            hist_pos: 0,
            navigating: false,
            opened_at: std::time::Instant::now(),
            last_fetch: None,
            auto_fetching: false,
            stale: true,
            activated: false,
            github_checked: false,
            looks_github: false,
            repo_id: String::new(),
            sync_state: margin_core::sync::RepoSync::Off,
            sync: Default::default(),
        };
        rv.looks_github = looks_github;
        rv.repo_id = repo_id;
        rv
    }

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

/// Recently opened directories kept (File → Open Recent, the + menu).
pub const RECENT_REPOS: usize = 10;

/// `/Users/me/src` → `~/src` for display.
pub fn tilde(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

pub struct MarginApp {
    pub cfg: Config,
    pub state: State,
    pub settings: margin_core::config::Settings,
    /// Margin Pro status (licensed / trial / expired); recomputed on focus.
    pub license: margin_core::license::Status,
    pub license_input: String,
    pub license_error: Option<String>,
    /// The free notes-sync limit was hit (shows the Pro hint in Settings).
    pub sync_limit_hit: bool,
    /// Typed sync folder path (non-macOS; macOS uses the folder picker).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub sync_folder_input: String,
    pub show_settings: bool,
    pub settings_tab: SettingsTab,
    pub jobs: Jobs,
    /// The active tab. Other open repositories live in `others`; the visual
    /// order is others[..active_pos] + rv + others[active_pos..].
    pub rv: Option<RepoView>,
    pub others: Vec<RepoView>,
    pub active_pos: usize,
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
    /// Journal ids the toolbar "Undo" button reverts (last auto re-attach).
    pub status_undo: Vec<String>,
    pub palette: Option<crate::palette::Palette>,
    /// Focus the commit search box on the next frame (Edit → Find Commits).
    pub focus_search: bool,
    /// Diff focus mode: the details/diff column takes the whole window.
    pub focus_diff: bool,
    menu: crate::menu_bar::MenuBar,
    ctx: egui::Context,
}

impl MarginApp {
    pub fn new(ctx: &egui::Context, repo: Option<PathBuf>) -> Self {
        let cfg = config::load_config();
        let settings = config::load_settings();
        theme::set(settings.theme.as_deref());
        theme::apply(
            ctx,
            settings.ui_font_size.unwrap_or(DEFAULT_FONT).clamp(MIN_UI_FONT, MAX_UI_FONT),
            settings.font_size.or(cfg.font_size).unwrap_or(DEFAULT_FONT).clamp(MIN_FONT, MAX_FONT),
        );
        if settings.ui_font.is_some() || settings.code_font.is_some() {
            crate::fonts::apply(ctx, settings.ui_font.as_ref(), settings.code_font.as_ref());
        }
        let tasks = tasks::provider_from_config(&cfg).map(Arc::from);
        let mut app = MarginApp {
            author: cfg.note_author.clone().unwrap_or_default(),
            state: config::load_state(),
            cfg: cfg.clone(),
            settings,
            settings_tab: SettingsTab::Appearance,
            license: margin_core::license::current(now_unix()),
            license_input: String::new(),
            license_error: None,
            sync_limit_hit: false,
            sync_folder_input: String::new(),
            show_settings: false,
            jobs: Jobs::new(ctx),
            rv: None,
            others: vec![],
            active_pos: 0,
            tasks,
            task_cache: HashMap::new(),
            my_tasks: None,
            status_line: None,
            busy: None,
            dialog: None,
            open_path: String::new(),
            was_focused: true,
            pending_copy: None,
            status_undo: vec![],
            palette: None,
            focus_search: false,
            focus_diff: false,
            menu: crate::menu_bar::MenuBar::new(ctx),
            ctx: ctx.clone(),
        };
        // Restore open tabs without loading them; only the active one loads.
        let saved = app.state.open_tabs.clone();
        let want = app.state.active_tab;
        for p in &saved {
            if let Ok(r) = Repo::open(p) {
                if app.tab_index_of(&r.workdir).is_none() {
                    if let Ok(n) = NoteStore::for_repo(&r) {
                        app.others.push(RepoView::new(r, n));
                    }
                }
            }
        }
        if !app.others.is_empty() {
            let i = want.min(app.others.len() - 1);
            app.rv = Some(app.others.remove(i));
            app.active_pos = i;
        }
        if let Some(p) = repo.filter(|p| Repo::open(p).is_ok()) {
            app.handle(Action::OpenRepo(p)); // switches if it's already a tab
        } else {
            app.activate();
        }
        app
    }

    fn err(&mut self, e: impl ToString) {
        self.status_undo.clear();
        self.status_line = Some((e.to_string(), true));
    }

    fn info(&mut self, s: impl ToString) {
        self.status_undo.clear();
        self.status_line = Some((s.to_string(), false));
    }

    pub fn refresh(&mut self) {
        let Some(rv) = &mut self.rv else { return };
        rv.loading = true;
        let repo = rv.repo.clone();
        let limit = rv.limit;
        let filter = rv.graph_filter.clone();
        self.jobs.spawn(move || {
            // status (slow in monorepos) runs while refs and the log load;
            // the log needs the refs to know which branches are shown.
            let r = std::thread::scope(|s| {
                let st = s.spawn(|| repo.status());
                let refs = repo.refs();
                let log = refs.as_ref().map_err(|e| e.clone()).and_then(|refs| margin_core::history::log(&repo, &filter, refs, limit, 0));
                let status = st.join().unwrap()?;
                let refs = refs?;
                let commits = log?;
                Ok::<_, margin_core::Error>(Box::new(Snapshot {
                    status,
                    commits,
                    refs,
                    stashes: repo.stashes().unwrap_or_default(),
                    pending_op: repo.pending_operation(),
                    limit,
                    worktrees: {
                        let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
                        let here = canon(&repo.workdir);
                        repo.worktrees()
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|w| !w.bare)
                            .map(|w| crate::jobs::WorktreeInfo { current: canon(&w.path) == here, wt: w })
                            .collect()
                    },
                }))
            });
            Msg::Refreshed { workdir: repo.workdir.clone(), result: r.map_err(|e| e.0) }
        });
    }

    fn load_work_diff(&mut self) {
        // Whole working directory: unstaged + untracked, staged, conflicts.
        let Some(rv) = &mut self.rv else { return };
        let Some(snap) = rv.snap.as_ref() else { return };
        let files = snap.status.files.clone();
        rv.generation += 1;
        rv.work_diff.loading = true;
        let generation = rv.generation;
        let repo = rv.repo.clone();
        let ignore_ws = self.state.diff_ignore_ws;
        self.jobs.spawn(move || {
            let r = (|| {
                let conflicted: Vec<String> = files.iter().filter(|f| f.conflicted).map(|f| f.path.clone()).collect();
                let (mut unstaged, t1) = diff::parse(&repo.worktree_diff(false, ignore_ws, &conflicted)?);
                // Untracked files (first 200; each read like `git diff --no-index`).
                let mut t3 = false;
                for (n, f) in files.iter().filter(|f| f.is_untracked()).enumerate() {
                    if n >= 200 {
                        t3 = true;
                        break;
                    }
                    if let Ok(t) = repo.file_diff_opts(f, false, false) {
                        unstaged.extend(diff::parse(&t).0);
                    }
                }
                unstaged.sort_by(|a, b| a.display_path().cmp(b.display_path()));
                let (staged, t2) = diff::parse(&repo.worktree_diff(true, ignore_ws, &conflicted)?);
                Ok::<_, margin_core::Error>(crate::diff_view::WorkDiff { unstaged, staged, conflicted, truncated: t1 || t2 || t3 })
            })();
            Msg::WorkDiff { workdir: repo.workdir.clone(), generation, result: r.map_err(|e| e.0) }
        });
    }

    fn load_commit(&mut self, id: String) {
        let Some(rv) = &mut self.rv else { return };
        rv.generation += 1;
        // Keep showing the previous commit until this one arrives (no blank
        // flash while moving through the history); the toolbar shows a spinner.
        rv.commit_diff.loading = true;
        let generation = rv.generation;
        let repo = rv.repo.clone();
        let ignore_ws = self.state.diff_ignore_ws;
        self.jobs.spawn(move || {
            let r = (|| {
                let d = repo.commit_detail(&id)?;
                let (files, trunc) = diff::parse(&repo.commit_diff_opts(&id, ignore_ws)?);
                let branches = repo.branches_containing(&id).unwrap_or_default();
                Ok::<_, margin_core::Error>((d, files, trunc, branches))
            })();
            Msg::CommitLoaded { workdir: repo.workdir.clone(), generation, result: r.map_err(|e| e.0) }
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
        self.jobs.spawn(move || Msg::OpDone { workdir: repo.workdir.clone(), result: f(&repo).map_err(|e| e.0), label, refresh: true });
    }

    /// Load PR details for the branches containing the selected commit (their
    /// review threads may sit on it).
    fn request_pr_threads(&mut self) {
        if !self.pro() {
            return;
        }
        let Some(rv) = &self.rv else { return };
        if !matches!(rv.sel, Sel::Commit(_)) {
            return;
        }
        let want: Vec<String> = rv.commit_branches.iter().filter(|b| rv.prs.contains_key(*b) && !rv.pr_requested.contains(*b)).cloned().collect();
        for b in want {
            self.handle(Action::FetchPrDetail(b));
        }
    }

    /// Review threads to show in the selected commit's diff: threads written
    /// on this commit (at their original line), and, on the PR's latest
    /// commit, the open threads at their current line.
    fn update_threads(&mut self) {
        let Some(rv) = &mut self.rv else { return };
        let Sel::Commit(id) = rv.sel.clone() else { return };
        let mut out: Vec<crate::diff_view::InlineThread> = vec![];
        for b in &rv.commit_branches {
            let Some(Ok(d)) = rv.pr_detail.get(b) else { continue };
            let head = d.summary.head_oid.as_deref() == Some(id.as_str());
            for t in &d.threads {
                let line = if t.original_commit == id {
                    t.original_line
                } else if head && !t.outdated {
                    t.line
                } else {
                    None
                };
                let Some(line) = line else { continue };
                let it = crate::diff_view::InlineThread {
                    path: t.path.clone(),
                    line,
                    old_side: t.old_side,
                    resolved: t.resolved,
                    outdated: t.outdated && t.original_commit != id,
                    id: t.id.clone(),
                    branch: b.clone(),
                    pr: d.summary.number,
                    comments: t.comments.iter().map(|c| (c.author.clone(), crate::emoji::render(&c.body).into_owned(), c.created_at.chars().take(10).collect(), c.url.clone())).collect(),
                };
                if !out.contains(&it) {
                    out.push(it);
                }
            }
        }
        rv.commit_diff.set_threads(out);
        // New comments go to the (first) open PR containing this commit.
        let target = rv.commit_branches.iter().find_map(|b| match rv.pr_detail.get(b) {
            Some(Ok(d)) if d.summary.state == "OPEN" && !d.id.is_empty() => Some((b.clone(), d.summary.number)),
            _ => None,
        });
        rv.commit_diff.set_comment_target(target);
    }

    /// Run a review-thread mutation in the background (see `Msg::ThreadOp`).
    fn thread_op(&mut self, id: String, branch: String, what: &'static str, f: impl FnOnce(&margin_core::github::GitHub, &str) -> margin_core::Result<()> + Send + 'static) {
        if !self.pro() {
            return;
        }
        let Some(rv) = &mut self.rv else { return };
        let Some(gh) = rv.github.clone() else {
            rv.commit_diff.thread_done(&id, false);
            self.err("GitHub isn't connected");
            return;
        };
        let workdir = rv.repo.workdir.clone();
        self.jobs.spawn(move || {
            let result = f(&gh, &id).map_err(|e| e.0);
            Msg::ThreadOp { workdir, branch, thread: id, what, result }
        });
    }

    fn save_note(&mut self) {
        if let Some(rv) = &mut self.rv {
            rv.editor.save(&rv.notes);
            self.reload_notes_index();
        }
    }

    /// A menu bar command (see `menu_bar`).
    fn menu_cmd(&mut self, cmd: crate::menu_bar::Cmd, ctx: &egui::Context) {
        use crate::menu_bar::Cmd;
        // The menu takes ⌘C/⌘V/⌘Z… before egui sees the key, so hand the
        // focused widget the event it would have got.
        let key = |key: egui::Key, shift: bool| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { shift, ..egui::Modifiers::COMMAND },
        };
        let inject = |e: egui::Event| ctx.input_mut(|i| i.events.push(e));
        let workdir = self.rv.as_ref().map(|rv| rv.repo.workdir.clone());
        let a = match cmd {
            Cmd::Settings => Action::OpenSettings,
            Cmd::OpenFolder => Action::OpenFolderDialog,
            Cmd::OpenRecent(p) => Action::OpenRepo(PathBuf::from(p)),
            Cmd::SaveNote => return self.save_note(),
            Cmd::CloseTab => Action::CloseRepo,
            Cmd::Reveal | Cmd::Terminal => match workdir {
                Some(path) => Action::OpenPath { path, terminal: cmd == Cmd::Terminal },
                None => return,
            },
            Cmd::Undo => return inject(key(egui::Key::Z, false)),
            Cmd::Redo => return inject(key(egui::Key::Z, true)),
            Cmd::SelectAll => return inject(key(egui::Key::A, false)),
            Cmd::Cut => return inject(egui::Event::Cut),
            Cmd::Copy => return inject(egui::Event::Copy),
            Cmd::Paste => {
                #[cfg(target_os = "macos")]
                if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    inject(egui::Event::Paste(text));
                }
                return;
            }
            Cmd::FindCommits => {
                self.focus_search = true;
                return;
            }
            Cmd::Fetch => Action::Fetch,
            Cmd::Pull => Action::Pull,
            Cmd::Push => Action::Push { force: false },
            Cmd::Commit => {
                if let Some(rv) = &mut self.rv {
                    rv.focus_commit_msg = true;
                }
                Action::Select(Sel::Working)
            }
            Cmd::Stash => Action::StashPush,
            Cmd::NewBranch => Action::NewBranchDialog { start: None, name: String::new() },
            Cmd::Palette(key) => Action::RunPaletteCommand(key.to_string()),
            Cmd::Refresh => return self.refresh(),
            Cmd::CommandPalette => {
                if self.palette.take().is_none() {
                    self.handle(Action::OpenPalette);
                }
                return;
            }
            Cmd::Back => Action::NavBack,
            Cmd::Forward => Action::NavForward,
            Cmd::WorkingDirectory => Action::Select(Sel::Working),
            Cmd::FocusDiff => Action::ToggleFocusDiff,
            Cmd::NextTab => Action::NextTab,
            Cmd::PrevTab => Action::PrevTab,
            Cmd::Help => Action::OpenUrl(crate::pro::HELP_URL.into()),
            Cmd::ReportIssue => Action::OpenUrl(crate::pro::SUPPORT_URL.into()),
        };
        self.handle(a);
    }

    pub fn pro(&self) -> bool {
        self.license.is_pro()
    }

    /// Recompute the licence status (startup, focus, key changes). Turning
    /// Pro off drops GitHub data; turning it on reconnects.
    fn update_license(&mut self) {
        let was = self.pro();
        self.license = margin_core::license::current(now_unix());
        let now = self.pro();
        if was == now {
            return;
        }
        self.apply_sync(); // extra synced repos pause / resume with Pro
        for t in self.rv.iter_mut().chain(self.others.iter_mut()) {
            t.github = None;
            t.prs.clear();
            t.pr_detail.clear();
            t.pr_requested.clear();
            t.github_checked = false;
        }
        if !now {
            self.my_tasks = None;
            self.task_cache.clear();
        } else {
            self.connect_github();
        }
    }

    /// Point every open tab's watcher at its cloud folder (or stop syncing).
    fn apply_sync(&mut self) {
        let pro = self.pro();
        for t in self.rv.iter_mut().chain(self.others.iter_mut()) {
            let st = margin_core::sync::status_for(&t.repo_id, &self.settings, pro);
            let remote = match &st {
                margin_core::sync::RepoSync::Active { remote } => Some(remote.clone()),
                _ => None,
            };
            let mut g = t.sync.lock().unwrap();
            if g.remote != remote {
                g.force = remote.is_some();
                g.remote = remote;
            }
            drop(g);
            t.sync_state = st;
        }
    }

    pub fn tab_count(&self) -> usize {
        self.others.len() + self.rv.is_some() as usize
    }

    /// Open tabs in visual order: (workdir, name, active).
    pub fn tabs(&self) -> Vec<(PathBuf, String, bool)> {
        let mut v: Vec<(PathBuf, String, bool)> = self.others.iter().map(|t| (t.repo.workdir.clone(), t.repo.name(), false)).collect();
        if let Some(rv) = &self.rv {
            v.insert(self.active_pos.min(v.len()), (rv.repo.workdir.clone(), rv.repo.name(), true));
        }
        v
    }

    /// Recent directories that aren't open as a tab (newest first, ≤ 10).
    pub fn recent_closed(&self, tabs: &[(PathBuf, String, bool)]) -> Vec<String> {
        let open: Vec<String> = tabs.iter().map(|(p, _, _)| p.display().to_string()).collect();
        self.state.recent_repos.iter().filter(|r| !open.contains(r)).take(RECENT_REPOS).cloned().collect()
    }

    fn tab_index_of(&self, workdir: &std::path::Path) -> Option<usize> {
        self.tabs().iter().position(|(p, _, _)| p == workdir)
    }

    fn save_tabs(&mut self) {
        self.state.open_tabs = self.tabs().into_iter().map(|(p, _, _)| p.display().to_string()).collect();
        self.state.active_tab = self.active_pos;
        let _ = config::save_state(&self.state);
    }

    /// Make the active tab live: first-time setup (notes watcher, GitHub),
    /// then refresh if it was never loaded or changed while hidden.
    fn activate(&mut self) {
        let Some(rv) = &mut self.rv else { return };
        let first = !rv.activated;
        rv.activated = true;
        let stale = std::mem::replace(&mut rv.stale, false);
        if first {
            self.start_notes_watcher();
        }
        self.reload_notes_index();
        self.connect_github(); // no-op without Pro or if already done
        self.apply_sync();
        if first || stale {
            self.refresh();
        }
        self.save_tabs();
    }

    /// A result for a tab that isn't visible: update that tab only.
    fn background_msg(&mut self, wd: PathBuf, m: Msg) {
        let idx = self.others.iter().position(|t| t.repo.workdir == wd);
        if let Msg::OpDone { label, result, .. } = &m {
            // Busy is app-wide (one git operation at a time).
            self.busy = None;
            let name = wd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match result {
                Ok(_) => self.info(format!("{name}: {label} done")),
                Err(e) => self.err(format!("{name}: {label} failed: {e}")),
            }
        }
        let tab = idx.map(|i| &mut self.others[i]);
        match m {
            Msg::OpDone { .. } => {
                if let Some(t) = tab {
                    t.stale = true;
                }
            }
            Msg::GithubReady { result, .. } => {
                if let Some(t) = tab {
                    match result {
                        Ok(gh) => t.github = gh,
                        Err(e) => t.github_error = Some(e),
                    }
                }
            }
            Msg::AutoFetched { .. } => {
                if let Some(t) = tab {
                    t.auto_fetching = false;
                    t.last_fetch = Some(std::time::Instant::now());
                    t.stale = true;
                }
            }
            _ => {
                if let Some(t) = tab {
                    t.loading = false;
                    t.stale = true;
                }
            }
        }
    }

    /// Poll the notes dir off the UI thread (1 Hz, a few stat calls) and wake
    /// the UI only when something changed — zero repaints while idle.
    /// The thread exits when the repo is closed (it holds the last Arc).
    fn start_notes_watcher(&mut self) {
        let Some(rv) = &self.rv else { return };
        let flag = rv.notes_watch.clone();
        let notes = rv.notes.clone();
        let shared = rv.sync.clone();
        let ctx = self.ctx.clone();
        let mut last = rv.notes_stamp;
        let mut last_remote = 0u64;
        let mut synced_last = last.wrapping_add(1); // force a first pass when sync is on
        std::thread::spawn(move || {
            while Arc::strong_count(&flag) > 1 {
                std::thread::sleep(std::time::Duration::from_secs(1));
                // Notes sync: mirror when either side changed (or on request).
                let (remote, force) = {
                    let mut g = shared.lock().unwrap();
                    (g.remote.clone(), std::mem::take(&mut g.force))
                };
                if let Some(remote) = remote {
                    let local_now = notes.stamp();
                    let remote_now = margin_core::sync::dir_stamp(&remote);
                    if force || local_now != synced_last || remote_now != last_remote {
                        let res = margin_core::sync::mirror(notes.root(), &remote).map_err(|e| e.0);
                        let notify = force || res.as_ref().map(|r| !r.is_empty()).unwrap_or(true);
                        shared.lock().unwrap().last = Some((std::time::Instant::now(), res));
                        if notify {
                            ctx.request_repaint();
                        }
                        synced_last = notes.stamp();
                        last_remote = margin_core::sync::dir_stamp(&remote);
                    }
                }
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
            rv.reattached = margin_core::journal::active(&rv.notes);
            rv.notes_stamp = rv.notes.stamp();
        }
    }

    pub fn handle(&mut self, a: Action) {
        use Action::*;
        match a {
            OpenRepo(p) => match Repo::open(&p) {
                Ok(repo) => {
                    // Already open in a tab? Just switch to it.
                    if let Some(i) = self.tab_index_of(&repo.workdir) {
                        // The active tab itself (e.g. `margin .` on the tab restored
                        // as active at startup): make sure it was set up.
                        if i == self.active_pos {
                            if self.rv.as_ref().is_some_and(|rv| !rv.activated) {
                                self.activate();
                            }
                            return;
                        }
                        return self.handle(SwitchTab(i));
                    }
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
                    self.state.recent_repos.truncate(RECENT_REPOS);
                    // New tab right after the current one.
                    if let Some(cur) = self.rv.take() {
                        self.others.insert(self.active_pos, cur);
                        self.active_pos += 1;
                    }
                    self.rv = Some(RepoView::new(repo, notes));
                    self.activate();
                }
                Err(e) => self.err(format!("{}: {e}", p.display())),
            },
            SwitchTab(i) => {
                if i == self.active_pos || i >= self.tab_count() {
                    return;
                }
                if let Some(mut cur) = self.rv.take() {
                    cur.editor.save_if_dirty(&cur.notes);
                    self.others.insert(self.active_pos, cur);
                }
                self.rv = Some(self.others.remove(i));
                self.active_pos = i;
                self.status_line = None;
                self.status_undo.clear();
                self.palette = None;
                self.activate();
            }
            NextTab | PrevTab => {
                let n = self.tab_count();
                if n > 1 {
                    let i = if matches!(a, NextTab) { (self.active_pos + 1) % n } else { (self.active_pos + n - 1) % n };
                    self.handle(SwitchTab(i));
                }
            }
            CloseTab(i) => {
                if i == self.active_pos {
                    return self.handle(CloseRepo);
                }
                // others index: tabs before the active one keep their index, later ones shift by one
                let oi = if i < self.active_pos { i } else { i - 1 };
                if oi < self.others.len() {
                    let mut t = self.others.remove(oi);
                    t.editor.save_if_dirty(&t.notes);
                    if i < self.active_pos {
                        self.active_pos -= 1;
                    }
                    self.save_tabs();
                }
            }
            CloseRepo => {
                // Close the active tab and activate its neighbour (if any).
                if let Some(rv) = &mut self.rv {
                    rv.editor.save_if_dirty(&rv.notes);
                }
                self.rv = None;
                if !self.others.is_empty() {
                    let i = self.active_pos.min(self.others.len() - 1);
                    self.rv = Some(self.others.remove(i));
                    self.active_pos = i;
                    self.activate();
                } else {
                    self.active_pos = 0;
                    self.save_tabs();
                }
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
                if !rv.navigating {
                    // New selection: drop any "forward" entries, then record it.
                    rv.history.truncate(rv.hist_pos + 1);
                    rv.history.push(sel.clone());
                    if rv.history.len() > 200 {
                        rv.history.remove(0);
                    }
                    rv.hist_pos = rv.history.len() - 1;
                }
                rv.focus_task = None;
                rv.focus_branch = None;
                match sel {
                    Sel::Working => {
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
            Fetch => {
                if self.rv.as_ref().is_some_and(|rv| rv.auto_fetching) {
                    return self.info("Already fetching in the background…");
                }
                self.op("Fetch", |r| r.fetch())
            }
            OpenSettings => self.show_settings = true,
            ToggleAutoFetch => {
                self.settings.auto_fetch = !self.settings.auto_fetch;
                if let Err(e) = config::save_settings(&self.settings) {
                    self.err(e);
                }
                let m = self.settings.auto_fetch_minutes;
                self.info(if self.settings.auto_fetch { format!("Auto-fetch on: every {m} min") } else { "Auto-fetch off".into() });
            }
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
                // The task opens in the notes panel: make sure it's visible.
                if self.state.notes_collapsed {
                    self.state.notes_collapsed = false;
                    let _ = config::save_state(&self.state);
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
                if !self.pro() {
                    return;
                }
                let Some(p) = self.tasks.clone() else { return };
                self.jobs.spawn(move || Msg::Task { result: p.get(&key).map_err(|e| e.0), key });
            }
            FetchPrs { force } => {
                if !self.pro() {
                    return;
                }
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
                if !self.pro() {
                    return;
                }
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
            ReplyThread { id, branch, body } => self.thread_op(id, branch, "Reply", move |gh, id| gh.reply_thread(id, &body)),
            ResolveThread { id, branch, resolve } => {
                self.thread_op(id, branch, if resolve { "Resolve" } else { "Unresolve" }, move |gh, id| gh.set_thread_resolved(id, resolve))
            }
            NewThread { branch, path, line, old_side, body } => {
                let Some(rv) = &self.rv else { return };
                let Sel::Commit(commit) = rv.sel.clone() else { return };
                let pr_id = rv.pr_detail.get(&branch).and_then(|d| d.as_ref().ok()).map(|d| d.id.clone()).unwrap_or_default();
                self.thread_op("draft".into(), branch, "Comment", move |gh, _| gh.new_thread(&pr_id, &commit, &path, line, old_side, &body));
            }
            ScanDetached => {
                let Some(rv) = &mut self.rv else { return };
                let Some(snap) = &rv.snap else { return };
                // Only rescan when refs or the set of notes changed.
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for r in &snap.refs {
                    std::hash::Hash::hash(&(&r.name, &r.target), &mut h);
                }
                std::hash::Hash::hash(&rv.note_index.len(), &mut h);
                let sig = std::hash::Hasher::finish(&h);
                if sig == rv.detached_sig {
                    return;
                }
                rv.detached_sig = sig;
                let (repo, notes) = (rv.repo.clone(), rv.notes.clone());
                // Everything in the loaded log is reachable from a ref: skip those.
                let known: std::collections::HashSet<String> = snap.commits.iter().map(|c| c.id.clone()).collect();
                let workdir = repo.workdir.clone();
                let auto = self.cfg.auto_reattach.unwrap_or(true);
                self.jobs.spawn(move || {
                    let renamed = margin_core::detached::follow_renames(&repo, &notes).unwrap_or_default();
                    let mut reattached = vec![];
                    let result = margin_core::detached::find(&repo, &notes, &known).and_then(|mut d| {
                        if auto {
                            reattached = margin_core::detached::auto_reattach(&repo, &notes, &mut d)?;
                        }
                        Ok(d)
                    });
                    Msg::Detached { workdir, renamed, reattached, result: result.map_err(|e| e.0) }
                });
            }
            RenameBranchDialog(old) => self.dialog = Some(Dialog::RenameBranch { new: old.clone(), old }),
            RenameBranch { old, new } => {
                let Some(rv) = &self.rv else { return };
                let notes = rv.notes.clone();
                self.op("Rename branch", move |r| {
                    r.rename_branch(&old, &new)?;
                    // Carry the branch note and task link to the new name.
                    notes.rename_branch(&old, &new)?;
                    Ok(format!("{old} → {new}"))
                });
            }
            OpenNote(t) => {
                if let Some(rv) = &mut self.rv {
                    rv.pinned_note = Some(t);
                    rv.editor.tab = crate::notes_panel::NoteTab::Pinned;
                }
            }
            ReattachDialog(from) => {
                let Some(rv) = &self.rv else { return };
                let (to_commit, value) = match &from {
                    NoteTarget::Commit(_) => {
                        // Suggest the rewritten commit: a patch-id match first,
                        // else the same subject on a current branch.
                        let d = rv.detached.iter().find(|d| d.note == from);
                        let by_patch = d.and_then(|d| d.candidates.first()).map(|c| c[..10.min(c.len())].to_string());
                        let subject = d.and_then(|d| d.subject.clone());
                        let guess = by_patch.or_else(|| {
                            subject.and_then(|s| rv.snap.as_ref()?.commits.iter().find(|c| c.subject == s).map(|c| c.short_id().to_string()))
                        });
                        (true, guess.unwrap_or_default())
                    }
                    _ => (false, rv.current_branch().unwrap_or_default().to_string()),
                };
                self.dialog = Some(Dialog::Reattach { from, to_commit, value });
            }
            MoveNote { from, to } => {
                let Some(rv) = &mut self.rv else { return };
                let target = match to.split_once(':') {
                    Some(("commit", rev)) => rv.repo.resolve(rev.trim()).map(NoteTarget::Commit),
                    Some(("branch", b)) if !b.trim().is_empty() => Ok(NoteTarget::Branch(b.trim().to_string())),
                    _ => Err(margin_core::Error(format!("bad target '{to}'"))),
                };
                match target.and_then(|t| rv.notes.move_note(&from, &t).map(|_| t)) {
                    Ok(t) => {
                        if rv.pinned_note.as_ref() == Some(&from) {
                            rv.pinned_note = Some(t.clone());
                        }
                        rv.detached_sig = 0; // force rescan
                        self.info(format!("Moved note {from} → {t}"));
                        self.reload_notes_index();
                        self.handle(Action::ScanDetached);
                    }
                    Err(e) => self.err(e),
                }
            }
            DeleteNote(t) => {
                let Some(rv) = &mut self.rv else { return };
                match rv.notes.delete(&t) {
                    Ok(()) => {
                        if rv.editor.target.as_ref() == Some(&t) {
                            rv.editor.forget();
                        }
                        if rv.pinned_note.as_ref() == Some(&t) {
                            rv.pinned_note = None;
                            rv.editor.tab = crate::notes_panel::NoteTab::Branch;
                        }
                        rv.detached_sig = 0;
                        self.info(format!("Deleted note {t}"));
                        self.reload_notes_index();
                        self.handle(Action::ScanDetached);
                    }
                    Err(e) => self.err(e),
                }
            }
            RunGit { label, f } => self.op(&label, move |r| f(r)),
            OpenPath { path, terminal } => open_path(&path, terminal, self),
            CommitWithMessage(msg) => {
                if let Some(rv) = &mut self.rv {
                    rv.commit_msg = msg;
                }
                self.handle(Commit);
            }
            StartAmend => {
                let Some(rv) = &mut self.rv else { return };
                rv.amend = true;
                rv.commit_msg = rv.repo.last_commit_message().unwrap_or_default();
                self.handle(Select(Sel::Working));
                self.info("Amending: edit the message (and stage changes), then Amend commit");
            }
            GotoRev(rev) => {
                let Some(rv) = &self.rv else { return };
                let is_local = rv.snap.as_ref().is_some_and(|s| s.refs.iter().any(|r| r.kind == margin_core::git::RefKind::Local && r.name == rev));
                match rv.repo.resolve(&rev) {
                    Ok(id) => {
                        self.handle(JumpTo(id));
                        if is_local {
                            self.handle(FocusBranch(rev));
                        }
                    }
                    Err(e) => self.err(e),
                }
            }
            ToggleFocusDiff => self.focus_diff = !self.focus_diff,
            ReloadDiff => {
                match self.rv.as_ref().map(|rv| rv.sel.clone()) {
                    Some(Sel::Commit(id)) => self.load_commit(id),
                    Some(Sel::Working) => self.load_work_diff(),
                    None => {}
                }
            }
            SetGraphFilter(f) => {
                let Some(rv) = &mut self.rv else { return };
                if let Err(e) = margin_core::history::save(&rv.repo, &f) {
                    return self.err(e);
                }
                rv.graph_filter = f;
                self.refresh();
            }
            ToggleNotesDock => {
                self.state.notes_under_history = !self.state.notes_under_history;
                let _ = config::save_state(&self.state);
            }
            OpenFolderDialog => {
                #[cfg(target_os = "macos")]
                if let Some(p) = rfd::FileDialog::new().set_title("Open a git repository").pick_folder() {
                    self.handle(Action::OpenRepo(p));
                }
                #[cfg(not(target_os = "macos"))]
                self.handle(Action::OpenPaletteWith("open repository".into()));
            }
            OpenPalette => self.palette = Some(crate::palette::Palette::new(crate::commands::build(self)).with_recent(&self.state.recent_commands)),
            RunPaletteCommand(key) => {
                let mut p = crate::palette::Palette::new(crate::commands::build(self)).with_recent(&self.state.recent_commands);
                match p.find(&key) {
                    Some(i) => match p.start(i) {
                        None => self.palette = Some(p),
                        Some(actions) => {
                            self.palette = None;
                            for a in actions {
                                self.handle(a);
                            }
                        }
                    },
                    // Not available here: show the matching commands instead.
                    None => self.palette = Some(p.with_query(key.rsplit('/').next().unwrap_or(&key).trim_end_matches('…').to_string())),
                }
            }
            OpenPaletteWith(q) => self.palette = Some(crate::palette::Palette::new(crate::commands::build(self)).with_query(q)),
            NavBack | NavForward => {
                let Some(rv) = &mut self.rv else { return };
                let fwd = matches!(a, NavForward);
                // Skip entries whose commit no longer exists (e.g. rebased away).
                let mut pos = rv.hist_pos;
                let target = loop {
                    pos = if fwd { pos + 1 } else { match pos.checked_sub(1) { Some(p) => p, None => break None } };
                    let Some(s) = rv.history.get(pos) else { break None };
                    match s {
                        Sel::Commit(id) if !rv.row_of.contains_key(id) && rv.repo.resolve(id).is_err() => continue,
                        _ => break Some(s.clone()),
                    }
                };
                if let Some(sel) = target {
                    rv.hist_pos = pos;
                    if let Sel::Commit(id) = &sel {
                        if let Some(&row) = rv.row_of.get(id) {
                            rv.scroll_to_row = Some(row + 1);
                        }
                    }
                    rv.navigating = true;
                    self.handle(Select(sel));
                    if let Some(rv) = &mut self.rv {
                        rv.navigating = false;
                    }
                }
            }
            UndoReattach(id) => {
                let Some(rv) = &mut self.rv else { return };
                match margin_core::journal::undo(&rv.notes, &id) {
                    Ok(out) => {
                        let back = crate::notes_panel::short_target(&NoteTarget::parse(&out.restored).unwrap_or(NoteTarget::Repo));
                        let mut msg = format!("Undone: the note is back on {back} (see Detached notes); it won't be re-attached there again");
                        if out.kept_edited_destination {
                            msg += ". The re-attached copy was edited afterwards, so it was left in place";
                        }
                        rv.detached_sig = 0; // rescan → the note shows up as detached again
                        rv.editor.on_external_change(&rv.notes);
                        self.info(msg);
                        self.reload_notes_index();
                        self.handle(Action::ScanDetached);
                    }
                    Err(e) => self.err(e),
                }
            }
            KeepReattach(id) => {
                let Some(rv) = &mut self.rv else { return };
                match margin_core::journal::keep(&rv.notes, &id) {
                    Ok(()) => self.reload_notes_index(),
                    Err(e) => self.err(e),
                }
            }
            FetchMyTasks => {
                if !self.pro() {
                    return;
                }
                let Some(p) = self.tasks.clone() else { return };
                self.jobs.spawn(move || Msg::MyTasks(p.my_open(30).map_err(|e| e.0)));
            }
            InstallLicense(key) => match margin_core::license::install(&key) {
                Ok(l) => {
                    self.license_input.clear();
                    self.license_error = None;
                    self.update_license();
                    self.info(format!("Margin Pro activated — thank you, {}!", l.name));
                }
                Err(e) => {
                    self.license_error = Some(e.0.clone());
                    self.err(format!("Licence key: {e}"));
                }
            },
            SetNotesSync(on) => {
                let pro = self.pro();
                let Some(rv) = &self.rv else { return };
                let id = rv.repo_id.clone();
                if on {
                    if !self.settings.synced_repos.contains(&id) {
                        if !margin_core::sync::can_enable(&self.settings, pro) {
                            self.sync_limit_hit = true;
                            return;
                        }
                        self.settings.synced_repos.push(id);
                    }
                } else {
                    self.settings.synced_repos.retain(|r| *r != id);
                }
                self.sync_limit_hit = false;
                if let Err(e) = config::save_settings(&self.settings) {
                    self.err(e);
                }
                self.apply_sync();
            }
            SetSyncFolder(f) => {
                self.settings.sync_folder = f;
                if let Err(e) = config::save_settings(&self.settings) {
                    self.err(e);
                }
                self.apply_sync();
            }
            SyncNow => {
                if let Some(rv) = &self.rv {
                    rv.sync.lock().unwrap().force = true;
                }
            }
            RemoveLicense => {
                if let Err(e) = margin_core::license::remove() {
                    self.err(e);
                }
                self.update_license();
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
        let pro = self.pro();
        let Some(rv) = &mut self.rv else { return };
        if !pro || rv.github_checked {
            return;
        }
        rv.github_checked = true;
        let repo = rv.repo.clone();
        let cfg = self.cfg.clone();
        let workdir = repo.workdir.clone();
        self.jobs.spawn(move || Msg::GithubReady {
            workdir,
            result: GitHub::for_repo(&repo, &cfg, None).map(|o| o.map(Arc::new)).map_err(|e| e.0),
        });
    }

    fn on_msg(&mut self, m: Msg) {
        if let Some(wd) = m.workdir() {
            if self.rv.as_ref().map(|r| &r.repo.workdir) != Some(wd) {
                let wd = wd.clone();
                return self.background_msg(wd, m);
            }
        }
        match m {
            Msg::GithubReady { workdir, result } => {
                let Some(rv) = self.rv.as_mut().filter(|rv| rv.repo.workdir == workdir) else { return };
                match result {
                    Ok(gh) => rv.github = gh,
                    Err(e) => rv.github_error = Some(e),
                }
                self.handle(Action::FetchPrs { force: true });
            }
            Msg::Detached { workdir, renamed, reattached, result } => {
                let Some(rv) = self.rv.as_mut().filter(|rv| rv.repo.workdir == workdir) else { return };
                match result {
                    Ok(d) => rv.detached = d,
                    Err(e) => self.status_line = Some((format!("Detached-notes scan: {e}"), true)),
                }
                if !reattached.is_empty() {
                    let short = |t: &str| t.strip_prefix("commit:").map(|c| c[..7.min(c.len())].to_string()).unwrap_or_else(|| t.to_string());
                    let list: Vec<String> = reattached.iter().map(|r| format!("{} → {}", short(&r.from), short(&r.to))).collect();
                    self.info(format!(
                        "Re-attached {} commit note{} to the rewritten commit (same patch-id): {}",
                        reattached.len(),
                        if reattached.len() == 1 { "" } else { "s" },
                        list.join(", ")
                    ));
                    self.status_undo = reattached.iter().map(|r| r.id.clone()).collect();
                    self.reload_notes_index();
                    if let Some(rv) = &mut self.rv {
                        // Follow a pinned note to its new home.
                        if let Some(p) = rv.pinned_note.clone() {
                            if let Some(r) = reattached.iter().find(|r| r.from == p.to_string()) {
                                rv.pinned_note = NoteTarget::parse(&r.to).ok();
                            }
                        }
                    }
                }
                if !renamed.is_empty() {
                    let list: Vec<String> = renamed.iter().map(|r| format!("{} → {}", r.from, r.to)).collect();
                    self.info(format!("Branch renamed — moved its note: {}", list.join(", ")));
                    self.reload_notes_index();
                    if let Some(rv) = &mut self.rv {
                        rv.editor.on_external_change(&rv.notes);
                    }
                    // PR status is keyed by local branch name: reload under the new names.
                    self.handle(Action::FetchPrs { force: true });
                }
            }
            Msg::AutoFetched { workdir, result } => {
                let Some(rv) = self.rv.as_mut().filter(|rv| rv.repo.workdir == workdir) else { return };
                rv.auto_fetching = false;
                // Count failures too, so a broken remote retries once per interval, not every frame.
                rv.last_fetch = Some(std::time::Instant::now());
                match result {
                    // Quiet on success: just show the new remote state.
                    Ok(_) => {
                        rv.prs_fetched_at = None;
                        self.refresh();
                    }
                    Err(e) => self.err(format!("Auto-fetch failed: {e}")),
                }
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
                        self.request_pr_threads();
                    }
                    Err(e) => rv.github_error = Some(e),
                }
            }
            Msg::PrDetail { workdir, branch, result } => {
                let Some(rv) = self.rv.as_mut().filter(|rv| rv.repo.workdir == workdir) else { return };
                rv.pr_detail.insert(branch, result.and_then(|d| d.ok_or_else(|| "No pull request for this branch".to_string())));
                self.update_threads();
            }
            Msg::ThreadOp { branch, thread, what, result, .. } => {
                let Some(rv) = &mut self.rv else { return };
                rv.commit_diff.thread_done(&thread, result.is_ok());
                match result {
                    // Reload the PR so the card shows the new comment / state.
                    Ok(()) => self.handle(Action::FetchPrDetail(branch)),
                    Err(e) => self.err(format!("{what} failed: {e}")),
                }
            }
            Msg::Refreshed { result: r, .. } => {
                let Some(rv) = &mut self.rv else { return };
                rv.loading = false;
                match r {
                    Ok(snap) => {
                        // Forget hidden branches that no longer exist (deleted / pruned).
                        if rv.graph_filter.prune(&snap.refs) {
                            let _ = margin_core::history::save(&rv.repo, &rv.graph_filter);
                        }
                        let (g, _w) = graph::layout(&snap.commits);
                        rv.graph = g;
                        rv.row_of = snap.commits.iter().enumerate().map(|(i, c)| (c.id.clone(), i)).collect();
                        rv.tips.clear();
                        for r in snap.refs.iter().filter(|r| r.kind == margin_core::git::RefKind::Local) {
                            let v = rv.tips.entry(r.target.clone()).or_default();
                            if r.is_head { v.insert(0, r.name.clone()) } else { v.push(r.name.clone()) }
                        }
                        // Each task key on its newest commit only (subject keys, plus the
                        // task linked to a branch whose tip this is).
                        rv.row_tasks = crate::commit_list::task_rows(snap.commits.iter(), &rv.tips, &rv.notes);
                        rv.filtered = None;
                        rv.snap = Some(snap);
                        if rv.amend && rv.commit_msg.is_empty() {
                            rv.commit_msg = rv.repo.last_commit_message().unwrap_or_default();
                        }
                        if rv.sel == Sel::Working {
                            self.load_work_diff();
                        }
                        self.handle(Action::FetchPrs { force: false });
                        self.handle(Action::ScanDetached);
                    }
                    Err(e) => self.err(e),
                }
            }
            Msg::WorkDiff { generation, result, .. } => {
                let Some(rv) = &mut self.rv else { return };
                if generation != rv.generation {
                    return;
                }
                match result {
                    Ok(w) => rv.work_diff.set_work(w),
                    Err(e) => rv.work_diff.set_error(e),
                }
            }
            Msg::CommitLoaded { generation, result, .. } => {
                let Some(rv) = &mut self.rv else { return };
                if generation != rv.generation {
                    return;
                }
                match result {
                    Ok((d, f, t, b)) => {
                        rv.commit_detail = Some(d);
                        rv.commit_branches = b;
                        rv.commit_diff.set(f, t);
                        // PR review threads on this commit (Pro): load what's needed, show what's loaded.
                        self.request_pr_threads();
                        self.update_threads();
                    }
                    Err(e) => rv.commit_diff.set_error(e),
                }
            }
            Msg::OpDone { label, result, refresh, .. } => {
                self.busy = None;
                let succeeded = result.is_ok();
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
                        if label != "Push" && succeeded {
                            rv.last_fetch = Some(std::time::Instant::now());
                        }
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
                    // Name the button after what it does; destructive ones in red.
                    let (label, danger) = match action {
                        Action::DeleteNote(_) | Action::DeleteBranch { .. } | Action::StashDrop(_) => ("Delete", true),
                        Action::Discard(_) => ("Discard", true),
                        Action::Hunk { op: HunkOp::Discard, .. } => ("Discard", true),
                        Action::Reset(_, ResetMode::Hard) => ("Reset", true),
                        _ => ("OK", false),
                    };
                    ui.horizontal(|ui| {
                        let text = if danger { egui::RichText::new(label).color(theme::ERROR()) } else { egui::RichText::new(label) };
                        if ui.button(text).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
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
                Dialog::RenameBranch { old, new } => {
                    ui.heading("Rename branch");
                    ui.weak(format!("{old} — its branch note and task link move with it"));
                    let r = ui.add(egui::TextEdit::singleline(new).desired_width(f32::INFINITY));
                    r.request_focus();
                    ui.horizontal(|ui| {
                        let ok = !new.trim().is_empty() && new.trim() != old.as_str();
                        if ui.add_enabled(ok, egui::Button::new("Rename")).clicked() || (ok && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                            run = Some(Action::RenameBranch { old: old.clone(), new: new.trim().to_string() });
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Reattach { from, to_commit, value } => {
                    ui.heading("Re-attach note");
                    ui.weak(format!("from {from}"));
                    ui.horizontal(|ui| {
                        ui.radio_value(to_commit, true, "Commit note");
                        ui.radio_value(to_commit, false, "Branch note");
                    });
                    let hint = if *to_commit { "commit SHA or rev (e.g. HEAD~2)" } else { "branch name" };
                    ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY));
                    ui.weak("If the target already has a note, this one is appended to it.");
                    ui.horizontal(|ui| {
                        let ok = !value.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("Move")).clicked() || (ok && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                            let kind = if *to_commit { "commit" } else { "branch" };
                            run = Some(Action::MoveNote { from: from.clone(), to: format!("{kind}:{}", value.trim()) });
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
                        actions.push(Action::OpenFolderDialog);
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

    /// Single top toolbar:
    /// [repo ▾] [branch ▾] [‹][›]      [Commands ⌘P][Search commits]      status … [Pull ↓n][Push ↑n]
    fn toolbar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let tabs = self.tabs();
        let recent = self.recent_closed(&tabs);
        let Some(rv) = &mut self.rv else { return };
        let busy = self.busy.clone();
        let working = rv.loading || self.jobs.in_flight > 0;
        egui::Panel::top("toolbar").show(ui, |ui| {
            let h = ui.spacing().interact_size.y + 8.0;
            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
            let bar = |ui: &mut egui::Ui, layout: egui::Layout, rect: egui::Rect| ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
            let left_to_right = egui::Layout::left_to_right(egui::Align::Center);
            let right_to_left = egui::Layout::right_to_left(egui::Align::Center);

            // ---- left: repository menu, branch switcher, back/forward
            let mut left = bar(ui, left_to_right, row);
            // Directory menu: open tabs (switch / ×), recent, open, reveal, close.
            let mut title = egui::text::LayoutJob::default();
            let body = egui::TextStyle::Body.resolve(left.style());
            title.append(&rv.repo.name(), 0.0, egui::TextFormat { font_id: body, color: theme::TEXT(), ..Default::default() });
            let menu = left.menu_button(title, |ui| {
                ui.set_min_width(260.0);
                ui.label(egui::RichText::new("OPEN").small().strong().color(theme::TEXT_DIM()));
                for (i, (path, name, active)) in tabs.iter().enumerate() {
                    let (mut switch, mut close) = (false, false);
                    egui::Sides::new().show(
                        ui,
                        |ui| {
                            let mut rt = egui::RichText::new(format!("{}{name}", if *active { "● " } else { "   " }));
                            if *active {
                                rt = rt.strong();
                            }
                            let r = ui.add(egui::Button::new(rt).frame(false));
                            if r.on_hover_text(path.display().to_string()).clicked() {
                                switch = true;
                                ui.close();
                            }
                        },
                        |ui| {
                            close = ui.small_button("×").on_hover_text("Close tab").clicked();
                        },
                    );
                    if switch {
                        actions.push(Action::SwitchTab(i));
                    }
                    if close {
                        actions.push(Action::CloseTab(i));
                    }
                }
                if !recent.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("RECENT").small().strong().color(theme::TEXT_DIM()));
                }
                for r in &recent {
                    let name = std::path::Path::new(r).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if ui.button(format!("   {name}")).on_hover_text(r).clicked() {
                        actions.push(Action::OpenRepo(PathBuf::from(r)));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Open folder…").clicked() {
                    actions.push(Action::OpenFolderDialog);
                    ui.close();
                }
                let wd = rv.repo.workdir.clone();
                if ui.button("Reveal in Finder").clicked() {
                    actions.push(Action::OpenPath { path: wd.clone(), terminal: false });
                    ui.close();
                }
                if ui.button("Open in Terminal").clicked() {
                    actions.push(Action::OpenPath { path: wd, terminal: true });
                    ui.close();
                }
                ui.separator();
                if ui.button("Close tab").clicked() {
                    actions.push(Action::CloseRepo);
                    ui.close();
                }
            });
            menu.response.on_hover_text("Switch directory (Ctrl+Tab, or ⌘P → Change directory)");
            if let Some(snap) = &rv.snap {
                let branch = snap.status.branch.clone().unwrap_or_else(|| "(detached HEAD)".into());
                left.menu_button(egui::RichText::new(&branch).color(theme::ACCENT()), |ui| {
                    ui.set_min_width(220.0);
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        // Same prefix groups as the sidebar; groups are submenus.
                        let tree = crate::branch_tree::build(
                            snap.refs.iter().filter(|r| r.kind == margin_core::git::RefKind::Local).map(|r| (r.name.clone(), r)).collect(),
                        );
                        branch_menu(ui, &tree, actions);
                    });
                    ui.separator();
                    if ui.button("New branch…").clicked() {
                        actions.push(Action::NewBranchDialog { start: None, name: String::new() });
                        ui.close();
                    }
                    if ui.button("More branch commands…").clicked() {
                        actions.push(Action::OpenPaletteWith("branch".into()));
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Switch branch");
                if let Some(op) = snap.pending_op {
                    let r = left.add(egui::Button::new(egui::RichText::new(format!("{op} in progress")).color(theme::ERROR())).small());
                    if r.on_hover_text("Continue / skip / abort").clicked() {
                        actions.push(Action::OpenPaletteWith(op.to_string()));
                    }
                }
            }
            left.add_space(4.0);
            let mod_key = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
            let can_back = rv.hist_pos > 0;
            let can_fwd = rv.hist_pos + 1 < rv.history.len();
            if left.add_enabled(can_back, egui::Button::new(egui::RichText::new("‹").size(17.0))).on_hover_text(format!("Back ({mod_key}[)")).clicked() {
                actions.push(Action::NavBack);
            }
            if left.add_enabled(can_fwd, egui::Button::new(egui::RichText::new("›").size(17.0))).on_hover_text(format!("Forward ({mod_key}])")).clicked() {
                actions.push(Action::NavForward);
            }
            let left_end = left.min_rect().right();

            // ---- right: pull / push with counts
            let mut right = bar(ui, right_to_left, row);
            let (ahead, behind) = rv.snap.as_ref().map(|s| (s.status.ahead, s.status.behind)).unwrap_or((0, 0));
            right.add_enabled_ui(busy.is_none(), |ui| {
                let push = if ahead > 0 { format!("Push ↑{ahead}") } else { "Push".into() };
                if ui.button(push).on_hover_text("Push the current branch (sets upstream if missing)").clicked() {
                    actions.push(Action::Push { force: false });
                }
                let pull = if behind > 0 { format!("Pull ↓{behind}") } else { "Pull".into() };
                if ui.button(pull).on_hover_text("Pull (fetch + merge/rebase per your git config)").clicked() {
                    actions.push(Action::Pull);
                }
                // Split button: [Fetch][▾]
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    let auto = self.settings.auto_fetch;
                    let mins = self.settings.auto_fetch_minutes;
                    let last = rv.last_fetch.map(ago).unwrap_or_else(|| "never".into());
                    ui.menu_button("▾", |ui| {
                        ui.set_min_width(230.0);
                        if ui.button("Fetch now").clicked() {
                            actions.push(Action::Fetch);
                            ui.close();
                        }
                        if ui.button("Fetch a remote…").clicked() {
                            actions.push(Action::OpenPaletteWith("fetch remote".into()));
                            ui.close();
                        }
                        ui.separator();
                        let mut on = auto;
                        if ui.checkbox(&mut on, format!("Auto-fetch every {mins} min")).clicked() {
                            actions.push(Action::ToggleAutoFetch);
                            ui.close();
                        }
                        if ui.button("Fetch settings…").clicked() {
                            actions.push(Action::OpenSettings);
                            ui.close();
                        }
                        ui.separator();
                        ui.weak(format!("Last fetched: {last}"));
                    })
                    .response
                    .on_hover_text("Fetch options");
                    let tip = if auto {
                        format!("Fetch all remotes (prune)\nLast fetched: {last} · auto every {mins} min")
                    } else {
                        format!("Fetch all remotes (prune)\nLast fetched: {last} · auto-fetch off")
                    };
                    let label = if auto { "Fetch ⟳" } else { "Fetch" };
                    if ui.button(label).on_hover_text(tip).clicked() {
                        actions.push(Action::Fetch);
                    }
                });
            });
            let right_start = right.min_rect().left();

            // ---- center: command palette + commit search, centred in the window
            let gap = 16.0;
            let palette_w = 150.0;
            let avail = (right_start - left_end - 2.0 * gap).max(0.0);
            let search_w = (avail - palette_w - 8.0).clamp(120.0, 300.0);
            let center_w = palette_w + 8.0 + search_w;
            let x0 = (row.center().x - center_w / 2.0).max(left_end + gap).min(right_start - gap - center_w);
            let center_rect = egui::Rect::from_min_size(egui::pos2(x0, row.top()), egui::vec2(center_w, row.height()));
            let mut center = bar(ui, left_to_right, center_rect);
            let key = if cfg!(target_os = "macos") { "⌘P" } else { "Ctrl+P" };
            if center
                .add_sized([palette_w, center.spacing().interact_size.y], egui::Button::new(format!("Commands   {key}")))
                .on_hover_text("Command palette: every git action")
                .clicked()
            {
                actions.push(Action::OpenPalette);
            }
            let search = crate::search::search_box(&mut center, rv, search_w, "Search commits…  author: title: commit: task: note: after:")
                .on_hover_text(crate::search::HELP);
            if std::mem::take(&mut self.focus_search) {
                search.request_focus();
            }

            // ---- status (between centre and Pull/Push): spinner, message, Undo
            let status_rect = egui::Rect::from_min_max(egui::pos2(center_rect.right() + gap, row.top()), egui::pos2(right_start - 8.0, row.bottom()));
            if status_rect.width() > 24.0 {
                let mut st = bar(ui, right_to_left, status_rect);
                if !self.status_undo.is_empty() && st.small_button("Undo").clicked() {
                    for id in self.status_undo.drain(..) {
                        actions.push(Action::UndoReattach(id));
                    }
                }
                if let Some((m, err)) = &self.status_line {
                    let first = m.lines().next().unwrap_or("");
                    let color = if *err { theme::ERROR() } else { theme::TEXT_DIM() };
                    let resp = st
                        .add(egui::Label::new(egui::RichText::new(first).color(color)).truncate().sense(egui::Sense::click()))
                        .on_hover_text(format!("{m}\n\n(click to dismiss)"));
                    if resp.clicked() {
                        self.status_line = None;
                        self.status_undo.clear();
                    }
                }
                let reminder = match &self.license {
                    margin_core::license::Status::Trial { days_left } if *days_left <= 3 => Some(format!("Pro trial: {days_left}d left")),
                    margin_core::license::Status::TrialExpired => Some("Pro trial ended".to_string()),
                    margin_core::license::Status::NeedsRenewal(_) => Some("Renew Margin Pro".to_string()),
                    _ => None,
                };
                if let Some(r) = reminder {
                    if st.small_button(egui::RichText::new(r).color(theme::ACCENT())).on_hover_text(self.license.describe()).clicked() {
                        actions.push(Action::OpenSettings);
                    }
                }
                if let Some(b) = &busy {
                    st.label(egui::RichText::new(format!("{b}…")).color(theme::TEXT_DIM()));
                    st.spinner();
                } else if working {
                    st.spinner();
                }
            }
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
        // Native menu bar (macOS): keep it current, then run what was chosen.
        let tabs = self.tabs();
        let recent = self.recent_closed(&tabs);
        self.menu.update(&recent, self.rv.is_some());
        for cmd in self.menu.poll() {
            self.menu_cmd(cmd, &ctx);
        }

        // Refresh when the window regains focus (like Sublime Merge) and
        // pick up note edits made by agents (see start_notes_watcher).
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if focused && !self.was_focused {
            self.update_license();
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
        let (refresh, save, palette) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::R),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::P)
                    || i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::P),
            )
        });
        let (back, fwd) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::OpenBracket) || i.pointer.button_pressed(egui::PointerButton::Extra1),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::CloseBracket) || i.pointer.button_pressed(egui::PointerButton::Extra2),
            )
        });
        // ⌘⇧↩: diff focus mode on / off
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Enter)) {
            self.focus_diff = !self.focus_diff;
        }
        let (next_tab, prev_tab) = ctx.input_mut(|i| {
            let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
            (
                i.consume_key(egui::Modifiers::CTRL, egui::Key::Tab) || i.consume_key(shift, egui::Key::CloseBracket),
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Tab) || i.consume_key(shift, egui::Key::OpenBracket),
            )
        });
        if next_tab {
            self.handle(Action::NextTab);
        }
        if prev_tab {
            self.handle(Action::PrevTab);
        }
        if back {
            self.handle(Action::NavBack);
        }
        if fwd {
            self.handle(Action::NavForward);
        }
        if palette {
            if self.palette.is_some() {
                self.palette = None;
            } else {
                self.handle(Action::OpenPalette);
            }
        }
        if refresh {
            self.refresh();
        }
        if save {
            self.save_note();
        }

        let mut actions = Vec::new();
        if self.rv.is_none() {
            actions = self.welcome(ui);
        } else {
            // Tab strip first so it sits above the toolbar.
            let tabs = self.tabs();
            let recent = self.recent_closed(&tabs);
            crate::tab_strip::show(ui, &tabs, &recent, &mut actions);
            self.toolbar(ui, &mut actions);
            let tasks_configured = self.tasks.is_some();
            let pro = self.pro();
            let tasks_enabled = tasks_configured && pro;
            let license = self.license.clone();
            let rv = self.rv.as_mut().unwrap();
            // Notes sit at the bottom of the details column, or under the sidebar
            // and history together (Notes → "Move notes…"); same panel either way.
            let under_history = self.state.notes_under_history;
            let task_url = |k: &str| tasks::task_url(&self.cfg, self.tasks.as_deref(), self.settings.ticket_url.as_deref(), k);
            let notes = |ui: &mut egui::Ui, rv: &mut RepoView, state: &mut State, actions: &mut Vec<Action>| {
                // The dragged height, or fitted to the content when collapsed /
                // empty (see notes_panel). The panel draws its own grip.
                let max_h = (ui.available_height() - 160.0).max(crate::notes_panel::MIN_H);
                let target = match rv.editor.fit_h {
                    Some(fit) => fit + 6.0,
                    None => state.notes_height.unwrap_or(crate::notes_panel::DEFAULT_H).clamp(crate::notes_panel::MIN_H, max_h),
                };
                // Collapse / expand slides; dragging the grip follows the mouse exactly.
                let dragging = ui.input(|i| i.pointer.primary_down());
                let notes_h = ui.ctx().animate_value_with_time(egui::Id::unique("notes_panel_h"), target, if dragging { 0.0 } else { 0.12 });
                egui::Panel::bottom("notes").exact_size(notes_h).show(ui, |ui| {
                    crate::notes_panel::show(ui, rv, &self.task_cache, tasks_configured, &license, &task_url, &self.author, state, notes_h, actions);
                });
            };
            let before = crate::diff_view::DiffPrefs {
                split: self.state.diff_split,
                ignore_ws: self.state.diff_ignore_ws,
                expand_commit: !self.state.diff_collapse_commits,
                expand_staged: !self.state.diff_collapse_staged,
                expand_unstaged: self.state.diff_expand_unstaged,
                focused: self.focus_diff,
            };
            let mut diff_prefs = before;
            let state = &mut self.state;
            let focus = self.focus_diff;
            let details = |ui: &mut egui::Ui, rv: &mut RepoView, state: &mut State, diff_prefs: &mut crate::diff_view::DiffPrefs, actions: &mut Vec<Action>| {
                if !under_history {
                    notes(ui, rv, state, actions);
                }
                egui::CentralPanel::default().show(ui, |ui| {
                    crate::details::show(ui, rv, diff_prefs, &task_url, actions);
                });
            };
            if focus {
                // Focus mode: the diff column gets the whole window.
                egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| details(ui, rv, state, &mut diff_prefs, &mut actions));
            } else {
                // The details column can't grow over the branches and history:
                // they keep enough width that their edges stay grabbable.
                let max_details = (ui.available_width() - 320.0).max(360.0);
                egui::Panel::right("details").resizable(true).default_size(560.0).min_size(360.0).max_size(max_details).show(ui, |ui| {
                    details(ui, rv, state, &mut diff_prefs, &mut actions)
                });
            }
            if diff_prefs != before {
                state.diff_split = diff_prefs.split;
                state.diff_ignore_ws = diff_prefs.ignore_ws;
                let _ = config::save_state(state);
                if diff_prefs.ignore_ws != before.ignore_ws {
                    actions.push(Action::ReloadDiff);
                }
            }
            // Notes on the left: one wide panel under both the sidebar and the
            // history (added before them, so it spans their full width).
            if !focus {
                if under_history {
                    notes(ui, rv, state, &mut actions);
                }
                let max_sidebar = (ui.available_width() * 0.5).max(160.0);
                egui::Panel::left("sidebar").resizable(true).default_size(220.0).min_size(150.0).max_size(max_sidebar).show(ui, |ui| {
                    // Fill the panel: otherwise it shrinks to its content when no
                    // row has a right-aligned badge (e.g. a repo without upstreams).
                    ui.set_min_width(ui.available_width());
                    crate::sidebar::show(ui, rv, tasks_enabled, self.my_tasks.as_ref(), &mut actions);
                });
                egui::CentralPanel::default().show(ui, |ui| {
                    crate::commit_list::show(ui, rv, &mut actions);
                });
            }
        }
        self.auto_fetch_tick(&ctx);
        self.dialogs(&ctx);
        self.settings_window(&ctx, &mut actions);
        if let Some(p) = &mut self.palette {
            match p.show(&ctx) {
                crate::palette::Outcome::Open => {}
                crate::palette::Outcome::Close => self.palette = None,
                crate::palette::Outcome::Run(a, key) => {
                    self.palette = None;
                    actions.extend(a);
                    let recent = &mut self.state.recent_commands;
                    recent.retain(|k| k != &key);
                    recent.insert(0, key);
                    recent.truncate(crate::palette::RECENT_MAX);
                    let _ = config::save_state(&self.state);
                }
            }
        }
        for a in actions {
            self.handle(a);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(rv) = &mut self.rv {
            rv.editor.save_if_dirty(&rv.notes);
        }
        for t in &mut self.others {
            t.editor.save_if_dirty(&t.notes);
        }
        self.save_tabs();
    }
}

impl MarginApp {
    /// Start a background fetch when due; otherwise ask egui to wake us at the
    /// due time (no polling thread, no repaints in between).
    fn auto_fetch_tick(&mut self, ctx: &egui::Context) {
        if !self.settings.auto_fetch {
            return;
        }
        let Some(rv) = &mut self.rv else { return };
        if rv.snap.is_none() || rv.auto_fetching {
            return;
        }
        let interval = std::time::Duration::from_secs(self.settings.auto_fetch_minutes.max(1) as u64 * 60);
        // First run shortly after opening the repo, then every `interval`.
        let due = rv.last_fetch.map(|t| t + interval).unwrap_or(rv.opened_at + std::time::Duration::from_secs(3));
        let now = std::time::Instant::now();
        if now < due {
            ctx.request_repaint_after(due - now);
            return;
        }
        if self.busy.is_some() {
            // A user operation is running: try again shortly.
            ctx.request_repaint_after(std::time::Duration::from_secs(5));
            return;
        }
        rv.auto_fetching = true;
        let repo = rv.repo.clone();
        let workdir = repo.workdir.clone();
        self.jobs.spawn(move || Msg::AutoFetched { workdir, result: repo.fetch().map_err(|e| e.0) });
    }

    fn settings_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if !self.show_settings {
            return;
        }
        let mut changed = false;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("settings")).show(ctx, |ui| {
            ui.set_width(720.0);
            ui.horizontal(|ui| {
                ui.heading("Settings");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    ui.label(egui::RichText::new("Saved automatically").small().color(theme::TEXT_DIM()))
                        .on_hover_text(config::config_dir().join("settings.json").display().to_string());
                });
            });
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                // ---- sections
                ui.vertical(|ui| {
                    ui.set_width(150.0);
                    ui.style_mut().interaction.selectable_labels = false;
                    for t in SettingsTab::ALL {
                        let on = self.settings_tab == t;
                        let text = if on { egui::RichText::new(t.label()).strong() } else { egui::RichText::new(t.label()) };
                        if ui.add(egui::Button::selectable(on, text).min_size(egui::vec2(146.0, 26.0))).clicked() {
                            self.settings_tab = t;
                        }
                    }
                });
                ui.separator();
                // ---- the selected section
                ui.vertical(|ui| {
                    ui.set_width(540.0);
                    let tab = self.settings_tab;
                    ui.label(egui::RichText::new(tab.label()).size(ui.style().text_styles[&egui::TextStyle::Heading].size - 1.0).strong());
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(tab.intro()).color(theme::TEXT_DIM()));
                    ui.add_space(14.0);
                    egui::ScrollArea::vertical().id_salt("settings_scroll").max_height(440.0).min_scrolled_height(440.0).auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_width(526.0);
                        match tab {
                            SettingsTab::Appearance => changed |= self.settings_appearance(ui),
                            SettingsTab::Pro => self.settings_pro(ui, actions),
                            SettingsTab::Sync => self.sync_settings(ui, actions),
                            SettingsTab::Fetch => changed |= self.settings_fetch(ui, actions),
                            SettingsTab::Tasks => changed |= self.settings_tasks(ui),
                            SettingsTab::Diffs => self.settings_diffs(ui),
                        }
                    });
                });
            });
        });
        if changed {
            self.settings.auto_fetch_minutes = self.settings.auto_fetch_minutes.clamp(1, 1440);
            if let Err(e) = config::save_settings(&self.settings) {
                self.err(e);
            }
        }
        if close || modal.should_close() {
            self.show_settings = false;
        }
    }

    /// Code and notes text size (Settings, else config.json, else the default).
    fn font_size(&self) -> f32 {
        self.settings.font_size.or(self.cfg.font_size).unwrap_or(DEFAULT_FONT).clamp(MIN_FONT, MAX_FONT)
    }

    /// Interface text size.
    fn ui_font_size(&self) -> f32 {
        self.settings.ui_font_size.unwrap_or(DEFAULT_FONT).clamp(MIN_UI_FONT, MAX_UI_FONT)
    }

    /// Appearance: text size and theme picker. Returns whether settings changed.
    fn settings_appearance(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        // ---- text sizes (applied live while dragging; saved on release)
        let (mut code, mut iface) = (self.font_size(), self.ui_font_size());
        let (mut set, mut save) = (false, false);
        let mut size_row = |ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, presets: &[(&str, f32)]| {
            ui.horizontal(|ui| {
                let r = ui.add(egui::Slider::new(v, range).step_by(0.5).fixed_decimals(1).suffix(" pt"));
                set |= r.changed();
                save |= r.drag_stopped() || (r.changed() && !r.dragged());
                ui.add_space(8.0);
                for &(label, p) in presets {
                    if ui.selectable_label((*v - p).abs() < 0.01, label).clicked() {
                        *v = p;
                        set = true;
                        save = true;
                    }
                }
            });
        };
        // ---- fonts (start listing the installed ones now, for the pickers)
        let _ = crate::fonts::families(ui.ctx());
        let mut fonts_changed = false;
        setting(ui, "Fonts", "Any installed font. Characters a font lacks (e.g. Korean in a code font) still show, from the built-in and system fonts.", |ui| {
            egui::Grid::new("font-pickers").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Interface");
                fonts_changed |= font_picker(ui, "ui-font", &mut self.settings.ui_font, false);
                ui.end_row();
                ui.label("Code");
                fonts_changed |= font_picker(ui, "code-font", &mut self.settings.code_font, true);
                ui.end_row();
            });
        });
        if fonts_changed {
            crate::fonts::apply(ui.ctx(), self.settings.ui_font.as_ref(), self.settings.code_font.as_ref());
            changed = true;
        }
        setting(ui, "Code and notes", "Diffs, notes and commit messages: the text you read.", |ui| {
            size_row(ui, &mut code, MIN_FONT..=MAX_FONT, &[("Small", 12.0), ("Default", DEFAULT_FONT), ("Large", 15.0), ("Larger", 17.0)]);
            ui.add_space(6.0);
            ui.label(egui::RichText::new("fn login(user: &str) -> Result<()>").monospace().size(code - 0.5));
            ui.label(egui::RichText::new("Release notes · 다람쥐 헌 쳇바퀴에 타고파").size(code).color(theme::TEXT_DIM()));
        });
        setting(ui, "Interface", "Lists, sidebar, toolbar, buttons and menus. Kept in a small range so the layout stays the same.", |ui| {
            size_row(ui, &mut iface, MIN_UI_FONT..=MAX_UI_FONT, &[("Compact", 12.0), ("Default", DEFAULT_FONT), ("Large", 14.0)]);
        });
        if set {
            self.settings.font_size = Some(code);
            self.settings.ui_font_size = Some(iface);
            theme::apply(ui.ctx(), iface, code);
        }
        changed |= save;
        let current = self.settings.theme.clone().unwrap_or_else(|| "margin".into());
        for (title, dark) in [("Dark themes", true), ("Light themes", false)] {
            setting(ui, title, "", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
                    for p in theme::THEMES.iter().filter(|p| p.dark == dark) {
                        if theme_card(ui, p, p.id == current).clicked() && p.id != current {
                            self.settings.theme = Some(p.id.to_string());
                            theme::set(Some(p.id));
                            theme::apply(ui.ctx(), self.ui_font_size(), self.font_size());
                            changed = true;
                        }
                    }
                });
            });
        }
        ui.label(
            egui::RichText::new("Material themes follow the Material Theme palettes (Material, Darker, Palenight, Ocean, Lighter); Solarized uses Ethan Schoonover's palette.")
                .small()
                .color(theme::TEXT_DIM()),
        );
        changed
    }

    fn settings_pro(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let color = if self.license.is_pro() { theme::ADDED() } else { theme::ACCENT() };
        setting(ui, "Status", if margin_core::license::is_dev_build() { "Development build: accepts development keys only." } else { "" }, |ui| {
            ui.label(egui::RichText::new(self.license.describe()).color(color));
        });
        if matches!(self.license, margin_core::license::Status::Pro(_)) {
            setting(ui, "This computer", "Frees the licence for use on another computer.", |ui| {
                if ui.button("Remove licence from this computer").clicked() {
                    actions.push(Action::RemoveLicense);
                }
            });
            return;
        }
        setting(ui, "Licence key", "Paste the key from your purchase email (it starts with MARGIN1.).", |ui| {
            ui.add(egui::TextEdit::multiline(&mut self.license_input).hint_text("MARGIN1.…").desired_rows(3).desired_width(f32::INFINITY));
            if let Some(e) = &self.license_error {
                ui.colored_label(theme::ERROR(), e);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.license_input.trim().is_empty(), egui::Button::new("Activate")).clicked() {
                    actions.push(Action::InstallLicense(self.license_input.clone()));
                }
                if ui.button("Buy Margin Pro").clicked() {
                    actions.push(Action::OpenUrl(crate::pro::PURCHASE_URL.into()));
                }
            });
        });
    }

    fn settings_fetch(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) -> bool {
        let mut changed = false;
        let last = self.rv.as_ref().and_then(|rv| rv.last_fetch).map(ago);
        setting(
            ui,
            "Automatic fetch",
            "Fetches all remotes (with prune) for the open repository, using your git credential helper or SSH agent. It never prompts; failures show in the toolbar.",
            |ui| {
                changed |= ui.checkbox(&mut self.settings.auto_fetch, "Fetch in the background").changed();
                ui.add_enabled_ui(self.settings.auto_fetch, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label("Every");
                        changed |= ui.add(egui::DragValue::new(&mut self.settings.auto_fetch_minutes).range(1..=1440).speed(0.2).suffix(" min")).changed();
                        for m in [5, 15, 30, 60] {
                            if ui.selectable_label(self.settings.auto_fetch_minutes == m, format!("{m} min")).clicked() {
                                self.settings.auto_fetch_minutes = m;
                                changed = true;
                            }
                        }
                    });
                });
            },
        );
        setting(ui, "Last fetch", "", |ui| {
            ui.horizontal(|ui| {
                ui.label(last.clone().unwrap_or_else(|| "never".into()));
                if self.rv.is_some() && ui.small_button("Fetch now").clicked() {
                    actions.push(Action::Fetch);
                }
            });
        });
        changed
    }

    fn settings_tasks(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let example = self.settings.ticket_url.as_deref().and_then(|t| tasks::ticket_link(t, "PROJ-123"));
        let help = match &example {
            Some(u) => format!("PROJ-123 opens {u}"),
            None => "A Jira site such as https://yourcompany.atlassian.net (keys open /browse/KEY), or for other trackers a link with {key}, e.g. https://linear.app/acme/issue/{key}.".into(),
        };
        setting(ui, "Ticket link", &help, |ui| {
            let mut t = self.settings.ticket_url.clone().unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut t).hint_text("https://yourcompany.atlassian.net").desired_width(f32::INFINITY)).changed() {
                self.settings.ticket_url = Some(t.trim().to_string()).filter(|t| !t.is_empty());
                changed = true;
            }
        });
        changed
    }

    fn settings_diffs(&mut self, ui: &mut egui::Ui) {
        let st = &mut self.state;
        let mut commits = !st.diff_collapse_commits;
        let mut staged = !st.diff_collapse_staged;
        let mut view_changed = false;
        setting(ui, "Files expanded when a diff opens", "Click a file's header to expand or collapse it; that choice is kept while you work.", |ui| {
            view_changed |= ui.checkbox(&mut commits, "In commits").changed();
            view_changed |= ui.checkbox(&mut staged, "Staged changes").changed();
            view_changed |= ui.checkbox(&mut st.diff_expand_unstaged, "Unstaged changes").changed();
        });
        let mut ws = st.diff_ignore_ws;
        let mut split = st.diff_split;
        setting(ui, "Layout", "Also switchable from the icons above every diff.", |ui| {
            ui.horizontal(|ui| {
                view_changed |= ui.radio_value(&mut split, false, "Inline").changed();
                view_changed |= ui.radio_value(&mut split, true, "Side by side").changed();
            });
        });
        setting(ui, "Whitespace", "Hide changes that only touch spaces, tabs or indentation (git diff -w). Staging single hunks is off while this is on.", |ui| {
            view_changed |= ui.checkbox(&mut ws, "Ignore whitespace changes").changed();
        });
        if view_changed {
            st.diff_collapse_commits = !commits;
            st.diff_collapse_staged = !staged;
            st.diff_split = split;
            let reload = st.diff_ignore_ws != ws;
            st.diff_ignore_ws = ws;
            let _ = config::save_state(st);
            if reload {
                self.handle(Action::ReloadDiff);
            }
        }
    }

    fn sync_settings(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        use margin_core::sync::{FREE_REPOS, RepoSync};
        ui.label(egui::RichText::new(format!("Free: {FREE_REPOS} repository · Margin Pro: unlimited.")).small().color(theme::TEXT_DIM()));
        ui.add_space(12.0);
        ui.label(egui::RichText::new("Sync folder").strong());
        ui.add_space(4.0);
        // Folder
        ui.horizontal_wrapped(|ui| {
            ui.label("Folder:");
            match &self.settings.sync_folder {
                Some(f) => {
                    ui.label(egui::RichText::new(f).monospace().small());
                    if ui.small_button("Clear").clicked() {
                        actions.push(Action::SetSyncFolder(None));
                    }
                }
                None => {
                    ui.weak("not set");
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (label, path) in margin_core::sync::detect_providers() {
                if ui.small_button(&label).on_hover_text(path.display().to_string()).clicked() {
                    actions.push(Action::SetSyncFolder(Some(path.display().to_string())));
                }
            }
            #[cfg(target_os = "macos")]
            if ui.small_button("Choose folder…").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    actions.push(Action::SetSyncFolder(Some(p.display().to_string())));
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                ui.add(egui::TextEdit::singleline(&mut self.sync_folder_input).hint_text("/path/to/synced/folder").desired_width(200.0));
                if ui.small_button("Use").clicked() && !self.sync_folder_input.trim().is_empty() {
                    actions.push(Action::SetSyncFolder(Some(self.sync_folder_input.trim().to_string())));
                }
            }
        });
        ui.add_space(16.0);
        // This repository
        let Some(rv) = &self.rv else { return };
        ui.label(egui::RichText::new("This repository").strong());
        ui.add_space(4.0);
        let on = self.settings.synced_repos.contains(&rv.repo_id);
        let mut want = on;
        if ui.checkbox(&mut want, format!("Sync notes for {}", rv.repo.name())).changed() {
            actions.push(Action::SetNotesSync(want));
        }
        let (text, color) = sync_status_text(rv);
        if !text.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(text).small().color(color));
                if matches!(rv.sync_state, RepoSync::Active { .. }) && ui.small_button("Sync now").clicked() {
                    actions.push(Action::SyncNow);
                }
            });
        }
        if self.sync_limit_hit || matches!(rv.sync_state, RepoSync::Paused) {
            crate::pro::lock_hint(ui, &format!("Syncing more than {FREE_REPOS} repository"), &self.license, actions);
        }
        let n = self.settings.synced_repos.len();
        if n > 0 {
            ui.label(egui::RichText::new(format!("{n} repositor{} with sync on", if n == 1 { "y" } else { "ies" })).small().color(theme::TEXT_DIM()));
        }
    }
}

/// Human status for a tab's notes sync: ("synced 2 min ago", colour).
pub fn sync_status_text(rv: &RepoView) -> (String, egui::Color32) {
    use margin_core::sync::RepoSync;
    match &rv.sync_state {
        RepoSync::Off => (String::new(), theme::TEXT_DIM()),
        RepoSync::NoFolder => ("Sync is on, but no sync folder is set (or it's not available)".into(), theme::ACCENT()),
        RepoSync::Paused => ("Sync paused — this repository is over the free limit".into(), theme::ACCENT()),
        RepoSync::Active { remote } => match &rv.sync.lock().unwrap().last {
            None => (format!("Syncing with {}", remote.display()), theme::TEXT_DIM()),
            Some((t, Ok(_))) => (format!("Synced {} · {}", ago(*t), remote.display()), theme::ADDED()),
            Some((_, Err(e))) => (format!("Sync failed: {e}"), theme::ERROR()),
        },
    }
}

/// "just now", "4 min ago", "2 h ago"
fn ago(t: std::time::Instant) -> String {
    let s = t.elapsed().as_secs();
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        _ => format!("{} h ago", s / 3600),
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn open_path(path: &std::path::Path, terminal: bool, app: &mut MarginApp) {
    #[cfg(target_os = "macos")]
    let r = if terminal {
        std::process::Command::new("/usr/bin/open").args(["-a", "Terminal"]).arg(path).spawn()
    } else {
        std::process::Command::new("/usr/bin/open").arg(path).spawn()
    };
    #[cfg(not(target_os = "macos"))]
    let r = if terminal {
        std::process::Command::new("x-terminal-emulator").current_dir(path).spawn()
    } else {
        std::process::Command::new(if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(path).spawn()
    };
    if let Err(e) = r {
        app.err(format!("open {}: {e}", path.display()));
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


fn branch_menu(ui: &mut egui::Ui, nodes: &[crate::branch_tree::Node<&margin_core::git::RefInfo>], actions: &mut Vec<Action>) {
    for n in nodes {
        match n {
            crate::branch_tree::Node::Leaf { label, item: r } => {
                let text = if r.is_head { format!("● {label}") } else { format!("   {label}") };
                if ui.add_enabled(!r.is_head, egui::Button::new(text)).on_hover_text(&r.name).clicked() {
                    actions.push(Action::Checkout(r.name.clone()));
                    ui.close();
                }
            }
            crate::branch_tree::Node::Group { label, children, count, .. } => {
                let has_head = crate::branch_tree::any(children, &|r: &&margin_core::git::RefInfo| r.is_head);
                let text = format!("{}{label}  ({count})", if has_head { "● " } else { "   " });
                ui.menu_button(text, |ui| branch_menu(ui, children, actions));
            }
        }
    }
}
