//! Notes panel: Markdown notes for the selected commit, branch, task or repo.
//! Autosaves when valid; detects edits made concurrently by MCP agents.
//!
//! Layout: a grip on top (drag to resize, double-click to collapse), the tab
//! row, then the note. The height is whatever the user dragged it to (it
//! never changes by itself); a collapsed panel shows only the tab row. An
//! empty note shows a one-line "Write note". The trash button deletes a note
//! after a confirmation (it goes to the notes trash folder). Notes open in Preview;
//! Edit and Split (editor + live preview) have a formatting toolbar.

use crate::app::{Action, RepoView, Sel};
use crate::icons::{self, Icon};
use crate::markdown_view::{self, MdAction};
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::markdown;
use margin_core::notes::{NoteStore, NoteTarget};
use margin_core::tasks::Task;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum NoteTab {
    Commit,
    #[default]
    Branch,
    Task,
    Repo,
    /// A note opened explicitly (e.g. from "Detached notes").
    Pinned,
}

#[derive(Default)]
pub struct NoteEditor {
    pub tab: NoteTab,
    pub target: Option<NoteTarget>,
    pub text: String,
    base_version: String,
    dirty: bool,
    preview: bool,
    /// Changed on disk (e.g. by an agent) while we have unsaved edits.
    external: bool,
    error: Option<String>,
    violations: Vec<markdown::Violation>,
    last_edit: f64,
    requested_tasks: HashSet<String>,
    /// Height of the content when the panel is fitted to it (collapsed, or an
    /// empty note); `None` when it uses the dragged height. Read by the layout.
    pub fit_h: Option<f32>,
    /// Last editor selection (char indices), for the formatting toolbar.
    sel: (usize, usize),
}

/// Smallest height for the notes panel when it shows a note.
pub const MIN_H: f32 = 140.0;
pub const DEFAULT_H: f32 = 300.0;

fn edit_id() -> egui::Id {
    egui::Id::unique("margin_note_edit")
}

impl NoteEditor {
    fn load(&mut self, t: NoteTarget, notes: &NoteStore) {
        self.error = None;
        match notes.read(&t) {
            Ok(Some(n)) => {
                self.text = n.content;
                self.base_version = n.version;
            }
            Ok(None) => {
                self.text.clear();
                self.base_version.clear();
            }
            Err(e) => self.error = Some(e.0),
        }
        // Always open in Preview; an empty note shows a one-line "Write note".
        self.preview = true;
        self.sel = (0, 0);
        self.target = Some(t);
        self.dirty = false;
        self.external = false;
        self.violations.clear();
    }

    /// The note was deleted: show it as empty (nothing to save).
    pub fn forget(&mut self) {
        self.text.clear();
        self.base_version.clear();
        self.dirty = false;
        self.external = false;
        self.error = None;
        self.violations.clear();
        self.preview = true;
    }

    pub fn save(&mut self, notes: &NoteStore) -> bool {
        let Some(t) = &self.target else { return true };
        if !self.dirty {
            return true;
        }
        match notes.write(t, &self.text, Some(&self.base_version)) {
            Ok(n) => {
                self.base_version = n.version;
                self.dirty = false;
                self.external = false;
                self.error = None;
                true
            }
            Err(e) => {
                if e.0.starts_with("version conflict") {
                    self.external = true;
                }
                self.error = Some(e.0);
                false
            }
        }
    }

    fn force_save(&mut self, notes: &NoteStore) {
        if let Some(t) = &self.target {
            self.base_version = notes.current_version(t).unwrap_or_default();
            self.external = false;
            self.save(notes);
        }
    }

    pub fn save_if_dirty(&mut self, notes: &NoteStore) {
        if self.dirty {
            self.save(notes);
        }
    }

    pub fn on_external_change(&mut self, notes: &NoteStore) {
        let Some(t) = self.target.clone() else { return };
        let cur = notes.current_version(&t).unwrap_or_default();
        if cur != self.base_version {
            if self.dirty {
                self.external = true;
            } else {
                let preview = self.preview;
                self.load(t, notes);
                self.preview = preview || !self.text.is_empty();
            }
        }
    }

    fn sync(&mut self, desired: Option<NoteTarget>, notes: &NoteStore) {
        if desired == self.target {
            return;
        }
        if self.dirty && !self.save(notes) {
            return; // keep the user on the unsaved note until resolved
        }
        match desired {
            Some(t) => self.load(t, notes),
            None => {
                self.target = None;
                self.text.clear();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    rv: &mut RepoView,
    task_cache: &HashMap<String, Result<Task, String>>,
    tasks_enabled: bool,
    license: &margin_core::license::Status,
    task_url: &dyn Fn(&str) -> Option<String>,
    author: &str,
    prefs: &mut margin_core::config::State,
    panel_h: f32,
    actions: &mut Vec<Action>,
) {
    let pro = license.is_pro();
    let top = ui.cursor().top();
    let commit_id = match &rv.sel {
        Sel::Commit(id) => Some(id.clone()),
        Sel::Working => None,
    };
    let branch = rv.note_branch();
    let task = rv.note_task();
    if rv.editor.tab == NoteTab::Commit && commit_id.is_none() {
        rv.editor.tab = NoteTab::Branch;
    }
    let desired = match rv.editor.tab {
        NoteTab::Commit => commit_id.clone().map(NoteTarget::Commit),
        NoteTab::Branch => branch.clone().map(NoteTarget::Branch),
        NoteTab::Task => task.clone().map(NoteTarget::Task),
        NoteTab::Repo => Some(NoteTarget::Repo),
        NoteTab::Pinned => rv.pinned_note.clone(),
    };
    rv.editor.sync(desired, &rv.notes);
    let now = ui.input(|i| i.time);
    let compact = is_compact(rv, pro);
    let fit_before = rv.editor.fit_h;

    // ---- grip: drag to resize, double-click to collapse / expand
    grip(ui, prefs, panel_h);

    // ---- tab bar (wraps when the panel is narrow)
    let tab_before = rv.editor.tab;
    ui.horizontal_wrapped(|ui| {
        let chevron = if prefs.notes_collapsed { "▸" } else { "▾" };
        let tip = if prefs.notes_collapsed { "Show the note" } else { "Collapse to this row" };
        if ui.add(egui::Button::new(RichText::new(chevron).color(theme::TEXT_DIM)).frame(false)).on_hover_text(tip).clicked() {
            prefs.notes_collapsed = !prefs.notes_collapsed;
            let _ = margin_core::config::save_state(prefs);
        }
        // Move the panel to the other column (under the history / under the details).
        let (arrow, tip) = if prefs.notes_under_history { ("⇥", "Move notes under the commit details") } else { ("⇤", "Move notes under the branches and history") };
        if ui.add(egui::Button::new(RichText::new(arrow).color(theme::TEXT_DIM)).frame(false)).on_hover_text(tip).clicked() {
            actions.push(Action::ToggleNotesDock);
        }
        let ctx = ui.ctx().clone();
        let icon_size = ui.text_style_height(&egui::TextStyle::Body) - 1.0;
        // Tab label: text, then a note icon if the note exists, then a GitHub icon + PR number.
        let label = |text: String, has: bool, pr: Option<&margin_core::github::PrSummary>| {
            let mut a = egui::Atoms::new(text);
            if has {
                a.push_right(icons::image(&ctx, Icon::Note, icon_size, theme::ACCENT));
            }
            if let Some(pr) = pr {
                let tone = crate::pr_view::tone_color(pr.readiness().1);
                a.push_right(icons::image(&ctx, Icon::Github, icon_size, tone));
                a.push_right(RichText::new(format!("#{}", pr.number)).color(tone));
            }
            a
        };
        if let Some(id) = &commit_id {
            let has = rv.has_note(&NoteTarget::Commit(id.clone()));
            ui.selectable_value(&mut rv.editor.tab, NoteTab::Commit, label(format!("Commit {}", &id[..7.min(id.len())]), has, None))
                .on_hover_text("Bound to this exact commit. After a rebase/amend it stays with the old commit.");
        }
        if let Some(b) = &branch {
            let has = rv.has_note(&NoteTarget::Branch(b.clone()));
            ui.selectable_value(&mut rv.editor.tab, NoteTab::Branch, label(format!("Branch {b}"), has, rv.prs.get(b)))
                .on_hover_text("Bound to the branch name. Shown on the branch's top commit and follows it through rebase/amend/rename.");
        }
        if let Some(k) = &task {
            let has = rv.has_note(&NoteTarget::Task(k.clone()));
            ui.selectable_value(&mut rv.editor.tab, NoteTab::Task, label(format!("Task {k}"), has, None));
        }
        let has = rv.has_note(&NoteTarget::Repo);
        ui.selectable_value(&mut rv.editor.tab, NoteTab::Repo, label("Repo".into(), has, None));
        if let Some(p) = rv.pinned_note.clone() {
            let detached = rv.detached.iter().any(|d| d.note == p);
            let text = if detached { format!("Detached {}", short_target(&p)) } else { short_target(&p) };
            ui.selectable_value(&mut rv.editor.tab, NoteTab::Pinned, label(text, false, None));
            if ui.small_button("×").on_hover_text("Close").clicked() {
                rv.pinned_note = None;
                if rv.editor.tab == NoteTab::Pinned {
                    rv.editor.tab = NoteTab::Branch;
                }
            }
        }
    });
    // Clicking a tab while collapsed opens the panel again.
    if prefs.notes_collapsed && rv.editor.tab != tab_before {
        prefs.notes_collapsed = false;
        let _ = margin_core::config::save_state(prefs);
    }
    if prefs.notes_collapsed {
        rv.editor.fit_h = Some(ui.cursor().top() - top);
        if rv.editor.fit_h != fit_before {
            ui.ctx().request_repaint();
        }
        return;
    }
    // "Link task…" for a branch without one (an action, so not shaped like a tab).
    let link_task = |ui: &mut Ui, actions: &mut Vec<Action>| {
        if let (None, Some(b)) = (&task, &branch) {
            if ui.link(RichText::new("Link task…").small()).on_hover_text("Link a Jira / Linear / Asana task to this branch").clicked() {
                actions.push(Action::LinkTaskDialog { branch: b.clone() });
            }
        }
    };

    // ---- empty note: one line instead of a big empty editor
    if compact {
        ui.horizontal(|ui| {
            let ed = &mut rv.editor;
            match &ed.target {
                Some(t) => {
                    ui.label(RichText::new(format!("No note on {} yet.", short_target(t))).color(theme::TEXT_DIM));
                    if ui.small_button("Write note").clicked() {
                        ed.preview = false;
                        ui.ctx().memory_mut(|m| m.request_focus(edit_id()));
                    }
                }
                None => {
                    ui.label(RichText::new(match ed.tab {
                        NoteTab::Branch => "Detached HEAD — no branch to attach notes to.",
                        NoteTab::Task => "No task linked.",
                        _ => "Nothing selected.",
                    }).color(theme::TEXT_DIM));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| link_task(ui, actions));
        });
        // Same height as any other note: switching notes never resizes the panel.
        rv.editor.fit_h = None;
        if fit_before.is_some() {
            ui.ctx().request_repaint();
        }
        return;
    }
    rv.editor.fit_h = None;
    if fit_before.is_some() {
        ui.ctx().request_repaint();
    }

    // Second row: Edit / Split / Preview, formatting, + Entry, save state; Link task on the right.
    let mut fmt: Option<crate::note_format::Fmt> = None;
    // Wraps in a narrow panel (e.g. under the history).
    ui.horizontal_wrapped(|ui| {
        {
            let ed = &mut rv.editor;
            if ed.target.is_some() {
                let editing = !ed.preview;
                if ui.selectable_label(editing && !prefs.notes_split, "Edit").clicked() {
                    ed.preview = false;
                    prefs.notes_split = false;
                    let _ = margin_core::config::save_state(prefs);
                }
                if ui.selectable_label(editing && prefs.notes_split, "Split").on_hover_text("Editor and live preview side by side").clicked() {
                    ed.preview = false;
                    prefs.notes_split = true;
                    let _ = margin_core::config::save_state(prefs);
                }
                if ui.selectable_label(ed.preview, "Preview").clicked() {
                    ed.preview = true;
                }
                if editing {
                    ui.separator();
                    for f in crate::note_format::Fmt::ALL {
                        let (l, tip) = f.label();
                        let mut t = RichText::new(l);
                        t = match f {
                            crate::note_format::Fmt::Bold => t.strong(),
                            crate::note_format::Fmt::Italic => t.italics(),
                            crate::note_format::Fmt::Code => t.monospace(),
                            _ => t,
                        };
                        if ui.add(egui::Button::new(t).frame(false).min_size(egui::vec2(18.0, 0.0))).on_hover_text(tip).clicked() {
                            fmt = Some(f);
                        }
                    }
                    ui.separator();
                }
                // Delete (with confirmation) when there's a saved note.
                if ed.target.is_some() && !ed.base_version.is_empty() {
                    if let Some(t) = ed.target.clone() {
                        if trash_button(ui).on_hover_text("Delete this note…").clicked() {
                            actions.push(Action::Confirm {
                                msg: format!(
                                    "Delete the note on {}?\n\nIt's moved to the notes trash folder ({}), so it can still be recovered from there.",
                                    short_target(&t),
                                    ".git/margin/notes/.trash"
                                ),
                                action: Box::new(Action::DeleteNote(t)),
                            });
                        }
                    }
                }
                if ui.small_button("+ Entry").on_hover_text("Append a timestamped entry").clicked() {
                    let mut t = ed.text.trim_end().to_string();
                    if !t.is_empty() {
                        t.push_str("\n\n");
                    }
                    // Local time with its zone, since notes are shared ("2026-11-16 14:32 KST").
                    let now_s = margin_core::time_now();
                    t.push_str(&format!("### {} {} · {}\n\n", margin_core::format_local(now_s), margin_core::local_zone(now_s), author));
                    ed.text = t;
                    ed.dirty = true;
                    ed.last_edit = now;
                    ed.preview = false;
                }
                // Notes sync indicator (click → Settings)
                {
                    use margin_core::sync::RepoSync;
                    let label = match &rv.sync_state {
                        RepoSync::Active { .. } => match &rv.sync.lock().unwrap().last {
                            Some((_, Err(_))) => Some(("sync failed", theme::ERROR)),
                            _ => Some(("synced", theme::ADDED)),
                        },
                        RepoSync::Paused => Some(("sync paused (Pro)", theme::ACCENT)),
                        RepoSync::NoFolder => Some(("sync: no folder", theme::ACCENT)),
                        RepoSync::Off => None,
                    };
                    if let Some((t, c)) = label {
                        let tip = crate::app::sync_status_text(rv).0;
                        if ui.add(egui::Label::new(RichText::new(t).small().color(c)).sense(egui::Sense::click())).on_hover_text(tip).clicked() {
                            actions.push(Action::OpenSettings);
                        }
                    }
                }
                let ed = &mut rv.editor;
                let (txt, color) = if ed.error.is_some() {
                    ("error", theme::ERROR)
                } else if ed.dirty {
                    ("unsaved", theme::ACCENT)
                } else {
                    ("saved", theme::TEXT_DIM)
                };
                ui.label(RichText::new(txt).small().color(color));
            }
        }
        ui.add_space(6.0);
        link_task(ui, actions);
    });
    ui.separator();

    // ---- "re-attached automatically" banner (Commit / pinned tabs)
    if let Some(t) = &rv.editor.target {
        let ts = t.to_string();
        if let Some(e) = rv.reattached.iter().find(|e| e.to == ts) {
            let from = e.from.strip_prefix("commit:").unwrap_or(&e.from);
            let id = e.id.clone();
            egui::Frame::new().fill(theme::HUNK_BG).inner_margin(6.0).corner_radius(3.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!(
                        "Re-attached automatically from {} (same change, git patch-id) · {}.",
                        &from[..8.min(from.len())],
                        margin_core::format_local(e.at)
                    )).color(theme::TEXT_DIM));
                    if ui.small_button("Undo").on_hover_text("Move the note back to the old commit and restore this one").clicked() {
                        actions.push(Action::UndoReattach(id.clone()));
                    }
                    if ui.small_button("Keep").on_hover_text("Looks right — stop offering undo").clicked() {
                        actions.push(Action::KeepReattach(id.clone()));
                    }
                });
            });
        }
    }

    // ---- detached-note banner
    if rv.editor.tab == NoteTab::Pinned {
        if let Some(d) = rv.pinned_note.as_ref().and_then(|p| rv.detached.iter().find(|d| &d.note == p)) {
            let from = d.note.clone();
            egui::Frame::new().fill(theme::HUNK_BG).inner_margin(6.0).corner_radius(3.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("Detached: {}.", d.reason.describe())).color(theme::ACCENT));
                    if let Some(sj) = &d.subject {
                        ui.label(RichText::new(format!("Was: “{sj}”")).color(theme::TEXT_DIM));
                    }
                    if !d.candidates.is_empty() {
                        ui.label(RichText::new("Same change found in:").color(theme::TEXT_DIM));
                        for c in &d.candidates {
                            if ui.small_button(&c[..8.min(c.len())]).on_hover_text("Attach the note to this commit").clicked() {
                                actions.push(Action::MoveNote { from: from.clone(), to: format!("commit:{c}") });
                            }
                        }
                    }
                    if ui.small_button("Re-attach…").clicked() {
                        actions.push(Action::ReattachDialog(from.clone()));
                    }
                    if ui.small_button("Delete…").clicked() {
                        actions.push(Action::Confirm { msg: format!("Delete note {from}?"), action: Box::new(Action::DeleteNote(from.clone())) });
                    }
                });
            });
        }
    }

    // ---- PR card (branch tab) / task card (task tab)
    if rv.editor.tab == NoteTab::Branch {
        if let Some(b) = &branch {
            // Own scroll region so a long review never hides the note below.
            let max = (ui.available_height() * 0.5).max(80.0);
            egui::ScrollArea::vertical().id_salt("pr_scroll").max_height(max).auto_shrink([false, true]).show(ui, |ui| {
                if pro {
                    crate::pr_view::card(ui, rv, b, actions);
                } else if rv.looks_github {
                    crate::pro::lock_hint(ui, "GitHub PR status", license, actions);
                }
            });
        }
    }
    if rv.editor.tab == NoteTab::Task {
        if let Some(k) = &task {
            if tasks_enabled && !pro {
                crate::pro::lock_hint(ui, "Jira ticket details", license, actions);
            }
            task_card(ui, k, task_cache.get(k), tasks_enabled && pro, task_url(k), &mut rv.editor.requested_tasks, actions);
        }
    }

    let ed = &mut rv.editor;
    if ed.target.is_none() {
        ui.weak(match ed.tab {
            NoteTab::Branch => "Detached HEAD — no branch to attach notes to.",
            NoteTab::Task => "No task linked. Name the branch like feature/PROJ-123-… or use “Link task…”.",
            _ => "Nothing selected.",
        });
        return;
    }

    if ed.external {
        egui::Frame::new().fill(theme::REMOVED_BG).inner_margin(6.0).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("This note was changed elsewhere (another agent or window) while you were editing.");
                if ui.button("Load theirs").clicked() {
                    if let Some(t) = ed.target.clone() {
                        ed.load(t, &rv.notes);
                    }
                }
                if ui.button("Keep mine (overwrite)").clicked() {
                    ed.force_save(&rv.notes);
                }
            });
        });
    } else if let Some(e) = &ed.error {
        ui.colored_label(theme::ERROR, e);
    }
    for v in &ed.violations {
        ui.colored_label(theme::ERROR, v.to_string());
    }

    // ⌘B / ⌘I / ⌘K while typing in the note.
    if !ed.preview && ui.memory(|m| m.has_focus(edit_id())) {
        use crate::note_format::Fmt;
        let key = ui.input_mut(|i| {
            [(egui::Key::B, Fmt::Bold), (egui::Key::I, Fmt::Italic), (egui::Key::K, Fmt::Link)]
                .into_iter()
                .find(|(k, _)| i.consume_key(egui::Modifiers::COMMAND, *k))
                .map(|(_, f)| f)
        });
        fmt = fmt.or(key);
    }
    if let Some(f) = fmt {
        apply_format(ui.ctx(), ed, f, now);
    }

    if ed.preview {
        egui::ScrollArea::vertical().id_salt("note_scroll").auto_shrink([false, false]).show(ui, |ui| {
            if ed.text.trim().is_empty() {
                if ui.link("Empty note — click to write").clicked() {
                    ed.preview = false;
                }
                return;
            }
            let resp = preview(ui, ed, &rv.notes, actions);
            if resp.interact(egui::Sense::click()).double_clicked() {
                ed.preview = false;
            }
        });
    } else if prefs.notes_split {
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical().id_salt("note_edit_scroll").auto_shrink([false, false]).show(&mut cols[0], |ui| {
                editor(ui, ed, now);
            });
            egui::ScrollArea::vertical().id_salt("note_split_preview").auto_shrink([false, false]).show(&mut cols[1], |ui| {
                if ed.text.trim().is_empty() {
                    ui.weak("Preview");
                } else {
                    preview(ui, ed, &rv.notes, actions);
                }
            });
        });
    } else {
        egui::ScrollArea::vertical().id_salt("note_edit_scroll").auto_shrink([false, false]).show(ui, |ui| {
            editor(ui, ed, now);
        });
    }

    // Autosave 1.5 s after the last keystroke, if the content is valid.
    if ed.dirty && !ed.external && ed.violations.is_empty() {
        if now - ed.last_edit > 1.5 {
            ed.save(&rv.notes);
            rv.note_index = rv.notes.list().map(|l| l.into_iter().map(|m| (m.target, m.title)).collect()).unwrap_or_default();
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
        }
    }
}

/// The note would be an empty editor: show one line instead (and shrink the panel).
fn is_compact(rv: &RepoView, pro: bool) -> bool {
    let ed = &rv.editor;
    if ed.tab == NoteTab::Task || ed.tab == NoteTab::Pinned || ed.dirty || ed.external || ed.error.is_some() {
        return false;
    }
    if ed.tab == NoteTab::Branch {
        // The PR card (or its Pro hint) lives in the branch tab.
        if let Some(b) = rv.note_branch() {
            if rv.prs.contains_key(&b) || (!pro && rv.looks_github) {
                return false;
            }
        }
    }
    let banner = ed.target.as_ref().is_some_and(|t| rv.reattached.iter().any(|e| e.to == t.to_string()));
    !banner && ed.preview && ed.text.trim().is_empty()
}

/// Thin bar on top of the panel: drag to resize, double-click to collapse.
/// (Resizing never changes the note's mode: Preview stays Preview.)
fn grip(ui: &mut Ui, prefs: &mut margin_core::config::State, panel_h: f32) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::click_and_drag());
    let hot = resp.hovered() || resp.dragged();
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let pill = egui::Rect::from_center_size(rect.center(), egui::vec2(36.0, 4.0));
    ui.painter().rect_filled(pill, 2.0, if hot { theme::TEXT_DIM } else { theme::HOVER });
    if resp.dragged() {
        let dy = resp.drag_delta().y;
        if dy != 0.0 {
            prefs.notes_height = Some((panel_h - dy).max(MIN_H));
            prefs.notes_collapsed = false;
        }
    }
    if resp.double_clicked() {
        prefs.notes_collapsed = !prefs.notes_collapsed;
    }
    if resp.drag_stopped() || resp.double_clicked() {
        let _ = margin_core::config::save_state(prefs);
    }
    resp.on_hover_text("Drag to resize · double-click to collapse");
}

/// Small trash-can icon button.
fn trash_button(ui: &mut Ui) -> egui::Response {
    let s = ui.spacing().interact_size.y;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(s, s), egui::Sense::click());
    let hot = resp.hovered();
    if hot {
        ui.painter().rect_filled(rect, 3.0, theme::HOVER);
    }
    let c = if hot { theme::ERROR } else { theme::TEXT_DIM };
    let st = egui::Stroke::new(1.2, c);
    let p = ui.painter();
    let ctr = rect.center();
    // lid, handle, can, two ribs
    p.line_segment([ctr + egui::vec2(-5.0, -3.5), ctr + egui::vec2(5.0, -3.5)], st);
    p.line_segment([ctr + egui::vec2(-1.5, -5.0), ctr + egui::vec2(1.5, -5.0)], st);
    let can = [ctr + egui::vec2(-3.8, -3.5), ctr + egui::vec2(-3.0, 5.0), ctr + egui::vec2(3.0, 5.0), ctr + egui::vec2(3.8, -3.5)];
    p.add(egui::Shape::line(can.to_vec(), st));
    p.line_segment([ctr + egui::vec2(-1.0, -1.5), ctr + egui::vec2(-1.0, 3.0)], egui::Stroke::new(1.0, c));
    p.line_segment([ctr + egui::vec2(1.0, -1.5), ctr + egui::vec2(1.0, 3.0)], egui::Stroke::new(1.0, c));
    resp
}

/// The Markdown editor, filling the space it's given.
fn editor(ui: &mut Ui, ed: &mut NoteEditor, now: f64) {
    let out = egui::TextEdit::multiline(&mut ed.text)
        .font(egui::TextStyle::Monospace)
        .desired_width(f32::INFINITY)
        .min_size(ui.available_size())
        .lock_focus(true)
        .hint_text("Markdown. Links OK, images/HTML not allowed. Agents can edit this via MCP.")
        .id(edit_id())
        .show(ui);
    if let Some(r) = out.cursor_range {
        let [a, b] = r.sorted_cursors();
        ed.sel = (a.index.0, b.index.0);
    }
    if out.response.changed() {
        ed.dirty = true;
        ed.last_edit = now;
        ed.violations = markdown::validate(&ed.text).err().unwrap_or_default();
    }
}

/// Rendered note; checkboxes toggle and save, links open.
fn preview(ui: &mut Ui, ed: &mut NoteEditor, notes: &NoteStore, actions: &mut Vec<Action>) -> egui::Response {
    let resp = ui.scope(|ui| markdown_view::render(ui, &ed.text));
    match resp.inner {
        Some(MdAction::Link(url)) => actions.push(Action::OpenUrl(url)),
        Some(MdAction::ToggleTask { offset, checked }) => {
            markdown_view::toggle_task(&mut ed.text, offset, checked);
            ed.dirty = true;
            ed.save(notes);
        }
        None => {}
    }
    resp.response
}

/// Apply a toolbar format to the editor's selection and put the cursor back.
fn apply_format(ctx: &egui::Context, ed: &mut NoteEditor, f: crate::note_format::Fmt, now: f64) {
    let (a, b) = ed.sel;
    let (text, na, nb) = crate::note_format::apply(&ed.text, a, b, f);
    ed.text = text;
    ed.sel = (na, nb);
    ed.dirty = true;
    ed.last_edit = now;
    ed.violations = markdown::validate(&ed.text).err().unwrap_or_default();
    let id = edit_id();
    if let Some(mut st) = egui::text_edit::TextEditState::load(ctx, id) {
        let range = egui::text::CCursorRange::two(egui::text::CCursor::new(na), egui::text::CCursor::new(nb));
        st.cursor.set_char_range(Some(range));
        st.store(ctx, id);
    }
    ctx.memory_mut(|m| m.request_focus(id));
}

/// "commit 1a2b3c4" / "branch feat/x" / "task PROJ-1".
pub fn short_target(t: &NoteTarget) -> String {
    match t {
        NoteTarget::Commit(c) => format!("commit {}", &c[..7.min(c.len())]),
        NoteTarget::Branch(b) => format!("branch {b}"),
        NoteTarget::Task(k) => format!("task {k}"),
        NoteTarget::Repo => "repo".into(),
    }
}

fn task_card(
    ui: &mut Ui,
    key: &str,
    t: Option<&Result<Task, String>>,
    enabled: bool,
    url: Option<String>,
    requested: &mut HashSet<String>,
    actions: &mut Vec<Action>,
) {
    if !enabled {
        // No API configured: still one click to the tracker (Linear, Asana, Jira…).
        match url {
            Some(u) => {
                if ui.link(format!("Open {key} ↗")).on_hover_text(&u).clicked() {
                    actions.push(Action::OpenUrl(u));
                }
            }
            None => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(key).color(theme::ACCENT));
                    if ui.link(RichText::new("Set the ticket link in Settings to open it").small()).clicked() {
                        actions.push(Action::OpenSettings);
                    }
                });
            }
        }
        ui.separator();
        return;
    }
    match t {
        None => {
            if requested.insert(key.to_string()) {
                actions.push(Action::FetchTask(key.to_string()));
            }
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(format!("Loading {key}…"));
            });
        }
        Some(Err(e)) => {
            ui.horizontal(|ui| {
                ui.colored_label(theme::ERROR, e);
                if ui.small_button("Retry").clicked() {
                    actions.push(Action::FetchTask(key.to_string()));
                }
            });
        }
        Some(Ok(t)) => {
            ui.horizontal_wrapped(|ui| {
                ui.strong(&t.summary);
            });
            ui.horizontal(|ui| {
                let color = match t.status_category.as_str() {
                    "done" => theme::ADDED,
                    "indeterminate" => theme::lane(0),
                    _ => theme::TEXT_DIM,
                };
                ui.label(RichText::new(&t.status).small().color(color));
                ui.label(RichText::new(&t.issue_type).small().color(theme::TEXT_DIM));
                if let Some(a) = &t.assignee {
                    ui.label(RichText::new(a).small().color(theme::TEXT_DIM));
                }
                if let Some(p) = &t.priority {
                    ui.label(RichText::new(p).small().color(theme::TEXT_DIM));
                }
                if ui.small_button("Open").clicked() {
                    actions.push(Action::OpenUrl(t.url.clone()));
                }
                if ui.small_button("⟳").clicked() {
                    actions.push(Action::FetchTask(key.to_string()));
                }
            });
            if !t.description.is_empty() {
                egui::CollapsingHeader::new(RichText::new("Description").small()).id_salt(("taskdesc", key)).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                        ui.label(&t.description);
                    });
                });
            }
        }
    }
    ui.separator();
}
