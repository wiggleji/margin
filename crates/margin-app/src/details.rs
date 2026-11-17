//! Right-hand pane: working-directory staging + commit box, or commit details.

use crate::app::{Action, RepoView, Sel};
use crate::diff_view::{DiffEvent, DiffMode};
use crate::theme;
use eframe::egui::{self, RichText, Ui};

/// `prefs`: diff layout and whitespace (the diff bar changes them);
/// `task_url`: link for a task key, if one is configured.
pub fn show(ui: &mut Ui, rv: &mut RepoView, prefs: &mut crate::diff_view::DiffPrefs, task_url: &dyn Fn(&str) -> Option<String>, actions: &mut Vec<Action>) {
    match rv.sel.clone() {
        Sel::Working => working(ui, rv, prefs, actions),
        Sel::Commit(_) => commit(ui, rv, prefs, task_url, actions),
    }
}

fn working(ui: &mut Ui, rv: &mut RepoView, prefs: &mut crate::diff_view::DiffPrefs, actions: &mut Vec<Action>) {
    let Some(snap) = rv.snap.as_ref() else { return };
    let staged_n = snap.status.files.iter().filter(|f| f.is_staged()).count();
    ui.vertical(|ui| {
        // ---- commit box
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
            let can = (staged_n > 0 || rv.amend) && !rv.commit_msg.trim().is_empty();
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
                    let color = if n > 72 { theme::ERROR() } else if n > 50 { theme::ACCENT() } else { theme::TEXT_DIM() };
                    ui.label(RichText::new(format!("{n}")).small().color(color))
                        .on_hover_text("Summary length. Keep it under 50 characters if you can (72 at most).");
                });
            }
        });
        ui.separator();
    });

    // All changed files: conflicts, then Unstaged and Staged sections; each
    // file expands to its diff (defaults in Settings → Diffs).
    for e in rv.work_diff.show(ui, DiffMode::Working, "workdiff", prefs, None) {
        actions.push(match e {
            DiffEvent::Hunk(a) => {
                let staged_view = rv.work_diff.section(a.file) == Some(crate::diff_view::Section::Staged);
                let action = Action::Hunk { file: a.file, hunk: a.hunk, op: a.op, staged_view };
                if a.op == crate::app::HunkOp::Discard {
                    Action::Confirm { msg: "Discard this hunk? This cannot be undone.".into(), action: Box::new(action) }
                } else {
                    action
                }
            }
            DiffEvent::Stage(p) => Action::Stage(p),
            DiffEvent::Unstage(p) => Action::Unstage(p),
            DiffEvent::Discard(p) => {
                let what = if p.len() == 1 { p[0].clone() } else { format!("{} files", p.len()) };
                Action::Confirm { msg: format!("Discard changes to {what}? Untracked files are deleted. This cannot be undone."), action: Box::new(Action::Discard(p)) }
            }
            DiffEvent::CopyPath(p) => Action::Copy(p),
            DiffEvent::ToggleFocus => Action::ToggleFocusDiff,
            DiffEvent::OpenUrl(u) => Action::OpenUrl(u),
        });
    }
}

fn commit(ui: &mut Ui, rv: &mut RepoView, prefs: &mut crate::diff_view::DiffPrefs, task_url: &dyn Fn(&str) -> Option<String>, actions: &mut Vec<Action>) {
    let Some(d) = rv.commit_detail.clone() else {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0));
            ui.weak("Loading commit…");
        });
        return;
    };
    // The metadata and message scroll with the diff (one page; the diff's
    // toolbar sticks to the top and shows the title once they're out of view).
    let branches = rv.commit_branches.clone();
    let files: Vec<(usize, usize)> = rv.commit_diff.files.iter().map(|f| (f.added, f.removed)).collect();
    let (title, _) = split_message(&d.message);
    let title = format!("{}  {}", &d.id[..8.min(d.id.len())], title);
    let mut header = |ui: &mut Ui| {
        // ---- metadata first
        let when = |t: i64| format!("{} {}", margin_core::format_local(t), margin_core::local_zone(t));
        egui::Grid::new("commit_meta").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            ui.weak("Commit");
            ui.horizontal(|ui| {
                // interface-sized (not the code size)
                let ui_size = ui.style().text_styles[&egui::TextStyle::Body].size;
                ui.label(RichText::new(&d.id[..12.min(d.id.len())]).family(egui::FontFamily::Monospace).size(ui_size - 0.5));
                if ui.small_button("Copy").clicked() {
                    actions.push(Action::Copy(d.id.clone()));
                }
            });
            ui.end_row();
            ui.weak("Author");
            ui.label(format!("{} <{}>  ·  {}", d.author, d.email, when(d.author_time)));
            ui.end_row();
            if d.committer != d.author || d.commit_time != d.author_time {
                ui.weak("Committer");
                ui.label(format!("{}  ·  {}", d.committer, when(d.commit_time)));
                ui.end_row();
            }
            if !d.parents.is_empty() {
                ui.weak("Parents");
                ui.horizontal(|ui| {
                    for p in &d.parents {
                        let ui_size = ui.style().text_styles[&egui::TextStyle::Body].size;
                        if ui.link(RichText::new(&p[..8.min(p.len())]).family(egui::FontFamily::Monospace).size(ui_size - 0.5)).clicked() {
                            actions.push(Action::JumpTo(p.clone()));
                        }
                    }
                });
                ui.end_row();
            }
            // Branches whose history contains this commit (local first).
            if !branches.is_empty() {
                ui.weak("Branches");
                let all = branches.join(", ");
                ui.horizontal_wrapped(|ui| {
                    const SHOW: usize = 6;
                    for b in branches.iter().take(SHOW) {
                        if ui.link(RichText::new(b).small()).on_hover_text("Show this branch").clicked() {
                            actions.push(Action::GotoRev(b.clone()));
                        }
                    }
                    let more = branches.len().saturating_sub(SHOW);
                    if more > 0 {
                        ui.label(RichText::new(format!("+{more} more")).small().color(theme::TEXT_DIM())).on_hover_text(&all);
                    }
                });
                ui.end_row();
            }
            // Changes: files and lines in this commit.
            let files = &files;
            if !files.is_empty() {
                let (a, r) = files.iter().fold((0, 0), |(a, r), f| (a + f.0, r + f.1));
                ui.weak("Changes");
                ui.horizontal(|ui| {
                    ui.label(format!("{} file{}", files.len(), if files.len() == 1 { "" } else { "s" }));
                    ui.label(RichText::new(format!("+{a}")).color(theme::ADDED()));
                    ui.label(RichText::new(format!("−{r}")).color(theme::REMOVED()));
                });
                ui.end_row();
            }
            // Tasks: click the key for its notes; ↗ opens the ticket.
            let keys = margin_core::tasks::extract_keys(&d.message);
            if !keys.is_empty() {
                ui.weak("Tasks");
                ui.horizontal_wrapped(|ui| {
                    for k in keys {
                        if ui.link(RichText::new(&k).color(theme::ACCENT())).on_hover_text("Show the task's notes").clicked() {
                            actions.push(Action::FocusTask(k.clone()));
                        }
                        if let Some(u) = task_url(&k) {
                            if ui.small_button("↗").on_hover_text(format!("Open {u}")).clicked() {
                                actions.push(Action::OpenUrl(u));
                            }
                        }
                        ui.add_space(6.0);
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        ui.separator();
        // ---- then the message: subject, and the body (selectable; scrolls when long)
        let (subject, body) = split_message(&d.message);
        // the message is reading text: code / notes size
        let size = crate::theme::content_size();
        let big = size + 2.0;
        // Roomier line spacing for reading (wrapped lines and the body).
        ui.add(egui::Label::new(RichText::new(subject).strong().size(big).line_height(Some(big * 1.4))).selectable(true).wrap());
        let body = body.trim_end();
        if !body.is_empty() {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(body).color(theme::TEXT()).line_height(Some(size * 1.55))).selectable(true).wrap());

        }
        ui.add_space(4.0);
    };
    for e in rv.commit_diff.show(ui, DiffMode::Commit, "commitdiff", prefs, Some(crate::diff_view::Header { ui: &mut header, title })) {
        match e {
            DiffEvent::ToggleFocus => actions.push(Action::ToggleFocusDiff),
            DiffEvent::OpenUrl(u) => actions.push(Action::OpenUrl(u)),
            _ => {}
        }
    }
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
