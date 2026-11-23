//! Review mode: what a branch changes compared with another, the way a pull
//! request shows it, and GitHub pull requests themselves.
//!
//! It takes over the history and details area (the sidebar stays):
//! a header with base ← head (or the pull request), then three tabs:
//! * **Files changed** — `base...head` (from where the branch split off), a
//!   file list with Viewed marks (see `margin_core::review`) beside the diff,
//!   which shows the PR's review threads; on a pull request any line takes a
//!   new comment, kept as *pending* until the review is submitted.
//! * **Commits** — `base..head`; one opens its own diff.
//! * **Conversation** — the PR's description, checks, reviews and comments.
//!
//! *Review changes* submits a review (comment / approve / request changes)
//! with the pending comments.

use crate::app::{Action, RepoView};
use crate::diff_view::{DRAFT, DiffEvent, DiffMode, DiffPrefs, DiffView, InlineThread, PENDING};
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::diff::FileDiff;
use margin_core::git::Commit;
use margin_core::github::{CommentKind, DraftComment, PrDetail, ReviewEvent};
use margin_core::review::{self, Viewed};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Files,
    Commits,
    Conversation,
}

/// The "Review changes" form.
pub struct Submit {
    pub event: ReviewEvent,
    pub body: String,
    /// Put the cursor in the summary when the form opens.
    focus: bool,
}

/// What a background load found (see `Msg::ReviewLoaded`).
pub struct Loaded {
    /// The revisions compared (a PR's base is its remote branch; its head
    /// `refs/margin/pr/<n>`).
    pub base_ref: String,
    pub head_ref: String,
    pub head_oid: String,
    pub files: Vec<FileDiff>,
    pub truncated: bool,
    pub commits: Vec<Commit>,
    pub detail: Option<PrDetail>,
}

pub struct Review {
    /// What was asked for: branch names (base None: the default branch), or a PR.
    pub base: Option<String>,
    pub head: String,
    pub pr: Option<u64>,
    /// Filled in once loaded.
    pub base_ref: String,
    pub head_ref: String,
    pub head_oid: String,
    pub detail: Option<PrDetail>,
    pub tab: Tab,
    pub diff: DiffView,
    pub commits: Vec<Commit>,
    /// Commits tab: the commit whose diff is shown.
    pub commit_sel: Option<String>,
    pub commit_diff: DiffView,
    pub loading: bool,
    pub error: Option<String>,
    /// Results for an older request are dropped.
    pub generation: u64,
    viewed: Viewed,
    /// Per file: a fingerprint of its changes (Viewed resets when it differs).
    fingerprints: Vec<String>,
    common_dir: PathBuf,
    /// Line comments waiting for the review to be submitted.
    pub pending: Vec<DraftComment>,
    pub submit: Option<Submit>,
    pub submitting: bool,
}

impl Review {
    pub fn new(base: Option<String>, head: String, pr: Option<u64>, common_dir: &Path, generation: u64) -> Review {
        let mut diff = DiffView::default();
        diff.loading = true;
        Review {
            base,
            head,
            pr,
            base_ref: String::new(),
            head_ref: String::new(),
            head_oid: String::new(),
            detail: None,
            tab: Tab::Files,
            diff,
            commits: vec![],
            commit_sel: None,
            commit_diff: DiffView::default(),
            loading: true,
            error: None,
            generation,
            viewed: Viewed::load(common_dir),
            fingerprints: vec![],
            common_dir: common_dir.to_path_buf(),
            pending: vec![],
            submit: None,
            submitting: false,
        }
    }

    /// The Viewed marks' key: the PR, or the two revisions compared.
    fn key(&self) -> String {
        review::key(self.pr, &self.base_ref, &self.head_ref)
    }

    pub fn loaded(&mut self, l: Loaded) {
        self.loading = false;
        self.error = None;
        self.base_ref = l.base_ref;
        self.head_ref = l.head_ref;
        self.head_oid = l.head_oid;
        self.commits = l.commits;
        self.fingerprints = l.files.iter().map(file_fingerprint).collect();
        self.diff.set(l.files, l.truncated);
        if l.detail.is_some() {
            self.detail = l.detail;
        }
        // files you've viewed (with these same changes) start folded, as on GitHub
        let key = self.key();
        for fi in 0..self.fingerprints.len() {
            let path = self.diff.files[fi].display_path().to_string();
            if self.viewed.is_viewed(&key, &path, &self.fingerprints[fi]) {
                self.diff.set_file_open(fi, false);
            }
        }
        self.update_threads();
    }

    pub fn failed(&mut self, e: String) {
        self.loading = false;
        self.diff.set_error(e.clone());
        self.error = Some(e);
    }

    /// A newer copy of the PR (after replying / resolving / submitting).
    pub fn set_detail(&mut self, d: PrDetail) {
        self.detail = Some(d);
        self.update_threads();
    }

    fn is_viewed(&self, fi: usize) -> bool {
        let Some(fp) = self.fingerprints.get(fi) else { return false };
        self.viewed.is_viewed(&self.key(), self.diff.files[fi].display_path(), fp)
    }

    fn set_viewed(&mut self, fi: usize, viewed: bool) {
        let Some(fp) = self.fingerprints.get(fi).cloned() else { return };
        let key = self.key();
        let path = self.diff.files[fi].display_path().to_string();
        self.viewed.set(&key, &path, &fp, viewed);
        let _ = self.viewed.save(&self.common_dir);
        self.diff.set_file_open(fi, !viewed);
    }

    /// The PR's open threads at their current line, and the pending comments.
    pub fn update_threads(&mut self) {
        let mut out: Vec<InlineThread> = vec![];
        let n = self.pr.unwrap_or(0);
        if let Some(d) = &self.detail {
            for t in d.threads.iter().filter(|t| !t.outdated) {
                let Some(line) = t.line else { continue };
                out.push(InlineThread {
                    id: t.id.clone(),
                    branch: String::new(),
                    path: t.path.clone(),
                    line,
                    old_side: t.old_side,
                    resolved: t.resolved,
                    outdated: false,
                    pr: n,
                    comments: t.comments.iter().map(|c| (c.author.clone(), crate::emoji::render(&c.body).into_owned(), c.created_at.chars().take(10).collect(), c.url.clone())).collect(),
                });
            }
        }
        for (i, c) in self.pending.iter().enumerate() {
            out.push(InlineThread {
                id: format!("{PENDING}{i}"),
                branch: String::new(),
                path: c.path.clone(),
                line: c.line,
                old_side: c.old_side,
                resolved: false,
                outdated: false,
                pr: n,
                comments: vec![("You".into(), crate::emoji::render(&c.body).into_owned(), "pending".into(), String::new())],
            });
        }
        self.diff.set_threads(out);
        // any line takes a comment on an open pull request
        let open = self.detail.as_ref().is_some_and(|d| d.summary.state == "OPEN" && !d.id.is_empty());
        self.diff.set_comment_target(open.then(|| ("review".to_string(), n)));
    }

    /// The diff's events: threads go to GitHub, new comments wait in the review.
    fn on_diff_events(&mut self, events: Vec<DiffEvent>, actions: &mut Vec<Action>) {
        for e in events {
            match e {
                DiffEvent::OpenUrl(u) => actions.push(Action::OpenUrl(u)),
                DiffEvent::CopyPath(p) => actions.push(Action::Copy(p)),
                DiffEvent::ThreadReply { id, body, .. } => actions.push(Action::ReviewThread { id, reply: Some(body), resolve: None }),
                DiffEvent::ThreadResolve { id, resolve, .. } => actions.push(Action::ReviewThread { id, reply: None, resolve: Some(resolve) }),
                DiffEvent::ThreadNew { path, line, old_side, body, .. } => {
                    self.pending.push(DraftComment { path, line, old_side, body });
                    self.diff.thread_done(DRAFT, true);
                    self.update_threads();
                }
                DiffEvent::PendingRemove { id } => {
                    if let Some(i) = id.strip_prefix(PENDING).and_then(|i| i.parse::<usize>().ok()).filter(|&i| i < self.pending.len()) {
                        self.pending.remove(i);
                        self.update_threads();
                    }
                }
                // (nothing to stage in a comparison; focus mode is the history's)
                DiffEvent::Hunk(_) | DiffEvent::Stage(_) | DiffEvent::Unstage(_) | DiffEvent::Resolve(_) | DiffEvent::Stash(_) | DiffEvent::Discard(_) | DiffEvent::ToggleFocus => {}
            }
        }
    }
}

/// Fingerprint of a file's changes (its hunks), for Viewed marks.
fn file_fingerprint(f: &FileDiff) -> String {
    let mut s = String::new();
    for h in &f.hunks {
        s.push_str(&h.header);
        s.push('\n');
        for l in &h.lines {
            s.push_str(&l.text);
            s.push('\n');
        }
    }
    review::fingerprint(&s)
}

/// Review mode (`rv.review` is Some): header, tabs, and the tab's content.
pub fn show(ui: &mut Ui, rv: &mut RepoView, prefs: &mut DiffPrefs, actions: &mut Vec<Action>) {
    // branches to compare (local first, then remote)
    let branches: Vec<String> = rv
        .snap
        .as_ref()
        .map(|s| {
            let mut v: Vec<String> = s.refs.iter().filter(|r| r.kind == margin_core::git::RefKind::Local).map(|r| r.name.clone()).collect();
            v.extend(s.refs.iter().filter(|r| r.kind == margin_core::git::RefKind::Remote && !r.name.ends_with("/HEAD")).map(|r| r.name.clone()));
            v
        })
        .unwrap_or_default();
    let Some(r) = rv.review.as_mut() else { return };

    egui::Panel::top("review_head").show(ui, |ui| {
        ui.add_space(4.0);
        header(ui, r, &branches, actions);
        ui.add_space(4.0);
    });

    match r.tab {
        Tab::Files => files_tab(ui, r, prefs, actions),
        Tab::Commits => commits_tab(ui, r, prefs, actions),
        Tab::Conversation => {
            egui::CentralPanel::default().show(ui, |ui| conversation_tab(ui, r, actions));
        }
    }
    submit_form(ui.ctx(), r, actions);
}

fn header(ui: &mut Ui, r: &mut Review, branches: &[String], actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("‹ History").on_hover_text("Back to the history (Esc)").clicked() || (r.submit.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) && ui.memory(|m| m.focused().is_none())) {
            actions.push(Action::CloseReview);
        }
        ui.add_space(6.0);
        match (&r.detail, r.pr) {
            (Some(d), _) => {
                let sz = ui.text_style_height(&egui::TextStyle::Body);
                ui.add(crate::icons::image(ui.ctx(), crate::icons::Icon::Github, sz, theme::TEXT()));
                let title = RichText::new(format!("#{} {}", d.summary.number, d.summary.title)).strong();
                if ui.add(egui::Label::new(title).sense(egui::Sense::click())).on_hover_text("Open on GitHub").clicked() {
                    actions.push(Action::OpenUrl(d.summary.url.clone()));
                }
                let (state, color) = match (d.summary.state.as_str(), d.summary.draft) {
                    ("MERGED", _) => ("Merged", theme::lane(3)),
                    ("CLOSED", _) => ("Closed", theme::ERROR()),
                    (_, true) => ("Draft", theme::TEXT_DIM()),
                    _ => ("Open", theme::ADDED()),
                };
                ui.label(RichText::new(state).small().strong().color(color));
                ui.label(RichText::new(format!("{} wants to merge {} into {}", d.author, d.head, d.base)).color(theme::TEXT_DIM()));
            }
            (None, Some(n)) => {
                ui.label(RichText::new(format!("Pull request #{n}")).strong());
            }
            (None, None) => {
                ui.label(RichText::new("Compare").strong());
                let base = r.base.clone().unwrap_or_else(|| r.base_ref.clone());
                ui.menu_button(format!("{} ▾", if base.is_empty() { "base" } else { &base }), |ui| {
                    ui.set_max_height(360.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for b in branches {
                            if ui.selectable_label(*b == base, b).clicked() {
                                actions.push(Action::OpenReview { base: Some(b.clone()), head: r.head.clone(), pr: None });
                                ui.close();
                            }
                        }
                    });
                })
                .response
                .on_hover_text("Base: the branch the changes go into");
                ui.label(RichText::new("←").color(theme::TEXT_DIM()));
                ui.menu_button(format!("{} ▾", r.head), |ui| {
                    ui.set_max_height(360.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for b in branches {
                            if ui.selectable_label(*b == r.head, b).clicked() {
                                actions.push(Action::OpenReview { base: Some(base.clone()), head: b.clone(), pr: None });
                                ui.close();
                            }
                        }
                    });
                })
                .response
                .on_hover_text("Compare: the branch with the changes");
            }
        }
        if r.loading {
            ui.spinner();
        }
        // right: Review changes
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let open = r.detail.as_ref().is_some_and(|d| d.summary.state == "OPEN");
            if open {
                let label = if r.pending.is_empty() { "Review changes".to_string() } else { format!("Review changes · {} pending", r.pending.len()) };
                if crate::widgets::primary_button(ui, &label).on_hover_text("Comment, approve, or request changes").clicked() {
                    r.submit.get_or_insert(Submit { event: ReviewEvent::Comment, body: String::new(), focus: true });
                }
            }
        });
    });
    // counts and tabs
    ui.horizontal(|ui| {
        use crate::widgets::Seg;
        let mut segs = vec![
            Seg::new(None, format!("Files changed {}", r.diff.files.len())).tip("What these changes do, file by file"),
            Seg::new(None, format!("Commits {}", r.commits.len())).tip("The commits on the branch"),
        ];
        let tabs = if r.pr.is_some() { vec![Tab::Files, Tab::Commits, Tab::Conversation] } else { vec![Tab::Files, Tab::Commits] };
        if r.pr.is_some() {
            let n = r.detail.as_ref().map_or(0, |d| d.comments.len());
            segs.push(Seg::new(None, format!("Conversation {n}")).tip("Description, checks, reviews and comments"));
        }
        let cur = tabs.iter().position(|t| *t == r.tab);
        if let Some(i) = crate::widgets::segmented(ui, &segs, cur) {
            r.tab = tabs[i];
        }
        ui.add_space(10.0);
        let (a, d) = r.diff.files.iter().fold((0, 0), |(a, d), f| (a + f.added, d + f.removed));
        ui.label(RichText::new(format!("+{a}")).color(theme::ADDED()));
        ui.label(RichText::new(format!("−{d}")).color(theme::REMOVED()));
        if let Some(e) = &r.error {
            ui.colored_label(theme::ERROR(), e);
        }
    });
}

fn files_tab(ui: &mut Ui, r: &mut Review, prefs: &mut DiffPrefs, actions: &mut Vec<Action>) {
    let n = r.diff.files.len();
    let viewed: Vec<bool> = (0..n).map(|fi| r.is_viewed(fi)).collect();
    let top = r.diff.top_file();
    egui::Panel::left("review_files").resizable(true).default_size(260.0).min_size(160.0).max_size(520.0).show(ui, |ui| {
        let done = viewed.iter().filter(|v| **v).count();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Viewed {done}/{n}")).small().color(theme::TEXT_DIM()));
            ui.add(egui::ProgressBar::new(if n == 0 { 0.0 } else { done as f32 / n as f32 }).desired_height(4.0));
        });
        ui.add_space(4.0);
        let mut toggle = None;
        let mut goto = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (fi, f) in r.diff.files.iter().enumerate() {
                let path = f.display_path();
                let (dir, name) = path.rsplit_once('/').map_or(("", path), |(d, n)| (d, n));
                let (block, checked) = entry_block(ui, egui::Id::unique(("review-file", fi)), top == Some(fi), Some(viewed[fi]), |ui| {
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 64.0).max(40.0);
                        let name = if dir.is_empty() { RichText::new(name) } else { RichText::new(name).strong() };
                        ui.allocate_ui_with_layout(egui::vec2(w, ui.spacing().interact_size.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(name).truncate());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("−{}", f.removed)).small().color(theme::REMOVED()));
                            ui.label(RichText::new(format!("+{}", f.added)).small().color(theme::ADDED()));
                        });
                    });
                    if !dir.is_empty() {
                        ui.add(egui::Label::new(RichText::new(dir).small().color(theme::TEXT_DIM())).truncate());
                    }
                });
                if let Some(v) = checked {
                    toggle = Some((fi, v));
                } else if block.on_hover_text(path).clicked() {
                    goto = Some(fi);
                }
            }
        });
        if let Some((fi, v)) = toggle {
            r.set_viewed(fi, v);
        }
        if let Some(fi) = goto {
            r.diff.goto_file(fi);
        }
    });
    egui::CentralPanel::default().show(ui, |ui| {
        if !r.loading && r.error.is_none() && r.diff.files.is_empty() {
            ui.weak("No changes: the branch has nothing that the base doesn't have.");
            return;
        }
        let events = r.diff.show(ui, DiffMode::Commit, "review_diff", prefs, None);
        r.on_diff_events(events, actions);
    });
}

/// One entry of a list (a file, a commit) as a single block: the whole
/// width and height takes the click and shows hover / selection, like the
/// rows of the history. `checkbox`: a Viewed box at the left, which keeps its
/// own clicks. Returns the block's response and the box's new value when it
/// was clicked.
pub(crate) fn entry_block(ui: &mut Ui, id: egui::Id, selected: bool, checkbox: Option<bool>, content: impl FnOnce(&mut Ui)) -> (egui::Response, Option<bool>) {
    let pad = egui::vec2(6.0, 4.0);
    let w = ui.available_width();
    let left = if checkbox.is_some() { ui.spacing().interact_size.y + 2.0 } else { 0.0 };
    let top = ui.cursor().min;
    // The block is registered before its content (a widget added later wins
    // the click, and the checkbox must keep its own), with last frame's height.
    let h_id = id.with("h");
    let h = ui.data(|d| d.get_temp::<f32>(h_id)).unwrap_or(ui.spacing().interact_size.y + pad.y * 2.0);
    let block = ui.interact(egui::Rect::from_min_size(top, egui::vec2(w, h)), id, egui::Sense::click());
    // a slot behind the content for the hover / selection fill
    let bg = ui.painter().add(egui::Shape::Noop);
    let mut checked = None;
    let inner = ui
        .scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(top + pad, egui::vec2(w - pad.x * 2.0, f32::INFINITY))), |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if let Some(mut v) = checkbox
                    && ui.checkbox(&mut v, "").on_hover_text("Viewed: fold this file; it comes back if it changes").changed()
                {
                    checked = Some(v);
                }
                ui.vertical(|ui| {
                    ui.set_width((w - pad.x * 2.0 - left).max(40.0));
                    content(ui);
                });
            });
        })
        .response;
    let h_now = inner.rect.height() + pad.y * 2.0;
    if (h_now - h).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(h_id, h_now));
        ui.ctx().request_repaint();
    }
    let rect = egui::Rect::from_min_size(top, egui::vec2(w, h_now));
    ui.advance_cursor_after_rect(rect);
    let fill = if selected {
        Some(theme::SELECTED())
    } else if block.hovered() {
        Some(theme::HOVER())
    } else {
        None
    };
    if let Some(c) = fill {
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 4.0, c));
    }
    if block.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.add_space(2.0);
    (block, checked)
}

fn commits_tab(ui: &mut Ui, r: &mut Review, prefs: &mut DiffPrefs, actions: &mut Vec<Action>) {
    egui::Panel::left("review_commits").resizable(true).default_size(360.0).min_size(220.0).max_size(700.0).show(ui, |ui| {
        let now = margin_core::time_now();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if r.commits.is_empty() && !r.loading {
                ui.weak("No commits.");
            }
            for c in &r.commits {
                let sel = r.commit_sel.as_deref() == Some(c.id.as_str());
                let (block, _) = entry_block(ui, egui::Id::unique(("review-commit", c.id.as_str())), sel, None, |ui| {
                    ui.add(egui::Label::new(RichText::new(&c.subject)).truncate());
                    ui.add(egui::Label::new(RichText::new(format!("{} · {} · {}", c.short_id(), c.author, crate::commit_list::rel_time(now, c.time))).small().color(theme::TEXT_DIM())).truncate());
                });
                if block.clicked() {
                    actions.push(Action::ReviewOpenCommit(c.id.clone()));
                }
            }
        });
    });
    egui::CentralPanel::default().show(ui, |ui| {
        if r.commit_sel.is_none() {
            ui.weak("Choose a commit to see its changes.");
            return;
        }
        let events = r.commit_diff.show(ui, DiffMode::Commit, "review_commit_diff", prefs, None);
        for e in events {
            match e {
                DiffEvent::OpenUrl(u) => actions.push(Action::OpenUrl(u)),
                DiffEvent::CopyPath(p) => actions.push(Action::Copy(p)),
                _ => {}
            }
        }
    });
}

fn conversation_tab(ui: &mut Ui, r: &mut Review, actions: &mut Vec<Action>) {
    let Some(d) = &r.detail else {
        ui.weak("Loading the pull request…");
        return;
    };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(880.0);
        // description
        ui.label(RichText::new(format!("{} opened this · +{} −{} in {} files", d.author, d.additions, d.deletions, d.changed_files)).color(theme::TEXT_DIM()));
        ui.add_space(6.0);
        egui::Frame::new().fill(theme::BG_DARK()).corner_radius(6.0).inner_margin(10.0).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if d.body.trim().is_empty() {
                ui.weak("No description.");
            } else if let Some(crate::markdown_view::MdAction::Link(u)) = crate::markdown_view::render(ui, &crate::emoji::render(&d.body)) {
                actions.push(Action::OpenUrl(u));
            }
        });
        // checks
        if !d.checks.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("Checks").strong());
            for c in &d.checks {
                ui.horizontal(|ui| {
                    let (mark, color) = match c.state {
                        margin_core::github::CiState::Success => ("✓", theme::ADDED()),
                        margin_core::github::CiState::Failure => ("✗", theme::ERROR()),
                        _ => ("●", theme::lane(1)),
                    };
                    ui.label(RichText::new(mark).color(color));
                    if let Some(u) = &c.url {
                        if ui.link(&c.name).clicked() {
                            actions.push(Action::OpenUrl(u.clone()));
                        }
                    } else {
                        ui.label(&c.name);
                    }
                    ui.label(RichText::new(c.detail.to_lowercase().replace('_', " ")).small().color(theme::TEXT_DIM()));
                });
            }
        }
        // reviews and comments, oldest first like the GitHub page
        ui.add_space(12.0);
        ui.label(RichText::new("Activity").strong());
        for c in d.comments.iter().rev() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&c.author).strong());
                let what = match &c.kind {
                    CommentKind::Comment => "commented".to_string(),
                    CommentKind::Review { state } => match state.as_str() {
                        "APPROVED" => "approved".into(),
                        "CHANGES_REQUESTED" => "requested changes".into(),
                        _ => "reviewed".into(),
                    },
                    CommentKind::Thread { path, line, .. } => format!("commented on {path}{}", line.map(|l| format!(":{l}")).unwrap_or_default()),
                };
                ui.label(RichText::new(what).color(theme::TEXT_DIM()));
                ui.label(RichText::new(c.created_at.chars().take(10).collect::<String>()).small().color(theme::TEXT_DIM()));
                if !c.url.is_empty() && ui.link(RichText::new("↗").small()).on_hover_text("Open on GitHub").clicked() {
                    actions.push(Action::OpenUrl(c.url.clone()));
                }
            });
            if !c.body.trim().is_empty() {
                egui::Frame::new().stroke(egui::Stroke::new(1.0, theme::BG_DARK())).corner_radius(6.0).inner_margin(8.0).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if let Some(crate::markdown_view::MdAction::Link(u)) = crate::markdown_view::render(ui, &crate::emoji::render(&c.body)) {
                        actions.push(Action::OpenUrl(u));
                    }
                });
            }
        }
    });
}

/// "Review changes": a summary and comment / approve / request changes.
fn submit_form(ctx: &egui::Context, r: &mut Review, actions: &mut Vec<Action>) {
    let Some(s) = &mut r.submit else { return };
    let mut close = false;
    let mut send = false;
    egui::Modal::new(egui::Id::unique("review_submit")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new("Review changes").strong());
        ui.add_space(6.0);
        let summary = ui.add(egui::TextEdit::multiline(&mut s.body).hint_text("Leave a comment (Markdown)").desired_rows(5).desired_width(f32::INFINITY));
        if std::mem::take(&mut s.focus) {
            summary.request_focus();
        }
        ui.add_space(6.0);
        ui.radio_value(&mut s.event, ReviewEvent::Comment, "Comment").on_hover_text("Feedback without approving");
        ui.radio_value(&mut s.event, ReviewEvent::Approve, "Approve").on_hover_text("Ready to merge");
        ui.radio_value(&mut s.event, ReviewEvent::RequestChanges, "Request changes").on_hover_text("Must be addressed before merging");
        ui.add_space(4.0);
        let n = r.pending.len();
        if n > 0 {
            ui.label(RichText::new(format!("{n} line comment{} will be sent with it.", if n == 1 { "" } else { "s" })).color(theme::TEXT_DIM()));
        }
        // GitHub needs a summary to request changes, and something to comment
        let missing = match s.event {
            ReviewEvent::RequestChanges => s.body.trim().is_empty().then_some("Say what needs to change."),
            ReviewEvent::Comment => (s.body.trim().is_empty() && n == 0).then_some("Write a comment, or comment on lines first."),
            ReviewEvent::Approve => None,
        };
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let ready = missing.is_none() && !r.submitting;
            if ui.add_enabled(ready, egui::Button::new(if r.submitting { "Submitting…" } else { "Submit review" })).clicked() {
                send = true;
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            if let Some(m) = missing {
                ui.label(RichText::new(m).small().color(theme::TEXT_DIM()));
            }
        });
    });
    if send {
        actions.push(Action::ReviewSubmit { event: s.event, body: s.body.clone() });
    }
    if close && !r.submitting {
        r.submit = None;
    }
}

/// The remote GitHub pull requests come from: `origin`, else the first one
/// (the same choice as `GitHub::for_repo`).
pub fn github_remote(repo: &margin_core::git::Repo) -> String {
    let remotes = repo.remotes();
    if remotes.iter().any(|r| r == "origin") { "origin".into() } else { remotes.first().cloned().unwrap_or_else(|| "origin".into()) }
}

/// Load a comparison (background). A pull request: its details, its head
/// fetched into `refs/margin/pr/<n>`, its base branch fresh from the remote.
pub fn load(
    repo: &margin_core::git::Repo,
    gh: Option<&margin_core::github::GitHub>,
    base: Option<String>,
    head: String,
    pr: Option<u64>,
    ignore_ws: bool,
) -> margin_core::Result<Loaded> {
    let mut detail = None;
    let (base_ref, head_ref) = match pr {
        Some(n) => {
            let gh = gh.ok_or_else(|| margin_core::Error("GitHub isn't connected (Settings → Accounts)".into()))?;
            let d = gh.pr_by_number(n)?.ok_or_else(|| margin_core::Error(format!("pull request #{n} wasn't found")))?;
            let remote = github_remote(repo);
            repo.fetch_pr(&remote, n)?;
            // the base as it is on GitHub (a stale local copy would show others' merged work)
            let base_ref = match repo.fetch_branch(&remote, &d.base) {
                Ok(()) => format!("{remote}/{}", d.base),
                Err(_) => d.base.clone(),
            };
            detail = Some(d);
            (base_ref, format!("refs/margin/pr/{n}"))
        }
        None => {
            let base = base.or_else(|| repo.default_base()).ok_or_else(|| margin_core::Error("choose a base branch to compare with".into()))?;
            (base, head)
        }
    };
    let head_oid = repo.resolve(&head_ref)?;
    let (files, truncated) = margin_core::diff::parse(&repo.range_diff(&base_ref, &head_ref, ignore_ws)?);
    let commits = repo.range_commits(&base_ref, &head_ref, 500)?;
    Ok(Loaded { base_ref, head_ref, head_oid, files, truncated, commits, detail })
}
