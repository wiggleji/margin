//! Right-hand pane: working-directory staging + commit box, or commit details.

use crate::app::{Action, RepoView, Sel};
use crate::diff_view::DiffMode;
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::git::FileChange;

pub fn show(ui: &mut Ui, rv: &mut RepoView, actions: &mut Vec<Action>) {
    match rv.sel.clone() {
        Sel::Working => working(ui, rv, actions),
        Sel::Commit(_) => commit(ui, rv, actions),
    }
}

fn working(ui: &mut Ui, rv: &mut RepoView, actions: &mut Vec<Action>) {
    let Some(snap) = rv.snap.as_ref() else { return };
    let files = snap.status.files.clone();
    let unstaged: Vec<&FileChange> = files.iter().filter(|f| f.is_unstaged()).collect();
    let staged: Vec<&FileChange> = files.iter().filter(|f| f.is_staged()).collect();
    let sel = rv.selected_file.clone();

    let lists_h = (ui.available_height() * 0.38).clamp(90.0, 340.0);
    ui.vertical(|ui| {
        egui::ScrollArea::vertical().id_salt("worklists_scroll").auto_shrink([false, true]).max_height(lists_h).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("Unstaged ({})", unstaged.len()));
                if !unstaged.is_empty() {
                    if ui.small_button("Stage all").clicked() {
                        actions.push(Action::Stage(unstaged.iter().map(|f| f.path.clone()).collect()));
                    }
                    if ui.small_button("Discard all").clicked() {
                        actions.push(Action::Confirm {
                            msg: format!("Discard ALL {} unstaged changes? Untracked files will be deleted.", unstaged.len()),
                            action: Box::new(Action::Discard(unstaged.iter().map(|f| f.path.clone()).collect())),
                        });
                    }
                }
            });
            for f in &unstaged {
                file_row(ui, f, false, sel.as_ref(), actions);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.strong(format!("Staged ({})", staged.len()));
                if !staged.is_empty() && ui.small_button("Unstage all").clicked() {
                    actions.push(Action::Unstage(staged.iter().map(|f| f.path.clone()).collect()));
                }
            });
            for f in &staged {
                file_row(ui, f, true, sel.as_ref(), actions);
            }
        });

        // ---- commit box
        ui.separator();
        let task = rv.current_branch().and_then(|b| rv.notes.task_for_branch(b));
        let hint = match &task {
            Some(t) => format!("{t}: summary of the change   (⌘↵ to commit)"),
            None => "Commit message   (⌘↵ to commit)".into(),
        };
        let edit = ui.add(
            egui::TextEdit::multiline(&mut rv.commit_msg)
                .hint_text(hint)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .id_salt("commit_msg"),
        );
        if std::mem::take(&mut rv.focus_commit_msg) {
            edit.request_focus();
        }
        if edit.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            actions.push(Action::Commit);
        }
        ui.horizontal(|ui| {
            let can = (!staged.is_empty() || rv.amend) && !rv.commit_msg.trim().is_empty();
            if ui.add_enabled(can, egui::Button::new(if rv.amend { "Amend commit" } else { "Commit" })).clicked() {
                actions.push(Action::Commit);
            }
            if ui.checkbox(&mut rv.amend, "Amend").changed() && rv.amend && rv.commit_msg.trim().is_empty() {
                rv.commit_msg = rv.repo.last_commit_message().unwrap_or_default();
            }
            if let Some(t) = &task {
                if !rv.commit_msg.contains(t.as_str()) && ui.small_button(format!("Insert {t}")).clicked() {
                    rv.commit_msg = format!("{t}: {}", rv.commit_msg);
                }
            }
        });
        ui.separator();
    });

    let mode = match &sel {
        Some((_, true)) => DiffMode::Staged,
        _ => DiffMode::Unstaged,
    };
    if sel.is_none() {
        ui.weak("Select a file to see its diff.");
        return;
    }
    if let Some(a) = rv.work_diff.show(ui, mode, "workdiff") {
        let action = Action::Hunk { file: a.file, hunk: a.hunk, op: a.op, staged_view: mode == DiffMode::Staged };
        if a.op == crate::app::HunkOp::Discard {
            actions.push(Action::Confirm { msg: "Discard this hunk? This cannot be undone.".into(), action: Box::new(action) });
        } else {
            actions.push(action);
        }
    }
}

fn file_row(ui: &mut Ui, f: &FileChange, staged: bool, sel: Option<&(String, bool)>, actions: &mut Vec<Action>) {
    let code = if f.conflicted {
        'U'
    } else if staged {
        f.index
    } else if f.is_untracked() {
        '?'
    } else {
        f.worktree
    };
    let is_sel = sel.is_some_and(|(p, s)| *p == f.path && *s == staged);
    ui.horizontal(|ui| {
        ui.label(RichText::new(code.to_string()).monospace().color(theme::status_color(code)));
        let label = match &f.orig_path {
            Some(o) => format!("{o} → {}", f.path),
            None => f.path.clone(),
        };
        let resp = ui.selectable_label(is_sel, label);
        if resp.clicked() {
            actions.push(Action::SelectFile { path: f.path.clone(), staged });
        }
        if resp.double_clicked() {
            actions.push(if staged { Action::Unstage(vec![f.path.clone()]) } else { Action::Stage(vec![f.path.clone()]) });
        }
        let p = f.path.clone();
        resp.context_menu(|ui| {
            if staged {
                if ui.button("Unstage").clicked() {
                    actions.push(Action::Unstage(vec![p.clone()]));
                    ui.close();
                }
            } else {
                if ui.button("Stage").clicked() {
                    actions.push(Action::Stage(vec![p.clone()]));
                    ui.close();
                }
                if ui.button("Discard changes…").clicked() {
                    actions.push(Action::Confirm { msg: format!("Discard changes to {p}?"), action: Box::new(Action::Discard(vec![p.clone()])) });
                    ui.close();
                }
            }
            if ui.button("Copy path").clicked() {
                actions.push(Action::Copy(p.clone()));
                ui.close();
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (label, a) = if staged { ("−", Action::Unstage(vec![f.path.clone()])) } else { ("+", Action::Stage(vec![f.path.clone()])) };
            if ui.small_button(label).on_hover_text(if staged { "Unstage" } else { "Stage" }).clicked() {
                actions.push(a);
            }
        });
    });
}

fn commit(ui: &mut Ui, rv: &mut RepoView, actions: &mut Vec<Action>) {
    let Some(d) = rv.commit_detail.clone() else {
        ui.spinner();
        return;
    };
    egui::Panel::top("commit_header").show(ui, |ui| {
        let mut lines = d.message.lines();
        ui.label(RichText::new(lines.next().unwrap_or("")).strong().size(ui.style().text_styles[&egui::TextStyle::Body].size + 2.0));
        let body: String = lines.collect::<Vec<_>>().join("\n");
        if !body.trim().is_empty() {
            ui.label(body.trim());
        }
        ui.add_space(4.0);
        egui::Grid::new("commit_meta").num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
            ui.weak("Commit");
            ui.horizontal(|ui| {
                ui.monospace(&d.id[..12.min(d.id.len())]);
                if ui.small_button("Copy").clicked() {
                    actions.push(Action::Copy(d.id.clone()));
                }
            });
            ui.end_row();
            ui.weak("Author");
            ui.label(format!("{} <{}>  ·  {} UTC", d.author, d.email, margin_core::format_utc(d.author_time)));
            ui.end_row();
            if d.committer != d.author {
                ui.weak("Committer");
                ui.label(format!("{}  ·  {} UTC", d.committer, margin_core::format_utc(d.commit_time)));
                ui.end_row();
            }
            if !d.parents.is_empty() {
                ui.weak("Parents");
                ui.horizontal(|ui| {
                    for p in &d.parents {
                        if ui.link(RichText::new(&p[..8.min(p.len())]).monospace()).clicked() {
                            actions.push(Action::JumpTo(p.clone()));
                        }
                    }
                });
                ui.end_row();
            }
            let keys = margin_core::tasks::extract_keys(&d.message);
            if !keys.is_empty() {
                ui.weak("Tasks");
                ui.horizontal(|ui| {
                    for k in keys {
                        if ui.link(RichText::new(&k).color(theme::ACCENT)).clicked() {
                            actions.push(Action::FocusTask(k));
                        }
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(4.0);
    });
    rv.commit_diff.show(ui, DiffMode::Commit, "commitdiff");
}
