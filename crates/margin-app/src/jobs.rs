//! Background work. Every git / network call runs on a short-lived thread
//! so the UI thread never blocks (monorepo `git status` can take seconds).

use eframe::egui;
use margin_core::diff::FileDiff;
use margin_core::git::{Commit, CommitDetail, RefInfo, Stash, Status};
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
}

pub enum Msg {
    Refreshed(Result<Box<Snapshot>, String>),
    WorkDiff { generation: u64, result: Result<(Vec<FileDiff>, bool), String> },
    CommitLoaded { generation: u64, result: Result<(CommitDetail, Vec<FileDiff>, bool), String> },
    /// A git operation finished. `refresh`: reload repo state afterwards.
    OpDone { label: String, result: Result<String, String>, refresh: bool },
    Task { key: String, result: Result<Task, String> },
    MyTasks(Result<Vec<Task>, String>),
    // GitHub messages carry the repo path so late replies for a closed repo are dropped.
    GithubReady { workdir: PathBuf, result: Result<Option<Arc<GitHub>>, String> },
    Prs { workdir: PathBuf, result: Result<HashMap<String, PrSummary>, String> },
    PrDetail { workdir: PathBuf, branch: String, result: Result<Option<PrDetail>, String> },
    /// Branch notes moved after renames, and the notes left detached.
    Detached {
        workdir: PathBuf,
        renamed: Vec<margin_core::detached::Renamed>,
        result: Result<Vec<margin_core::detached::DetachedNote>, String>,
    },
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
