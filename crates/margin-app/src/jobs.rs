//! Background work. Every git / network call runs on a short-lived thread
//! so the UI thread never blocks (monorepo `git status` can take seconds).

use eframe::egui;
use margin_core::diff::FileDiff;
use margin_core::git::{Commit, CommitDetail, RefInfo, Stash, Status, Worktree};
use margin_core::github::{GitHub, PrDetail, PrSummary};
use margin_core::tasks::Task;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

pub struct Snapshot {
    pub status: Status,
    pub commits: Vec<Commit>,
    pub refs: Vec<RefInfo>,
    pub stashes: Vec<Stash>,
    pub pending_op: Option<&'static str>,
    pub limit: usize,
    /// All worktrees of the repository (`git worktree list`), bare ones excluded.
    pub worktrees: Vec<WorktreeInfo>,
}

pub struct WorktreeInfo {
    pub wt: Worktree,
    /// The worktree this tab shows.
    pub current: bool,
}

impl Snapshot {
    /// Another worktree that has `branch` checked out (git refuses to check it out here).
    pub fn worktree_for(&self, branch: &str) -> Option<&WorktreeInfo> {
        self.worktrees.iter().find(|w| !w.current && w.wt.branch.as_deref() == Some(branch))
    }
}

pub enum Msg {
    // Repo-scoped messages carry `workdir` so a result never lands in the
    // wrong tab (see MarginApp::on_msg).
    Refreshed { workdir: PathBuf, result: Result<Box<Snapshot>, String> },
    WorkDiff { workdir: PathBuf, generation: u64, result: Result<crate::diff_view::WorkDiff, String> },
    CommitLoaded { workdir: PathBuf, generation: u64, result: Result<(CommitDetail, Vec<FileDiff>, bool, Vec<String>), String> },
    /// A git operation finished. `refresh`: reload repo state afterwards.
    OpDone { workdir: PathBuf, label: String, result: Result<String, String>, refresh: bool },
    Task { key: String, result: Result<Task, String> },
    MyTasks(Result<Vec<Task>, String>),
    /// Settings → Accounts "Test": who the token signs in as ("github" / "jira").
    AccountTest { which: &'static str, result: Result<String, String> },
    /// Review mode: the comparison (or pull request) loaded.
    ReviewLoaded { workdir: PathBuf, generation: u64, result: Result<Box<crate::review::Loaded>, String> },
    /// Review mode: one commit's diff (Commits tab).
    ReviewCommit { workdir: PathBuf, generation: u64, sha: String, result: Result<(Vec<FileDiff>, bool), String> },
    /// Review mode: the pull request again (after a reply, resolve or review).
    ReviewPr { workdir: PathBuf, generation: u64, result: Result<Option<PrDetail>, String> },
    /// Review mode: a reply / resolve finished.
    ReviewThreadDone { workdir: PathBuf, id: String, result: Result<(), String> },
    /// Review mode: the review was submitted (its URL).
    ReviewSubmitted { workdir: PathBuf, result: Result<String, String> },
    /// Open pull requests (for choosing one to review).
    OpenPrs { workdir: PathBuf, result: Result<Vec<margin_core::github::PrListItem>, String> },
    /// The update check finished: the latest release, if any.
    UpdateChecked { manual: bool, result: Result<Option<margin_core::update::Release>, String> },
    /// The update was downloaded and checked: the new Margin.app.
    UpdateStaged(Result<(margin_core::update::Release, PathBuf), String>),
    /// Installing the `margin` terminal command finished.
    CliInstalled(Result<String, String>),
    /// Background auto-fetch finished (kept separate from `OpDone` so it never
    /// touches the busy state of a user-started operation).
    AutoFetched { workdir: PathBuf, result: Result<String, String> },
    /// The commits whose changes add or remove `text` (a `code:` search).
    CodeSearch { workdir: PathBuf, text: String, result: Result<std::collections::HashSet<String>, String> },
    /// Insights: the commits of a range, with what they changed.
    InsightsLoaded { workdir: PathBuf, generation: u64, result: Result<Vec<margin_core::insights::CommitStat>, String> },
    /// GitHub answered an @mention lookup (the answers go into the mention directory).
    Mentions { workdir: PathBuf, result: Result<(), String> },
    // GitHub messages carry the repo path so late replies for a closed repo are dropped.
    GithubReady { workdir: PathBuf, result: Result<Option<Arc<GitHub>>, String> },
    Prs { workdir: PathBuf, result: Result<HashMap<String, PrSummary>, String> },
    PrDetail { workdir: PathBuf, branch: String, result: Result<Option<PrDetail>, String> },
    /// A review-thread reply or resolve finished.
    ThreadOp { workdir: PathBuf, branch: String, thread: String, what: &'static str, result: Result<(), String> },
    /// Branch notes moved after renames, and the notes left detached.
    Detached {
        workdir: PathBuf,
        renamed: Vec<margin_core::detached::Renamed>,
        reattached: Vec<margin_core::detached::Reattached>,
        result: Result<Vec<margin_core::detached::DetachedNote>, String>,
    },
}

impl Msg {
    /// The repository a message belongs to (None: app-wide, e.g. task lookups).
    pub fn workdir(&self) -> Option<&PathBuf> {
        match self {
            Msg::Refreshed { workdir, .. }
            | Msg::WorkDiff { workdir, .. }
            | Msg::CommitLoaded { workdir, .. }
            | Msg::OpDone { workdir, .. }
            | Msg::AutoFetched { workdir, .. }
            | Msg::CodeSearch { workdir, .. }
            | Msg::Mentions { workdir, .. }
            | Msg::InsightsLoaded { workdir, .. }
            | Msg::GithubReady { workdir, .. }
            | Msg::Prs { workdir, .. }
            | Msg::PrDetail { workdir, .. }
            | Msg::ThreadOp { workdir, .. }
            | Msg::ReviewLoaded { workdir, .. }
            | Msg::ReviewCommit { workdir, .. }
            | Msg::ReviewPr { workdir, .. }
            | Msg::ReviewThreadDone { workdir, .. }
            | Msg::ReviewSubmitted { workdir, .. }
            | Msg::OpenPrs { workdir, .. }
            | Msg::Detached { workdir, .. } => Some(workdir),
            Msg::Task { .. } | Msg::MyTasks(_) | Msg::AccountTest { .. } | Msg::CliInstalled(_) | Msg::UpdateChecked { .. } | Msg::UpdateStaged(_) => None,
        }
    }
}

pub struct Jobs {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    pub in_flight: usize,
}

impl Jobs {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        Jobs { tx, rx, ctx: ctx.clone(), in_flight: 0 }
    }

    pub fn spawn(&mut self, f: impl FnOnce() -> Msg + Send + 'static) {
        self.in_flight += 1;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) -> Vec<Msg> {
        let msgs: Vec<Msg> = self.rx.try_iter().collect();
        self.in_flight = self.in_flight.saturating_sub(msgs.len());
        msgs
    }
}
