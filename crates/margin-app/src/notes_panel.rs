//! Notes panel: Markdown notes for the selected commit, branch, task or repo.
//! Autosaves when valid; detects edits made concurrently by MCP agents.

use crate::app::{Action, RepoView, Sel};
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
        self.preview = !self.text.trim().is_empty();
        self.target = Some(t);
        self.dirty = false;
        self.external = false;
        self.violations.clear();
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

pub fn show(
    ui: &mut Ui,
    rv: &mut RepoView,
    task_cache: &HashMap<String, Result<Task, String>>,
    tasks_enabled: bool,
    author: &str,
    actions: &mut Vec<Action>,
) {
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
    };
    rv.editor.sync(desired, &rv.notes);
    let now = ui.input(|i| i.time);

    // ---- tab bar
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let dot = |has: bool| if has { " ●" } else { "" };
        if let Some(id) = &commit_id {
            let has = rv.has_note(&NoteTarget::Commit(id.clone()));
            ui.selectable_value(&mut rv.editor.tab, NoteTab::Commit, format!("Commit {}{}", &id[..7.min(id.len())], dot(has)));
        }
        if let Some(b) = &branch {
            let has = rv.has_note(&NoteTarget::Branch(b.clone()));
            ui.selectable_value(&mut rv.editor.tab, NoteTab::Branch, format!("Branch {b}{}", dot(has)));
        }
        match &task {
            Some(k) => {
                let has = rv.has_note(&NoteTarget::Task(k.clone()));
                ui.selectable_value(&mut rv.editor.tab, NoteTab::Task, format!("Task {k}{}", dot(has)));
            }
            None => {
                if let Some(b) = &branch {
                    if ui.small_button("Link task…").clicked() {
                        actions.push(Action::LinkTaskDialog { branch: b.clone() });
                    }
                }
            }
        }
        let has = rv.has_note(&NoteTarget::Repo);
        ui.selectable_value(&mut rv.editor.tab, NoteTab::Repo, format!("Repo{}", dot(has)));
    });
    // Second row: editor controls (kept separate so long branch names never overlap them).
    ui.horizontal(|ui| {
        {
            let ed = &mut rv.editor;
            if ed.target.is_some() {
                if ui.selectable_label(!ed.preview, "Edit").clicked() {
                    ed.preview = false;
                }
                if ui.selectable_label(ed.preview, "Preview").clicked() {
                    ed.preview = true;
                }
                if ui.small_button("+ Entry").on_hover_text("Append a timestamped entry").clicked() {
                    let mut t = ed.text.trim_end().to_string();
                    if !t.is_empty() {
                        t.push_str("\n\n");
                    }
                    t.push_str(&format!(
                        "### {} · {}\n\n",
                        margin_core::format_utc(
                            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
                        ),
                        author
                    ));
                    ed.text = t;
                    ed.dirty = true;
                    ed.last_edit = now;
                    ed.preview = false;
                }
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
    });
    ui.separator();

    // ---- task card
    if rv.editor.tab == NoteTab::Task {
        if let Some(k) = &task {
            task_card(ui, k, task_cache.get(k), tasks_enabled, &mut rv.editor.requested_tasks, actions);
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

    egui::ScrollArea::vertical().id_salt("note_scroll").auto_shrink([false, false]).show(ui, |ui| {
        if ed.preview {
            if ed.text.trim().is_empty() {
                if ui.link("Empty note — click to write").clicked() {
                    ed.preview = false;
                }
                return;
            }
            let resp = ui.scope(|ui| markdown_view::render(ui, &ed.text));
            match resp.inner {
                Some(MdAction::Link(url)) => actions.push(Action::OpenUrl(url)),
                Some(MdAction::ToggleTask { offset, checked }) => {
                    markdown_view::toggle_task(&mut ed.text, offset, checked);
                    ed.dirty = true;
                    ed.save(&rv.notes);
                }
                None => {}
            }
            if resp.response.interact(egui::Sense::click()).double_clicked() {
                ed.preview = false;
            }
        } else {
            let r = ui.add(
                egui::TextEdit::multiline(&mut ed.text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(12)
                    .lock_focus(true)
                    .hint_text("Markdown. Links OK, images/HTML not allowed. Agents can edit this via MCP.")
                    .id_salt("note_edit"),
            );
            if r.changed() {
                ed.dirty = true;
                ed.last_edit = now;
                ed.violations = markdown::validate(&ed.text).err().unwrap_or_default();
            }
        }
    });

    // Autosave 1.5 s after the last keystroke, if the content is valid.
    if ed.dirty && !ed.external && ed.violations.is_empty() {
        if now - ed.last_edit > 1.5 {
            ed.save(&rv.notes);
            rv.note_index = rv.notes.list().map(|l| l.into_iter().map(|m| m.target).collect()).unwrap_or_default();
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
        }
    }
}

fn task_card(
    ui: &mut Ui,
    key: &str,
    t: Option<&Result<Task, String>>,
    enabled: bool,
    requested: &mut HashSet<String>,
    actions: &mut Vec<Action>,
) {
    if !enabled {
        ui.weak("Configure Jira in config.json to see ticket details here.");
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
