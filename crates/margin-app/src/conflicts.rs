//! The conflict editor: resolve merge (rebase, cherry-pick, revert, stash)
//! conflicts in Margin. Each conflict shows the current and the incoming
//! side next to each other; keep one, both, or write your own, then mark the
//! file resolved. Files one side deleted get keep / delete. When every file
//! is resolved, continue (or abort) the operation from the header.

use crate::app::{Action, RepoView};
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::conflict::{self, Choice, Kind, Part, Side};
use std::sync::Arc;
use std::time::SystemTime;

pub struct Conflicts {
    /// The file being resolved.
    pub path: Option<String>,
    loaded: Option<Loaded>,
    /// Files resolved since the editor opened (shown ticked in the list).
    resolved: Vec<String>,
    /// A git step for `path` was asked for: show it as busy, reload after.
    /// (With a time limit: a step refused while another runs never reports back.)
    waiting: Option<std::time::Instant>,
    /// Continue / abort was asked: close when the operation is over.
    finishing: bool,
}

struct Loaded {
    path: String,
    mtime: Option<SystemTime>,
    body: Body,
}

enum Body {
    /// The usual case: text with conflict markers.
    Parts {
        parts: Vec<Part>,
        choices: Vec<Option<Choice>>,
        /// The conflict being written by hand, and its text.
        editing: Option<(usize, String)>,
        /// Text runs shown in full (long ones are folded).
        expanded: Vec<usize>,
        /// Conflicts showing their common ancestor.
        show_base: Vec<usize>,
        scroll_to: Option<usize>,
    },
    /// No markers (edited outside Margin, or resolved by hand).
    NoMarkers,
    Binary,
    /// One side deleted the file (or only one has it): keep or delete it.
    /// The version left in the working directory, if it's text.
    OneSide(Option<String>),
}

/// How long a run of unchanged text may be before it's folded.
const FOLD: usize = 8;
const KEEP: usize = 3;

impl Conflicts {
    pub fn new(path: Option<String>) -> Conflicts {
        Conflicts { path, loaded: None, resolved: vec![], waiting: None, finishing: false }
    }

    /// The repository was refreshed (after a git step): reload what changed.
    pub fn on_refresh(&mut self) {
        if self.waiting.take().is_some() {
            self.loaded = None;
        }
    }

    /// Close once continue / abort has finished the operation.
    fn waiting(&self) -> bool {
        self.waiting.is_some_and(|t| t.elapsed().as_secs() < 15)
    }

    pub fn done(&self, pending_op: Option<&str>, conflicted: usize) -> bool {
        self.finishing && pending_op.is_none() && conflicted == 0
    }
}

/// The two sides' names: in a rebase "ours" is the branch being rebased onto.
fn side_names(op: Option<&str>) -> (&'static str, &'static str) {
    match op {
        Some("rebase") => ("Rebasing onto", "Your commit"),
        _ => ("Current", "Incoming"),
    }
}

fn kind_text(k: Kind, op: Option<&str>) -> &'static str {
    let rebase = op == Some("rebase");
    match k {
        Kind::Both => "both changed",
        Kind::DeletedByUs if rebase => "deleted upstream",
        Kind::DeletedByUs => "deleted on current",
        Kind::DeletedByThem if rebase => "deleted by your commit",
        Kind::DeletedByThem => "deleted on incoming",
        Kind::BothDeleted => "deleted on both",
        Kind::AddedByUs => "only on current",
        Kind::AddedByThem => "only on incoming",
    }
}

fn git(label: &str, f: impl Fn(&margin_core::git::Repo) -> margin_core::Result<String> + Send + Sync + 'static) -> Action {
    Action::RunGit { label: label.to_string(), f: Arc::new(f) }
}

pub fn show(ui: &mut Ui, rv: &mut RepoView, actions: &mut Vec<Action>) {
    let op = rv.snap.as_ref().and_then(|s| s.pending_op);
    let files: Vec<(String, Kind)> = rv
        .snap
        .as_ref()
        .map(|s| s.status.files.iter().filter(|f| f.conflicted).map(|f| (f.path.clone(), conflict::kind(f.index, f.worktree))).collect())
        .unwrap_or_default();
    let repo = rv.repo.clone();
    let Some(c) = rv.conflicts.as_mut() else { return };

    // the chosen file stays chosen until it's resolved; then the next one
    if !c.waiting() && c.path.as_ref().is_none_or(|p| !files.iter().any(|(f, _)| f == p)) {
        if let Some(p) = c.path.take().filter(|p| !c.resolved.contains(p)) {
            c.resolved.push(p);
        }
        c.path = files.first().map(|(p, _)| p.clone());
        c.loaded = None;
    }
    c.resolved.retain(|p| !files.iter().any(|(f, _)| f == p));
    let kind = c.path.as_ref().and_then(|p| files.iter().find(|(f, _)| f == p)).map(|(_, k)| *k).unwrap_or(Kind::Both);

    // (re)load the file: first time, after a git step, or when it changed on disk
    if let Some(p) = c.path.clone().filter(|_| !c.waiting()) {
        let mtime = std::fs::metadata(repo.workdir.join(&p)).and_then(|m| m.modified()).ok();
        if c.loaded.as_ref().is_none_or(|l| l.path != p || l.mtime != mtime) {
            c.loaded = Some(load(&repo, &p, kind, mtime));
        }
    }

    egui::Panel::top("conflict_head").show(ui, |ui| {
        ui.add_space(4.0);
        header(ui, c, op, files.len(), actions);
        ui.add_space(4.0);
    });

    egui::Panel::left("conflict_files").resizable(true).default_size(260.0).min_size(160.0).max_size(520.0).show(ui, |ui| {
        ui.add_space(6.0);
        let total = files.len() + c.resolved.len();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Resolved {}/{total}", c.resolved.len())).small().color(theme::TEXT_DIM()));
            ui.add(egui::ProgressBar::new(if total == 0 { 1.0 } else { c.resolved.len() as f32 / total as f32 }).desired_height(4.0));
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (p, k) in &files {
                let (dir, name) = p.rsplit_once('/').map_or(("", p.as_str()), |(d, n)| (d, n));
                let sel = c.path.as_deref() == Some(p.as_str());
                let (block, _) = crate::review::entry_block(ui, egui::Id::unique(("conflict-file", p.as_str())), sel, None, |ui| {
                    ui.horizontal(|ui| {
                        let sz = ui.text_style_height(&egui::TextStyle::Body) - 2.0;
                        ui.add(crate::icons::image(ui.ctx(), crate::icons::Icon::Alert, sz, theme::ERROR()));
                        ui.add(egui::Label::new(if dir.is_empty() { RichText::new(name) } else { RichText::new(name).strong() }).truncate());
                    });
                    let mut sub = kind_text(*k, op).to_string();
                    if !dir.is_empty() {
                        sub = format!("{dir} · {sub}");
                    }
                    ui.add(egui::Label::new(RichText::new(sub).small().color(theme::TEXT_DIM())).truncate());
                });
                if block.on_hover_text(p).clicked() && !sel {
                    c.path = Some(p.clone());
                    c.loaded = None;
                }
            }
            if !c.resolved.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("RESOLVED").small().color(theme::TEXT_DIM()));
                for p in &c.resolved {
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new("✓").color(theme::ADDED()));
                        ui.add(egui::Label::new(RichText::new(p).color(theme::TEXT_DIM())).truncate());
                    });
                }
            }
        });
    });

    egui::CentralPanel::default().show(ui, |ui| {
        let Some(path) = c.path.clone() else {
            all_done(ui, c, op, actions);
            return;
        };
        if c.waiting() || c.loaded.is_none() {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Saving…");
            });
            return;
        }
        file_view(ui, c, &repo.workdir, &path, kind, op, actions);
    });
}

fn load(repo: &margin_core::git::Repo, path: &str, kind: Kind, mtime: Option<SystemTime>) -> Loaded {
    let file = repo.conflict_file(path);
    if kind != Kind::Both {
        let text = file.ok().flatten().filter(|b| !conflict::is_binary(b)).map(|b| String::from_utf8_lossy(&b).into_owned());
        return Loaded { path: path.to_string(), mtime, body: Body::OneSide(text) };
    }
    let body = match file {
        Ok(None) | Err(_) => Body::NoMarkers,
        Ok(Some(b)) if conflict::is_binary(&b) => Body::Binary,
        Ok(Some(b)) => match conflict::parse(&String::from_utf8_lossy(&b)) {
            Some(parts) => {
                let n = parts.iter().filter(|p| matches!(p, Part::Conflict(_))).count();
                Body::Parts { parts, choices: vec![None; n], editing: None, expanded: vec![], show_base: vec![], scroll_to: None }
            }
            None => Body::NoMarkers,
        },
    };
    Loaded { path: path.to_string(), mtime, body }
}

fn header(ui: &mut Ui, c: &mut Conflicts, op: Option<&str>, left: usize, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        let editing = matches!(c.loaded.as_ref().map(|l| &l.body), Some(Body::Parts { editing: Some(_), .. }));
        if ui.button("‹ History").on_hover_text("Back to the history (Esc). Your choices in this file are kept until you switch files.").clicked()
            || (!editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) && ui.memory(|m| m.focused().is_none()))
        {
            actions.push(Action::CloseConflicts);
        }
        ui.add_space(6.0);
        let sz = ui.text_style_height(&egui::TextStyle::Body);
        ui.add(crate::icons::image(ui.ctx(), crate::icons::Icon::Alert, sz, theme::ERROR()));
        let title = match op {
            Some(op) => format!("Resolve conflicts · {op} in progress"),
            None => "Resolve conflicts".to_string(),
        };
        ui.label(RichText::new(title).strong());
        ui.label(
            RichText::new(match left {
                0 => "all files resolved".to_string(),
                1 => "1 file left".to_string(),
                n => format!("{n} files left"),
            })
            .color(theme::TEXT_DIM()),
        );
        let Some(op) = op else { return };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let verb = match op {
                "merge" => "Commit merge",
                _ => "Continue",
            };
            let cont = ui.add_enabled(left == 0, egui::Button::new(RichText::new(verb).strong()).fill(if left == 0 { theme::ACCENT() } else { theme::BADGE() }));
            let cont = if left == 0 {
                cont.on_hover_text(match op {
                    "merge" => "Commit the merge with its usual message".to_string(),
                    "rebase" => "Go on with the rebase (the next commits may stop again)".to_string(),
                    _ => format!("Finish the {op}"),
                })
            } else {
                cont.on_disabled_hover_text("Resolve every file first")
            };
            if cont.clicked() {
                let op = op.to_string();
                c.finishing = true;
                actions.push(git(&format!("Continue {op}"), move |r| r.operation(&op, "continue")));
            }
            if op == "rebase" && ui.button("Skip commit").on_hover_text("Leave this commit out and go on with the rebase").clicked() {
                actions.push(Action::Confirm {
                    msg: "Skip this commit? Its changes are left out of the rebased branch.".into(),
                    action: Box::new(git("Skip commit", |r| r.operation("rebase", "skip"))),
                });
            }
            if ui.button(format!("Abort {op}")).on_hover_text(format!("Stop the {op} and put everything back as it was before it")).clicked() {
                let o = op.to_string();
                c.finishing = true;
                actions.push(Action::Confirm {
                    msg: format!("Abort the {op}? Conflicts you resolved are dropped and the branch goes back to how it was."),
                    action: Box::new(git(&format!("Abort {op}"), move |r| r.operation(&o, "abort"))),
                });
            }
        });
    });
}

fn all_done(ui: &mut Ui, c: &mut Conflicts, op: Option<&str>, actions: &mut Vec<Action>) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("✓").size(28.0).color(theme::ADDED()));
        ui.label(RichText::new("No conflicts left").heading());
        ui.add_space(4.0);
        match op {
            Some("merge") => {
                ui.weak("Commit the merge to finish it.");
            }
            Some(op) => {
                ui.weak(format!("Continue the {op} to finish it."));
            }
            None => {
                ui.weak("Review the changes and commit them when you're ready.");
                ui.add_space(8.0);
                if ui.button("Back to the history").clicked() {
                    actions.push(Action::CloseConflicts);
                }
            }
        }
        let _ = c;
    });
}

fn file_view(ui: &mut Ui, c: &mut Conflicts, workdir: &std::path::Path, path: &str, kind: Kind, op: Option<&str>, actions: &mut Vec<Action>) {
    let (cur, inc) = side_names(op);
    let Some(l) = c.loaded.as_mut() else { return };
    let mut wait = false;
    let p = path.to_string();

    // file bar: name, whole-file choices, mark resolved
    let all_chosen = matches!(&l.body, Body::Parts { choices, editing: None, .. } if choices.iter().all(|c| c.is_some()));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(path).strong());
        ui.label(RichText::new(kind_text(kind, op)).small().color(theme::TEXT_DIM()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Body::Parts { parts, choices, .. } = &l.body {
                let b = ui.add_enabled(all_chosen, egui::Button::new(RichText::new("Mark resolved").strong()).fill(if all_chosen { theme::ACCENT() } else { theme::BADGE() }));
                let b = if all_chosen { b.on_hover_text("Save the file and stage it") } else { b.on_disabled_hover_text("Choose what to keep for every conflict first") };
                if b.clicked() {
                    let text = conflict::assemble(parts, choices);
                    let p = p.clone();
                    actions.push(git("Resolve", move |r| r.conflict_resolve(&p, &text).map(|_| String::new())));
                    wait = true;
                }
            }
            ui.menu_button("Whole file ▾", |ui| {
                if kind != Kind::BothDeleted {
                    let take = |side: Side| {
                        let p = p.clone();
                        git("Resolve", move |r| r.conflict_take(&p, side, kind).map(|_| String::new()))
                    };
                    if ui.button(format!("Use {} version", cur.to_lowercase())).clicked() {
                        actions.push(take(Side::Ours));
                        wait = true;
                        ui.close();
                    }
                    if ui.button(format!("Use {} version", inc.to_lowercase())).clicked() {
                        actions.push(take(Side::Theirs));
                        wait = true;
                        ui.close();
                    }
                    ui.separator();
                }
                if kind == Kind::Both && ui.button("Start over").on_hover_text("Put the conflict markers back as git wrote them (with the common ancestor), dropping edits").clicked() {
                    let p = p.clone();
                    actions.push(Action::Confirm {
                        msg: format!("Start {p} over? Edits to the file since the conflict are dropped."),
                        action: Box::new(git("Start over", move |r| r.conflict_restart(&p).map(|_| String::new()))),
                    });
                    ui.close();
                }
                if ui.button("Mark resolved as it is").on_hover_text("Stage the file as it is now in the working directory").clicked() {
                    let p = p.clone();
                    actions.push(git("Resolve", move |r| r.stage(&[&p]).map(|_| String::new())));
                    wait = true;
                    ui.close();
                }
                if !matches!(l.body, Body::OneSide(None)) && ui.button("Open in default app").clicked() {
                    actions.push(Action::OpenPath { path: workdir.join(&p), terminal: false });
                    ui.close();
                }
            });
        });
    });
    ui.add_space(4.0);

    match &mut l.body {
        Body::Parts { parts, choices, editing, expanded, show_base, scroll_to } => {
            let n = choices.len();
            let done = choices.iter().filter(|c| c.is_some()).count();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{done} of {n} conflict{} resolved", if n == 1 { "" } else { "s" })).small().color(theme::TEXT_DIM()));
                if done < n && ui.small_button("Next unresolved ↓").clicked() {
                    *scroll_to = choices.iter().position(|c| c.is_none());
                }
            });
            ui.add_space(4.0);
            let lang = crate::syntax::for_path(path);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut ci = 0;
                for (pi, part) in parts.iter().enumerate() {
                    match part {
                        Part::Text(t) => text_run(ui, t, lang, pi, expanded),
                        Part::Conflict(h) => {
                            let r = conflict_card(ui, ci, h, &mut choices[ci], editing, show_base, lang, (cur, inc));
                            if *scroll_to == Some(ci) {
                                ui.scroll_to_rect(r, Some(egui::Align::TOP));
                                *scroll_to = None;
                            }
                            ci += 1;
                        }
                    }
                }
                ui.add_space(40.0);
            });
        }
        Body::NoMarkers => {
            ui.add_space(10.0);
            ui.label("This file has no conflict markers left (edited outside Margin?).");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Mark resolved as it is").strong()).clicked() {
                    actions.push(git("Resolve", move |r| r.stage(&[&p]).map(|_| String::new())));
                    wait = true;
                }
                if kind == Kind::Both && ui.button("Start over").clicked() {
                    let p = path.to_string();
                    actions.push(git("Start over", move |r| r.conflict_restart(&p).map(|_| String::new())));
                    wait = true;
                }
            });
        }
        Body::Binary | Body::OneSide(_) => {
            ui.add_space(10.0);
            let msg = match kind {
                Kind::DeletedByUs if op == Some("rebase") => "The branch you're rebasing onto deleted this file; your commit changed it.".to_string(),
                Kind::DeletedByUs => format!("The {} side deleted this file; the {} side changed it.", cur.to_lowercase(), inc.to_lowercase()),
                Kind::DeletedByThem if op == Some("rebase") => "Your commit deleted this file; the branch you're rebasing onto changed it.".to_string(),
                Kind::DeletedByThem => format!("The {} side deleted this file; the {} side changed it.", inc.to_lowercase(), cur.to_lowercase()),
                Kind::BothDeleted => "Both sides deleted (or renamed) this file.".to_string(),
                Kind::AddedByUs | Kind::AddedByThem => "Only one side has this file (it may have been renamed on the other).".to_string(),
                Kind::Both => "A binary file both sides changed: keep one version.".to_string(),
            };
            ui.label(msg);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let take = |side: Side| {
                    let p = p.clone();
                    git("Resolve", move |r| r.conflict_take(&p, side, kind).map(|_| String::new()))
                };
                match kind {
                    Kind::Both => {
                        if ui.button(format!("Use {} version", cur.to_lowercase())).clicked() {
                            actions.push(take(Side::Ours));
                            wait = true;
                        }
                        if ui.button(format!("Use {} version", inc.to_lowercase())).clicked() {
                            actions.push(take(Side::Theirs));
                            wait = true;
                        }
                    }
                    Kind::BothDeleted => {}
                    _ => {
                        let keep = match kind {
                            Kind::DeletedByUs | Kind::AddedByThem => Side::Theirs,
                            _ => Side::Ours,
                        };
                        if ui.button(RichText::new("Keep the file").strong()).on_hover_text("Keep the changed version").clicked() {
                            actions.push(take(keep));
                            wait = true;
                        }
                    }
                }
                if kind != Kind::Both && ui.button("Delete it").clicked() {
                    let p = p.clone();
                    actions.push(git("Resolve", move |r| r.conflict_delete(&p).map(|_| String::new())));
                    wait = true;
                }
            });
            if let Body::OneSide(Some(text)) = &l.body {
                let whose = match kind {
                    Kind::DeletedByUs | Kind::AddedByThem => inc,
                    _ => cur,
                };
                ui.add_space(10.0);
                ui.label(RichText::new(format!("The file as it is ({} version)", whose.to_lowercase())).small().color(theme::TEXT_DIM()));
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| code(ui, text, crate::syntax::for_path(path), Some(theme::BG_DARK()), false));
            }
        }
    }
    if wait {
        c.waiting = Some(std::time::Instant::now());
    }
}

/// Text both sides agree on: dim, long runs folded to their ends.
fn text_run(ui: &mut Ui, t: &str, lang: Option<&'static crate::syntax::Lang>, pi: usize, expanded: &mut Vec<usize>) {
    let lines: Vec<&str> = t.split_inclusive('\n').collect();
    if lines.len() <= FOLD || expanded.contains(&pi) {
        code(ui, t, lang, None, true);
        return;
    }
    code(ui, &lines[..KEEP].concat(), lang, None, true);
    let hidden = lines.len() - KEEP * 2;
    let r = ui.add(egui::Button::new(RichText::new(format!("⋯ {hidden} unchanged lines")).small().color(theme::TEXT_DIM())).frame(false));
    if r.on_hover_text("Show them").clicked() {
        expanded.push(pi);
    }
    code(ui, &lines[lines.len() - KEEP..].concat(), lang, None, true);
}

/// Code in the monospace font, coloured by the file's language.
fn code(ui: &mut Ui, text: &str, lang: Option<&'static crate::syntax::Lang>, bg: Option<egui::Color32>, dim: bool) {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let base = if dim { theme::TEXT_DIM() } else { theme::TEXT() };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width() - 12.0;
    let mut st = crate::syntax::State::Normal;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            job.append("\n", 0.0, egui::TextFormat::simple(font.clone(), base));
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        let colors = match lang {
            Some(l) if !dim => crate::syntax::colors(&crate::syntax::line(l, line, &mut st), line.len(), base),
            _ => vec![base; line.len()],
        };
        // runs of one colour
        let mut s = 0;
        for (j, ch) in line.char_indices() {
            if j > s && colors[j] != colors[s] {
                job.append(&line[s..j], 0.0, egui::TextFormat::simple(font.clone(), colors[s]));
                s = j;
            }
            let _ = ch;
        }
        if s < line.len() {
            job.append(&line[s..], 0.0, egui::TextFormat::simple(font.clone(), colors[s]));
        }
    }
    let frame = egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 3)).corner_radius(3.0);
    let frame = if let Some(bg) = bg { frame.fill(bg) } else { frame };
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        // (not justified, which the columns' layout would ask for)
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| ui.add(egui::Label::new(job).selectable(true)));
    });
}

#[allow(clippy::too_many_arguments)]
fn conflict_card(
    ui: &mut Ui,
    ci: usize,
    h: &conflict::Hunk,
    choice: &mut Option<Choice>,
    editing: &mut Option<(usize, String)>,
    show_base: &mut Vec<usize>,
    lang: Option<&'static crate::syntax::Lang>,
    (cur, inc): (&str, &str),
) -> egui::Rect {
    let cur_col = theme::lane(0);
    let inc_col = theme::lane(3);
    let stroke = if choice.is_some() { theme::ADDED().gamma_multiply(0.6) } else { theme::ERROR().gamma_multiply(0.7) };
    ui.add_space(4.0);
    let r = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(5.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // header
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Conflict {}", ci + 1)).strong());
                if let Some(c) = choice.as_ref() {
                    let what = match c {
                        Choice::Ours => format!("kept {}", cur.to_lowercase()),
                        Choice::Theirs => format!("kept {}", inc.to_lowercase()),
                        Choice::OursThenTheirs => format!("kept both, {} first", cur.to_lowercase()),
                        Choice::TheirsThenOurs => format!("kept both, {} first", inc.to_lowercase()),
                        Choice::Base => "kept the common ancestor".into(),
                        Choice::Custom(_) => "edited".into(),
                    };
                    ui.label(RichText::new(format!("✓ {what}")).small().color(theme::ADDED()));
                }
            });
            ui.add_space(4.0);

            // writing it by hand
            if let Some((_, text)) = editing.as_mut().filter(|(ei, _)| *ei == ci) {
                let rows = text.lines().count().clamp(3, 30);
                let id = egui::Id::unique(("conflict-edit", ci));
                let te = ui.add(egui::TextEdit::multiline(text).id(id).code_editor().desired_rows(rows).desired_width(f32::INFINITY).lock_focus(true));
                // focus the box when it opens
                if ui.data(|d| d.get_temp::<bool>(id.with("focused"))).is_none() {
                    te.request_focus();
                    ui.data_mut(|d| d.insert_temp(id.with("focused"), true));
                }
                let mut end = None;
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Done").strong()).clicked() {
                        let mut t = text.clone();
                        if !t.is_empty() && !t.ends_with('\n') {
                            t.push('\n');
                        }
                        end = Some(Some(t));
                    } else if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        end = Some(None);
                    }
                });
                if let Some(done) = end {
                    ui.data_mut(|d| d.remove::<bool>(id.with("focused")));
                    if let Some(t) = done {
                        *choice = Some(Choice::Custom(t));
                    }
                    *editing = None;
                }
                return;
            }

            if let Some(c) = choice.clone() {
                // resolved: the result, and a way back
                let text = h.resolve(&c);
                if text.is_empty() {
                    ui.weak("(nothing: both sides' lines are left out)");
                } else {
                    code(ui, &text, lang, Some(theme::ADDED_BG()), false);
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Change").clicked() {
                        *choice = None;
                    }
                    if ui.small_button("Edit…").clicked() {
                        *editing = Some((ci, text));
                    }
                });
                return;
            }

            // the two sides
            let side = |ui: &mut Ui, name: &str, label: &str, text: &str, col: egui::Color32| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, col);
                    ui.label(RichText::new(name).strong());
                    if !label.is_empty() {
                        ui.add(egui::Label::new(RichText::new(label).small().color(theme::TEXT_DIM())).truncate());
                    }
                });
                if text.is_empty() {
                    ui.weak("(nothing)");
                } else {
                    code(ui, text, lang, Some(col.gamma_multiply(0.12)), false);
                }
            };
            let mut pick = None;
            let wide = ui.available_width() > 640.0;
            if wide {
                ui.columns(2, |cols| {
                    for (i, col) in cols.iter_mut().enumerate() {
                        col.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            let (name, label, text, color, c) = if i == 0 { (cur, &h.ours_label, &h.ours, cur_col, Choice::Ours) } else { (inc, &h.theirs_label, &h.theirs, inc_col, Choice::Theirs) };
                            side(ui, name, label, text, color);
                            if ui.button(format!("Use {}", name.to_lowercase())).clicked() {
                                pick = Some(c);
                            }
                        });
                    }
                });
            } else {
                side(ui, cur, &h.ours_label, &h.ours, cur_col);
                if ui.button(format!("Use {}", cur.to_lowercase())).clicked() {
                    pick = Some(Choice::Ours);
                }
                ui.add_space(4.0);
                side(ui, inc, &h.theirs_label, &h.theirs, inc_col);
                if ui.button(format!("Use {}", inc.to_lowercase())).clicked() {
                    pick = Some(Choice::Theirs);
                }
            }
            if let Some(b) = &h.base
                && show_base.contains(&ci)
            {
                ui.add_space(4.0);
                side(ui, "Common ancestor", &h.base_label, b, theme::TEXT_DIM());
            }
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(format!("Both, {} first", cur.to_lowercase())).clicked() {
                    pick = Some(Choice::OursThenTheirs);
                }
                if ui.button(format!("Both, {} first", inc.to_lowercase())).clicked() {
                    pick = Some(Choice::TheirsThenOurs);
                }
                if ui.button("Edit…").on_hover_text("Write the result yourself (starts from both sides)").clicked() {
                    *editing = Some((ci, h.resolve(&Choice::OursThenTheirs)));
                }
                if let Some(b) = &h.base {
                    let showing = show_base.contains(&ci);
                    if ui.selectable_label(showing, "Common ancestor").on_hover_text("What this part was before either side changed it").clicked() {
                        if showing {
                            show_base.retain(|x| *x != ci);
                        } else {
                            show_base.push(ci);
                        }
                    }
                    if showing && ui.button("Use ancestor").clicked() {
                        pick = Some(Choice::Base);
                    }
                    let _ = b;
                }
            });
            if pick.is_some() {
                *choice = pick;
            }
        })
        .response
        .rect;
    ui.add_space(4.0);
    r
}
