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
use crate::icons::Icon;
use crate::markdown_view::{self, MdAction};
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::markdown;
use margin_core::notes::{NoteStore, NoteTarget};
use margin_core::tasks::Task;
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum NoteTab {
    /// Note on the selected commit.
    Commit,
    /// Note on the branch (follows it through rebase / amend / rename).
    #[default]
    Branch,
    /// The branch's pull request (no note: status, checks, conversation).
    Pr,
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
    /// Put the cursor in the editor once it's on screen (Write note, Edit…).
    focus_next: bool,
    /// Tasks already asked for (for the branch's ticket summary).
    pub(crate) requested_tasks: HashSet<String>,
    last_edit: f64,
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
    task_info: Option<&Result<Task, String>>,
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
    // The branch's PR (or, without Pro, a hint for it on GitHub repos).
    let pr = branch.as_ref().and_then(|b| rv.prs.get(b).cloned());
    let pr_tab = pr.is_some() || (branch.is_some() && !pro && rv.looks_github);
    // Fall back to a tab that exists for this selection.
    if rv.editor.tab == NoteTab::Commit && commit_id.is_none() {
        rv.editor.tab = NoteTab::Branch;
    }
    if rv.editor.tab == NoteTab::Pr && !pr_tab {
        rv.editor.tab = NoteTab::Branch;
    }
    if rv.editor.tab == NoteTab::Branch && branch.is_none() && commit_id.is_some() {
        rv.editor.tab = NoteTab::Commit;
    }
    let desired = match rv.editor.tab {
        NoteTab::Commit => commit_id.clone().map(NoteTarget::Commit),
        NoteTab::Branch => branch.clone().map(NoteTarget::Branch),
        NoteTab::Pr => None,
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
        use crate::widgets::{Glyph, Seg, icon_button, segmented};
        let tip = if prefs.notes_collapsed { "Show the note" } else { "Collapse to this row" };
        if icon_button(ui, Glyph::Chevron { open: !prefs.notes_collapsed }, tip).clicked() {
            prefs.notes_collapsed = !prefs.notes_collapsed;
            let _ = margin_core::config::save_state(prefs);
        }
        // Move the panel to the other column (under the history / under the details).
        let (to_right, tip) = if prefs.notes_under_history {
            (true, "Move notes under the commit details")
        } else {
            (false, "Move notes under the branches and history")
        };
        if icon_button(ui, Glyph::Dock { to_right }, tip).clicked() {
            actions.push(Action::ToggleNotesDock);
        }
        ui.add_space(4.0);
        // Tabs: the commit's note, the branch's note, the branch's PR. A note
        // icon marks a note that exists.
        let note_mark = |seg: Seg, has: bool| if has { seg.mark(Icon::Note, None, theme::ACCENT()) } else { seg };
        let mut tabs: Vec<(NoteTab, Seg)> = vec![];
        if let Some(id) = &commit_id {
            let has = rv.has_note(&NoteTarget::Commit(id.clone()));
            let seg = Seg::new(Some(Icon::GitCommit), format!("Commit {}", &id[..7.min(id.len())]))
                .tip("Note on this commit. It stays with this exact commit (after a rebase or amend, with the old one).");
            tabs.push((NoteTab::Commit, note_mark(seg, has)));
        }
        if let Some(b) = &branch {
            let has = rv.has_note(&NoteTarget::Branch(b.clone()));
            let seg = Seg::new(Some(Icon::GitBranch), format!("Branch {b}")).tip("Note on this branch. It follows the branch through rebase, amend and rename.");
            tabs.push((NoteTab::Branch, note_mark(seg, has)));
        }
        if pr_tab {
            let seg = match &pr {
                // "PR ◯ #42", the number coloured by readiness
                Some(p) => Seg::new(None, "PR")
                    .mark(Icon::Github, Some(format!("#{}", p.number)), crate::pr_view::tone_color(p.readiness().1))
                    .tip(format!("{} · {}", p.title, p.readiness().0)),
                None => Seg::new(Some(Icon::Github), "PR").tip("Pull request status (Margin Pro)"),
            };
            tabs.push((NoteTab::Pr, seg));
        }
        if let Some(p) = rv.pinned_note.clone() {
            let detached = rv.detached.iter().any(|d| d.note == p);
            let text = if detached { format!("Detached {}", short_target(&p)) } else { short_target(&p) };
            tabs.push((NoteTab::Pinned, Seg::new(Some(Icon::Note), text)));
        }
        let sel = tabs.iter().position(|(t, _)| *t == rv.editor.tab);
        let (ids, segs): (Vec<NoteTab>, Vec<Seg>) = tabs.into_iter().unzip();
        if let Some(i) = segmented(ui, &segs, sel) {
            rv.editor.tab = ids[i];
        }
        if rv.pinned_note.is_some() && ui.small_button("×").on_hover_text("Close this note").clicked() {
            rv.pinned_note = None;
            if rv.editor.tab == NoteTab::Pinned {
                rv.editor.tab = NoteTab::Branch;
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
    // The dragged height, except for an empty note (fitted below).
    rv.editor.fit_h = None;

    // ---- PR tab: the whole panel for status, checks and the conversation
    if rv.editor.tab == NoteTab::Pr {
        ui.separator();
        if let Some(b) = &branch {
            egui::ScrollArea::vertical().id_salt("pr_scroll").auto_shrink([false, false]).show(ui, |ui| {
                if pro {
                    crate::pr_view::card(ui, rv, b, actions);
                } else {
                    crate::pro::lock_hint(ui, "GitHub PR status", license, actions);
                }
            });
        }
        if fit_before.is_some() {
            ui.ctx().request_repaint();
        }
        return;
    }

    // ---- what this note is attached to (and, when empty, a button to start it)
    // (wraps onto more lines in a narrow panel instead of overlapping)
    ui.horizontal_wrapped(|ui| {
        if let Some(k) = &task
            && pro
            && task_info.is_none()
            && rv.editor.requested_tasks.insert(k.clone())
        {
            actions.push(Action::FetchTask(k.clone()));
        }
        target_line(ui, rv, commit_id.as_deref());
        if compact && rv.editor.target.is_some() {
            ui.add_space(6.0);
            if crate::widgets::primary_button(ui, "Write note").clicked() {
                rv.editor.preview = false;
                rv.editor.focus_next = true;
            }
        }
        if let (NoteTab::Branch, Some(b)) = (rv.editor.tab, &branch) {
            // Inline after the text (and Write note), wrapping to the next
            // line when there's no room: it never sits on top of anything.
            ui.add_space(10.0);
            task_link(ui, b, task.as_deref(), task_info, task_url, actions);
        }
    });
    // An empty note shrinks the panel to this one line (it grows back,
    // animated, on "Write note" or when switching to a note with text).
    if compact {
        rv.editor.fit_h = Some(ui.cursor().top() - top + 4.0);
    }
    if rv.editor.fit_h != fit_before {
        ui.ctx().request_repaint();
    }
    if compact {
        return;
    }

    // Second row: Edit / Split / Preview, the formatting buttons (while
    // editing), then on the right the save state, Add entry, ⋯ and Done.
    let mut fmt: Option<crate::note_format::Fmt> = None;
    let mut done = false;
    if rv.editor.target.is_some() {
        let tb = toolbar(ui, rv, prefs, author, now, actions);
        fmt = tb.0;
        done = tb.1;
    }
    ui.separator();

    // ---- "re-attached automatically" banner (Commit / pinned tabs)
    if let Some(t) = &rv.editor.target {
        let ts = t.to_string();
        if let Some(e) = rv.reattached.iter().find(|e| e.to == ts) {
            let from = e.from.strip_prefix("commit:").unwrap_or(&e.from);
            let id = e.id.clone();
            egui::Frame::new().fill(theme::HUNK_BG()).inner_margin(6.0).corner_radius(3.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!(
                        "Re-attached automatically from {} (same change, git patch-id) · {}.",
                        &from[..8.min(from.len())],
                        margin_core::format_local(e.at)
                    )).color(theme::TEXT_DIM()));
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
            egui::Frame::new().fill(theme::HUNK_BG()).inner_margin(6.0).corner_radius(3.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("Detached: {}.", d.reason.describe())).color(theme::ACCENT()));
                    if let Some(sj) = &d.subject {
                        ui.label(RichText::new(format!("Was: “{sj}”")).color(theme::TEXT_DIM()));
                    }
                    if !d.candidates.is_empty() {
                        ui.label(RichText::new("Same change found in:").color(theme::TEXT_DIM()));
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

    let ed = &mut rv.editor;
    if ed.target.is_none() {
        ui.weak(match ed.tab {
            NoteTab::Branch => "Detached HEAD — no branch to attach notes to.",
            _ => "Nothing selected.",
        });
        return;
    }

    if ed.external {
        egui::Frame::new().fill(theme::REMOVED_BG()).inner_margin(6.0).show(ui, |ui| {
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
        ui.colored_label(theme::ERROR(), e);
    }
    for v in &ed.violations {
        ui.colored_label(theme::ERROR(), v.to_string());
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

    // Done (button, Esc or ⌘↩ in the editor): save now and show the result.
    let editor_keys_done = !ed.preview
        && ui.input(|i| i.key_pressed(egui::Key::Escape) || (i.key_pressed(egui::Key::Enter) && i.modifiers.command))
        && ui.memory(|m| m.has_focus(edit_id()) || m.focused().is_none());
    if (done || editor_keys_done) && !ed.preview {
        if ed.dirty && ed.violations.is_empty() {
            ed.save(&rv.notes);
            rv.note_index = rv.notes.list().map(|l| l.into_iter().map(|m| (m.target, m.title)).collect()).unwrap_or_default();
        }
        ed.preview = true;
        ui.memory_mut(|m| m.surrender_focus(edit_id()));
    }

    if ed.preview {
        egui::ScrollArea::vertical().id_salt("note_scroll").auto_shrink([false, false]).show(ui, |ui| {
            if ed.text.trim().is_empty() {
                if ui.link("Empty note — click to write").clicked() {
                    ed.preview = false;
                    ed.focus_next = true;
                }
                return;
            }
            let resp = preview(ui, ed, &rv.notes, actions);
            let resp = resp.interact(egui::Sense::click()).on_hover_text_at_pointer("Double-click to edit");
            if resp.double_clicked() {
                ed.preview = false;
                ed.focus_next = true;
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
    let _ = pro;
    let ed = &rv.editor;
    if ed.tab == NoteTab::Pinned || ed.tab == NoteTab::Pr || ed.dirty || ed.external || ed.error.is_some() {
        return false;
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
    ui.painter().rect_filled(pill, 2.0, if hot { theme::TEXT_DIM() } else { theme::HOVER() });
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

/// The Markdown editor, filling the space it's given.
/// Longest readable line in the editor and the preview; wider panels keep
/// the text at this width (left-aligned) instead of stretching it.
const READ_W: f32 = 760.0;

/// The note editor: Markdown source with roomy lines and light highlighting
/// (headings, list and task markers, quotes, code) so it reads like the result.
fn editor(ui: &mut Ui, ed: &mut NoteEditor, now: f64) {
    theme::content_style(ui);
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let job = highlight(buf.as_str(), &font, wrap.min(READ_W));
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let out = egui::TextEdit::multiline(&mut ed.text)
        .font(egui::TextStyle::Monospace)
        .desired_width(f32::INFINITY)
        .min_size(ui.available_size())
        .margin(egui::vec2(14.0, 12.0))
        .lock_focus(true)
        .hint_text("Write in Markdown: ## heading, **bold**, - list, - [ ] task, `code`, [link](https://…).\nEsc or ⌘↩ when you're done. Agents can edit this note too, via MCP.")
        .layouter(&mut layouter)
        .id(edit_id())
        .show(ui);
    if std::mem::take(&mut ed.focus_next) {
        out.response.request_focus();
        // caret at the end, ready to add to the note
        let end = egui::text::CCursor::new(ed.text.chars().count());
        let mut st = out.state.clone();
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
        st.store(ui.ctx(), edit_id());
    }
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

/// Lay out Markdown source with per-line colours and 1.5× line height.
fn highlight(text: &str, font: &egui::FontId, wrap: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob { wrap: egui::text::TextWrapping { max_width: wrap, ..Default::default() }, ..Default::default() };
    let line_h = Some((font.size * 1.5).round());
    let fmt = |color: egui::Color32| TextFormat { font_id: font.clone(), color, line_height: line_h, ..Default::default() };
    let mut in_code = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") {
            in_code = !in_code;
            job.append(line, 0.0, fmt(theme::TEXT_DIM()));
            continue;
        }
        if in_code {
            job.append(line, 0.0, fmt(theme::ADDED()));
        } else if t.starts_with('#') {
            job.append(line, 0.0, fmt(theme::ACCENT()));
        } else if t.starts_with('>') {
            job.append(line, 0.0, fmt(theme::TEXT_DIM()));
        } else if let Some(m) = ["- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ "].iter().find(|m| t.starts_with(**m)) {
            // the list / task marker dim, the item text normal
            let lead = line.len() - t.len() + m.len();
            job.append(&line[..lead], 0.0, fmt(theme::TEXT_DIM()));
            job.append(&line[lead..], 0.0, fmt(theme::TEXT()));
        } else {
            job.append(line, 0.0, fmt(theme::TEXT()));
        }
    }
    job
}

/// Rendered note; checkboxes toggle and save, links open.
fn preview(ui: &mut Ui, ed: &mut NoteEditor, notes: &NoteStore, actions: &mut Vec<Action>) -> egui::Response {
    let resp = egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 12 }).show(ui, |ui| {
        ui.set_max_width(READ_W);
        theme::content_style(ui);
        markdown_view::render(ui, &ed.text)
    });
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

/// The note toolbar. Returns a formatting button pressed and whether Done was.
///
/// Wide panel: one row. Narrow: the formatting buttons get a row of their own.
fn toolbar(
    ui: &mut Ui,
    rv: &mut RepoView,
    prefs: &mut margin_core::config::State,
    author: &str,
    now: f64,
    actions: &mut Vec<Action>,
) -> (Option<crate::note_format::Fmt>, bool) {
    use crate::note_format::Fmt;
    let editing = !rv.editor.preview;
    // Three widths (scaled with the text size): one row; the formatting
    // buttons on a row of their own; or everything wrapping left to right.
    let w = ui.available_width() / ui_scale(ui);
    let (wide, medium) = (w >= 640.0, w >= 430.0);
    let mut fmt = None;
    let mut done = false;
    let format_buttons = |ui: &mut Ui, fmt: &mut Option<Fmt>| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, f) in Fmt::ALL.into_iter().enumerate() {
            // groups: text (H B I code link) | blocks (list, task, quote)
            if i == 5 {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }
            if fmt_button(ui, f).clicked() {
                *fmt = Some(f);
            }
        }
    };
    let modes = |ui: &mut Ui, rv: &mut RepoView, prefs: &mut margin_core::config::State, done: &mut bool| {
        use crate::widgets::Seg;
        let modes = [
            Seg::new(None, "Edit").tip("Write Markdown (double-click the note to start)"),
            Seg::new(None, "Split").tip("Editor and live preview side by side"),
            Seg::new(None, "Preview").tip("The note as it reads (Esc or ⌘↩ from the editor)"),
        ];
        let cur = if rv.editor.preview { 2 } else if prefs.notes_split { 1 } else { 0 };
        match crate::widgets::segmented(ui, &modes, Some(cur)) {
            Some(2) => *done = true,
            Some(m) => {
                rv.editor.preview = false;
                prefs.notes_split = m == 1;
                let _ = margin_core::config::save_state(prefs);
                rv.editor.focus_next = true;
            }
            None => {}
        }
    };
    // Saved · + Entry · ⋯ · Done, in reading order (`rtl`: drawn right to left)
    let actions_group = |ui: &mut Ui, rv: &mut RepoView, actions: &mut Vec<Action>, done: &mut bool, rtl: bool| {
        let mut items: Vec<u8> = vec![0, 1, 2];
        if editing {
            items.push(3);
        }
        if rtl {
            items.reverse();
        }
        for it in items {
            match it {
                0 => save_state(ui, rv, actions, rtl),
                1 => {
                    if ui.button("+ Entry").on_hover_text(format!("Add a dated entry at the end (### {} · {author})", margin_core::format_local(margin_core::time_now()))).clicked() {
                        add_entry(&mut rv.editor, author, now);
                        rv.editor.focus_next = true;
                    }
                }
                2 => more_menu(ui, rv, actions),
                _ => {
                    if crate::widgets::primary_button(ui, "Done").on_hover_text("Save and show the note (Esc or ⌘↩)").clicked() {
                        *done = true;
                    }
                }
            }
        }
    };
    if medium {
        ui.horizontal(|ui| {
            modes(ui, rv, prefs, &mut done);
            if editing && wide {
                ui.add_space(10.0);
                format_buttons(ui, &mut fmt);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| actions_group(ui, rv, actions, &mut done, true));
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            modes(ui, rv, prefs, &mut done);
            ui.add_space(6.0);
            actions_group(ui, rv, actions, &mut done, false);
        });
    }
    if editing && !wide {
        ui.horizontal_wrapped(|ui| format_buttons(ui, &mut fmt));
    }
    (fmt, done)
}

/// One formatting button: an icon (or "H"), 26×24, tooltip with its shortcut.
fn fmt_button(ui: &mut Ui, f: crate::note_format::Fmt) -> egui::Response {
    use crate::note_format::Fmt;
    let icon = match f {
        Fmt::Heading => Icon::Heading,
        Fmt::Bold => Icon::Bold,
        Fmt::Italic => Icon::Italic,
        Fmt::Code => Icon::Code,
        Fmt::Link => Icon::Link,
        Fmt::Bullet => Icon::ListUnordered,
        Fmt::Task => Icon::Tasklist,
        Fmt::Quote => Icon::Quote,
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::click());
    let hot = resp.hovered() || resp.has_focus();
    if hot {
        ui.painter().rect_filled(rect, 4.0, theme::HOVER());
    }
    let color = if hot { theme::TEXT() } else { theme::TEXT_DIM() };
    crate::icons::paint(ui.painter(), icon, egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0)), color);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text(f.label().1)
}

/// "● Saved" / "● Unsaved…" / "● Can't save" (+ notes sync state, click → Settings).
/// `rtl`: the caller lays out right to left, so draw the parts in reverse.
fn save_state(ui: &mut Ui, rv: &RepoView, actions: &mut Vec<Action>, rtl: bool) {
    use margin_core::sync::RepoSync;
    let ed = &rv.editor;
    let (txt, color, tip) = if ed.error.is_some() || !ed.violations.is_empty() {
        ("Can't save", theme::ERROR(), "Fix the problem shown above the note")
    } else if ed.dirty {
        ("Saving…", theme::ACCENT(), "Saved automatically a moment after you stop typing")
    } else {
        ("Saved", theme::ADDED(), "Saved in .git/margin/notes")
    };
    let sync = match &rv.sync_state {
        RepoSync::Active { .. } => match &rv.sync.lock().unwrap().last {
            Some((_, Err(_))) => Some(("sync failed", theme::ERROR())),
            _ => Some(("synced", theme::TEXT_DIM())),
        },
        RepoSync::Paused => Some(("sync paused (Pro)", theme::ACCENT())),
        RepoSync::NoFolder => Some(("sync: no folder", theme::ACCENT())),
        RepoSync::Off => None,
    };
    let mut parts: Vec<u8> = vec![0, 1];
    if sync.is_some() {
        parts.push(2);
    }
    if rtl {
        parts.reverse();
    }
    for part in parts {
        match part {
            0 => {
                let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 3.5, color);
            }
            1 => {
                ui.label(RichText::new(txt).small().color(theme::TEXT_DIM())).on_hover_text(tip);
            }
            _ => {
                if let Some((t, c)) = sync {
                    let stip = crate::app::sync_status_text(rv).0;
                    ui.label(RichText::new("·").small().color(theme::TEXT_DIM()));
                    if ui.add(egui::Label::new(RichText::new(t).small().color(c)).sense(egui::Sense::click())).on_hover_text(stip).clicked() {
                        actions.push(Action::OpenSettings);
                    }
                }
            }
        }
    }
}

/// How much bigger than the default (13 pt) the interface text is.
fn ui_scale(ui: &Ui) -> f32 {
    (ui.style().text_styles[&egui::TextStyle::Body].size / 13.0).max(1.0)
}

/// ⋯: less common note actions, away from the everyday buttons.
fn more_menu(ui: &mut Ui, rv: &RepoView, actions: &mut Vec<Action>) {
    let ed = &rv.editor;
    let resp = ui.add(egui::Button::new(crate::icons::image(ui.ctx(), Icon::KebabHorizontal, 14.0, theme::TEXT_DIM())).frame(false).min_size(egui::vec2(26.0, 24.0)));
    let resp = resp.on_hover_text("More");
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(200.0);
        let saved = !ed.base_version.is_empty();
        let label = RichText::new("Delete note…").color(if saved { theme::ERROR() } else { theme::TEXT_DIM() });
        if let Some(t) = ed.target.clone()
            && ui.add_enabled(saved, egui::Button::new(label)).clicked()
        {
            actions.push(Action::Confirm {
                msg: format!(
                    "Delete the note on {}?\n\nIt's moved to the notes trash folder ({}), so it can still be recovered from there.",
                    short_target(&t),
                    ".git/margin/notes/.trash"
                ),
                action: Box::new(Action::DeleteNote(t)),
            });
        }
    });
}

/// Append "### 2026-11-17 14:32 KST · author" and open the editor there.
fn add_entry(ed: &mut NoteEditor, author: &str, now: f64) {
    let mut t = ed.text.trim_end().to_string();
    if !t.is_empty() {
        t.push_str("\n\n");
    }
    // Local time with its zone, since notes are shared.
    let now_s = margin_core::time_now();
    t.push_str(&format!("### {} {} · {}\n\n", margin_core::format_local(now_s), margin_core::local_zone(now_s), author));
    ed.text = t;
    ed.dirty = true;
    ed.last_edit = now;
    ed.preview = false;
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

/// What the note is attached to: "Note on commit 5c070c2 “subject” — stays
/// with this exact commit" / "Note on branch feature/x — follows the branch…".
fn target_line(ui: &mut Ui, rv: &RepoView, commit: Option<&str>) {
    let has = rv.editor.target.as_ref().is_some_and(|t| rv.has_note(t));
    let empty = if has { "" } else { " · no note yet" };
    let dim = |ui: &mut Ui, t: String| ui.label(RichText::new(t).small().color(theme::TEXT_DIM()));
    match (&rv.editor.tab, &rv.editor.target) {
        (NoteTab::Commit, _) => {
            let Some(id) = commit else { return };
            let subject = rv.snap.as_ref().and_then(|s| s.commits.iter().find(|c| c.id == id)).map(|c| c.subject.as_str()).unwrap_or("");
            ui.label(RichText::new(format!("Note on commit {}", &id[..7.min(id.len())])).strong());
            if !subject.is_empty() {
                ui.add(egui::Label::new(RichText::new(format!("“{subject}”")).color(theme::TEXT())).truncate()).on_hover_text(subject);
            }
            dim(ui, format!("stays with this exact commit{empty}"));
        }
        (NoteTab::Branch, Some(NoteTarget::Branch(b))) => {
            ui.label(RichText::new(format!("Note on branch {b}")).strong());
            dim(ui, format!("follows the branch through rebase, amend and rename{empty}"));
        }
        _ => {}
    }
}

/// The branch's task as a link to its ticket, or "Link task…".
fn task_link(
    ui: &mut Ui,
    branch: &str,
    task: Option<&str>,
    task_info: Option<&Result<Task, String>>,
    task_url: &dyn Fn(&str) -> Option<String>,
    actions: &mut Vec<Action>,
) {
    // no tracker for this repository yet: offer to choose one
    let set_up = task_url("PROJ-1").is_some() || task_url("#1").is_some();
    let Some(k) = task else {
        if ui.small_button("Link task…").on_hover_text("Link a Jira / Linear / GitHub issue to this branch").clicked() {
            actions.push(Action::LinkTaskDialog { branch: branch.to_string() });
        }
        if !set_up && ui.link(RichText::new("Set up tasks…").small().color(theme::TEXT_DIM())).on_hover_text("Choose where this repository's tasks come from").clicked() {
            actions.push(Action::OpenRepoSettings);
        }
        return;
    };
    let text = match task_info {
        Some(Ok(t)) => format!("{k} · {}", t.summary),
        _ => k.to_string(),
    };
    match task_url(k) {
        Some(u) => {
            // (long ticket titles are cut short)
            let short: String = if text.chars().count() > 48 { text.chars().take(47).chain(['…']).collect() } else { text.clone() };
            let r = ui.link(RichText::new(format!("{short} ↗")).color(theme::ACCENT()));
            if r.on_hover_text(format!("Open {u}")).clicked() {
                actions.push(Action::OpenUrl(u));
            }
        }
        None => {
            ui.add(egui::Label::new(RichText::new(text).color(theme::ACCENT())).truncate());
            if ui.link(RichText::new("Set up tasks…").small().color(theme::TEXT_DIM())).on_hover_text("Choose where this repository's tasks come from, to link it").clicked() {
                actions.push(Action::OpenRepoSettings);
            }
        }
    }
}
