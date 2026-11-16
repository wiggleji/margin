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
    /// Background auto-fetch finished (kept separate from `OpDone` so it never
    /// touches the busy state of a user-started operation).
    AutoFetched { workdir: PathBuf, result: Result<String, String> },
    // GitHub messages carry the repo path so late replies for a closed repo are dropped.
    GithubReady { workdir: PathBuf, result: Result<Option<Arc<GitHub>>, String> },
    Prs { workdir: PathBuf, result: Result<HashMap<String, PrSummary>, String> },
    PrDetail { workdir: PathBuf, branch: String, result: Result<Option<PrDetail>, String> },
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
            | Msg::GithubReady { workdir, .. }
            | Msg::Prs { workdir, .. }
            | Msg::PrDetail { workdir, .. }
            | Msg::Detached { workdir, .. } => Some(workdir),
            Msg::Task { .. } | Msg::MyTasks(_) => None,
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
