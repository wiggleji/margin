//! Right-hand pane: working-directory staging + commit box, or commit details.

use crate::app::{Action, RepoView, Sel};
use crate::diff_view::DiffMode;
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::git::FileChange;

/// `prefs`: diff layout and whitespace (the diff bar changes them).
pub fn show(ui: &mut Ui, rv: &mut RepoView, prefs: &mut crate::diff_view::DiffPrefs, actions: &mut Vec<Action>) {
    match rv.sel.clone() {
        Sel::Working => working(ui, rv, prefs, actions),
        Sel::Commit(_) => commit(ui, rv, prefs, actions),
    }
}

fn working(ui: &mut Ui, rv: &mut RepoView, prefs: &mut crate::diff_view::DiffPrefs, actions: &mut Vec<Action>) {
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
        let enter = if cfg!(target_os = "macos") { "⌘↵" } else { "Ctrl+↵" };
        let hint = match &task {
            Some(t) => format!("{t}: summary   ({enter} to commit)"),
            None => format!("Summary   ({enter} to commit)"),
        };
        let (sum_id, body_id) = (egui::Id::unique("commit_summary"), egui::Id::unique("commit_body"));
        // ⌘↵ from either field commits (checked before the fields use the key).
        let typing = ui.memory(|m| m.has_focus(sum_id) || m.has_focus(body_id));
        if typing && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            actions.push(Action::Commit);
        }
        // One message, edited as a summary line + description.
        let (mut summary, mut body) = split_message(&rv.commit_msg);
        let s = ui.add(egui::TextEdit::singleline(&mut summary).hint_text(hint).desired_width(f32::INFINITY).id(sum_id));
        let b = ui.add(
            egui::TextEdit::multiline(&mut body)
                .hint_text("Description (optional)")
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .id(body_id),
        );
        if s.changed() || b.changed() {
            rv.commit_msg = join_message(&summary, &body);
        }
        // ↵ in the summary moves on to the description.
        if s.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.command) {
            b.request_focus();
        }
        if std::mem::take(&mut rv.focus_commit_msg) {
            s.request_focus();
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
            // Summary length: git convention is ≤ 50 (soft) and ≤ 72 (hard).
            let n = summary.chars().count();
            if n > 0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let color = if n > 72 { theme::ERROR } else if n > 50 { theme::ACCENT } else { theme::TEXT_DIM };
                    ui.label(RichText::new(format!("{n}")).small().color(color))
                        .on_hover_text("Summary length. Keep it under 50 characters if you can (72 at most).");
                });
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
    if let Some(a) = rv.work_diff.show(ui, mode, "workdiff", prefs) {
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

fn commit(ui: &mut Ui, rv: &mut RepoView, prefs: &mut crate::diff_view::DiffPrefs, actions: &mut Vec<Action>) {
    let Some(d) = rv.commit_detail.clone() else {
        ui.spinner();
        return;
    };
    egui::Panel::top("commit_header").show(ui, |ui| {
        // Full message: subject, then the body (selectable; scrolls when long).
        let (subject, body) = split_message(&d.message);
        let big = ui.style().text_styles[&egui::TextStyle::Body].size + 2.0;
        ui.add(egui::Label::new(RichText::new(subject).strong().size(big)).selectable(true).wrap());
        let body = body.trim_end();
        if !body.is_empty() {
            ui.add_space(2.0);
            let max = (ui.ctx().content_rect().height() * 0.3).max(80.0);
            egui::ScrollArea::vertical().id_salt("commit_body_scroll").max_height(max).auto_shrink([false, true]).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(body).color(theme::TEXT)).selectable(true).wrap());
            });
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
    rv.commit_diff.show(ui, DiffMode::Commit, "commitdiff", prefs);
}

/// "Summary\n\nBody" → ("Summary", "Body").
pub fn split_message(m: &str) -> (String, String) {
    match m.split_once('\n') {
        Some((s, rest)) => (s.to_string(), rest.trim_start_matches(['\n', '\r']).to_string()),
        None => (m.to_string(), String::new()),
    }
}

/// Inverse of `split_message`: a blank line between summary and body.
pub fn join_message(summary: &str, body: &str) -> String {
    if body.is_empty() { summary.to_string() } else { format!("{summary}\n\n{body}") }
}

#[cfg(test)]
mod tests {
    use super::{join_message, split_message};

    #[test]
    fn message_round_trip() {
        for m in ["Fix login", "Fix login\n\nRaised the timeout.\n\n- tested", "Subject\n\n"] {
            let (s, b) = split_message(m);
            assert_eq!(split_message(&join_message(&s, &b)), (s, b));
        }
        assert_eq!(split_message("A\nB"), ("A".into(), "B".into()));
        assert_eq!(join_message("A", "B"), "A\n\nB");
    }
}
