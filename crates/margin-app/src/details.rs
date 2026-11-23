//! Right-hand pane: working-directory staging + commit box, or commit details.

use crate::app::{Action, RepoView, Sel};
use crate::diff_view::{DiffEvent, DiffMode};
use crate::theme;
use eframe::egui::{self, RichText, Ui};

/// `prefs`: diff layout and whitespace (the diff bar changes them);
/// `task_url`: link for a task key, if one is configured.
/// Looked-up tasks (titles, status), by `app::task_cache_key`.
pub type TaskCache = std::collections::HashMap<String, Result<margin_core::tasks::Task, String>>;

pub fn show(ui: &mut Ui, rv: &mut RepoView, prefs: &mut crate::diff_view::DiffPrefs, task_url: &dyn Fn(&str) -> Option<String>, tasks: &TaskCache, actions: &mut Vec<Action>) {
    match rv.sel.clone() {
        Sel::Working => working(ui, rv, prefs, actions),
        Sel::Commit(_) => commit(ui, rv, prefs, task_url, tasks, actions),
    }
}

fn working(ui: &mut Ui, rv: &mut RepoView, prefs: &mut crate::diff_view::DiffPrefs, actions: &mut Vec<Action>) {
    let Some(snap) = rv.snap.as_ref() else { return };
    let staged_n = snap.status.files.iter().filter(|f| f.is_staged()).count();
    let changed = snap.status.files.iter().any(|f| !f.conflicted);
    ui.vertical(|ui| {
        // ---- commit box
        let task = rv.current_branch().and_then(|b| rv.notes.task_for_branch_in(b, rv.task_style));
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
        // (wraps in a narrow column instead of widening it)
        ui.horizontal_wrapped(|ui| {
            let has_msg = !rv.commit_msg.trim().is_empty();
            let can = (staged_n > 0 || rv.amend) && has_msg;
            // Say what the button will do: "Commit 2 files" / "Amend last commit".
            let label = if rv.amend {
                "Amend last commit".to_string()
            } else if staged_n > 0 {
                format!("Commit {staged_n} file{}", if staged_n == 1 { "" } else { "s" })
            } else {
                "Commit".to_string()
            };
            let enter = if cfg!(target_os = "macos") { "⌘↵" } else { "Ctrl+↵" };
            if ui.add_enabled_ui(can, |ui| crate::widgets::primary_button(ui, &label)).inner.on_hover_text(format!("Commit the staged changes ({enter})")).clicked() {
                actions.push(Action::Commit);
            }
            if ui.checkbox(&mut rv.amend, "Amend").changed() && rv.amend && rv.commit_msg.trim().is_empty() {
                rv.commit_msg = rv.repo.last_commit_message().unwrap_or_default();
            }
            // Why the button is off, in words.
            if !can {
                let why = if staged_n == 0 && !rv.amend { "Stage changes below to commit them" } else { "Write a summary to commit" };
                ui.label(RichText::new(why).small().color(theme::TEXT_DIM()));
            }
            if let Some(t) = &task {
                if !rv.commit_msg.contains(t.as_str()) && ui.small_button(format!("Insert {t}")).clicked() {
                    rv.commit_msg = format!("{t}: {}", rv.commit_msg);
                }
            }
            // Summary length: git convention is ≤ 50 (soft) and ≤ 72 (hard).
            let n = summary.chars().count();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if changed && ui.small_button("Stash…").on_hover_text("Put changes aside (all, only staged, or some files) to get back to later").clicked() {
                    actions.push(Action::StashDialog { paths: vec![] });
                }
                if n > 0 {
                    let color = if n > 72 { theme::ERROR() } else if n > 50 { theme::ACCENT() } else { theme::TEXT_DIM() };
                    ui.label(RichText::new(format!("{n}")).small().color(color))
                        .on_hover_text("Summary length. Keep it under 50 characters if you can (72 at most).");
                }
            });
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
            DiffEvent::Resolve(p) => Action::OpenConflicts(Some(p)),
            DiffEvent::Stash(p) => Action::StashDialog { paths: p },
            DiffEvent::Discard(p) => {
                let what = if p.len() == 1 { p[0].clone() } else { format!("{} files", p.len()) };
                Action::Confirm { msg: format!("Discard changes to {what}? Untracked files are deleted. This cannot be undone."), action: Box::new(Action::Discard(p)) }
            }
            DiffEvent::CopyPath(p) => Action::Copy(p),
            DiffEvent::ToggleFocus => Action::ToggleFocusDiff,
            DiffEvent::OpenUrl(u) => Action::OpenUrl(u),
            // (no review threads in the working directory)
            DiffEvent::ThreadReply { .. } | DiffEvent::ThreadResolve { .. } | DiffEvent::ThreadNew { .. } | DiffEvent::PendingRemove { .. } => continue,
        });
    }
}

fn commit(ui: &mut Ui, rv: &mut RepoView, prefs: &mut crate::diff_view::DiffPrefs, task_url: &dyn Fn(&str) -> Option<String>, tasks: &TaskCache, actions: &mut Vec<Action>) {
    // a stash: what it holds, and what to do with it
    let stash = match &rv.sel {
        Sel::Commit(id) => rv.snap.as_ref().and_then(|s| s.stashes.iter().find(|st| &st.id == id)).cloned(),
        _ => None,
    };
    if let Some(st) = stash {
        stash_view(ui, rv, &st, prefs, actions);
        return;
    }
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
    // How many files were added / deleted / renamed (modified ones are the rest).
    let kinds = rv.commit_diff.files.iter().fold([0usize; 3], |mut k, f| {
        match (&f.old_path, &f.new_path) {
            (None, Some(_)) => k[0] += 1,
            (Some(_), None) => k[1] += 1,
            (Some(o), Some(n)) if o != n => k[2] += 1,
            _ => {}
        }
        k
    });
    let (title, _) = split_message(&d.message);
    let title = format!("{}  {}", &d.id[..8.min(d.id.len())], title);
    // Task keys in the message, with what's known about them (looked up once)
    let task_rows: Vec<(String, Option<Result<margin_core::tasks::Task, String>>)> = margin_core::tasks::keys_in(&d.message, rv.task_style)
        .into_iter()
        .map(|k| {
            let info = tasks.get(&crate::app::task_cache_key(rv, &k)).cloned();
            if info.is_none() && rv.editor.requested_tasks.insert(k.clone()) {
                actions.push(Action::FetchTask(k.clone()));
            }
            (k, info)
        })
        .collect();
    let mut header = |ui: &mut Ui| {
        // ---- metadata first
        let when = |t: i64| format!("{} {}", margin_core::format_local(t), margin_core::local_zone(t));
        egui::Grid::new("commit_meta").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            ui.weak("Commit");
            ui.horizontal(|ui| {
                // interface-sized (not the code size)
                let ui_size = ui.style().text_styles[&egui::TextStyle::Body].size;
                ui.label(RichText::new(&d.id[..12.min(d.id.len())]).family(egui::FontFamily::Monospace).size(ui_size - 0.5));
                // Copy ▾: the SHA in either length, the message, or both
                ui.menu_button(RichText::new("Copy ▾").small(), |ui| {
                    let (subject, _) = split_message(&d.message);
                    let short = d.id[..8.min(d.id.len())].to_string();
                    for (label, text) in [
                        ("Full SHA", d.id.clone()),
                        ("Short SHA", short.clone()),
                        ("Message", d.message.trim_end().to_string()),
                        ("Short SHA and title", format!("{short} {subject}")),
                    ] {
                        if ui.button(label).clicked() {
                            actions.push(Action::Copy(text));
                            ui.close();
                        }
                    }
                });
            });
            ui.end_row();
            // name, then the email and the time (with how long ago) quieter
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_secs() as i64);
            ui.weak("Author");
            ui.horizontal_wrapped(|ui| {
                ui.label(&d.author);
                ui.label(RichText::new(&d.email).small().color(theme::TEXT_DIM()));
                ui.label(RichText::new(format!("·  {}  ·  {}", when(d.author_time), crate::commit_list::rel_time(now, d.author_time))).color(theme::TEXT_DIM()));
            });
            ui.end_row();
            if d.committer != d.author || d.commit_time != d.author_time {
                ui.weak("Committer");
                ui.label(format!("{}  ·  {}", d.committer, when(d.commit_time)));
                ui.end_row();
            }
            if !d.parents.is_empty() {
                ui.weak("Parents");
                // each parent: its short SHA (a link) and its title
                ui.vertical(|ui| {
                    for p in &d.parents {
                        ui.horizontal(|ui| {
                            let ui_size = ui.style().text_styles[&egui::TextStyle::Body].size;
                            if ui.link(RichText::new(&p[..8.min(p.len())]).family(egui::FontFamily::Monospace).size(ui_size - 0.5)).on_hover_text("Go to this commit").clicked() {
                                actions.push(Action::JumpTo(p.clone()));
                            }
                            let subject = rv.snap.as_ref().and_then(|s| s.commits.iter().find(|c| &c.id == p)).map(|c| c.subject.clone());
                            if let Some(sj) = subject {
                                ui.add(egui::Label::new(RichText::new(sj).color(theme::TEXT_DIM())).truncate());
                            }
                        });
                    }
                });
                ui.end_row();
            }
            // Branches whose history contains this commit (local first).
            if !branches.is_empty() {
                ui.weak("Branches");
                let all = branches.join(", ");
                // chips, like the labels in the history; "+N more" expands in place
                let all_id = egui::Id::unique(("commit-branches-all", &d.id));
                let show_all: bool = ui.data(|x| x.get_temp(all_id)).unwrap_or(false);
                // (wrapped inside a vertical block, so the grid row grows with
                // the second line instead of the chips overlapping the next row)
                ui.vertical(|ui| ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    const SHOW: usize = 6;
                    let n = if show_all { branches.len() } else { SHOW };
                    for b in branches.iter().take(n) {
                        let chip = egui::Button::new(RichText::new(b).color(theme::TEXT())).fill(theme::BG_DARK()).corner_radius(4.0);
                        if ui.add(chip).on_hover_text("Show this branch").clicked() {
                            actions.push(Action::GotoRev(b.clone()));
                        }
                    }
                    let more = branches.len().saturating_sub(SHOW);
                    if more > 0 {
                        let label = if show_all { "Show fewer".to_string() } else { format!("+{more} more") };
                        if ui.link(RichText::new(label).color(theme::TEXT_DIM())).on_hover_text(&all).clicked() {
                            ui.data_mut(|x| x.insert_temp(all_id, !show_all));
                        }
                    }
                }));
                ui.end_row();
            }
            // Changes: files and lines in this commit.
            let files = &files;
            if !files.is_empty() {
                let (a, r) = files.iter().fold((0, 0), |(a, r), f| (a + f.0, r + f.1));
                ui.weak("Changes");
                ui.horizontal(|ui| {
                    ui.label(format!("{} file{}", files.len(), if files.len() == 1 { "" } else { "s" }));
                    if a > 0 {
                        ui.label(RichText::new(format!("+{a}")).color(theme::ADDED()));
                    }
                    if r > 0 {
                        ui.label(RichText::new(format!("−{r}")).color(theme::REMOVED()));
                    }
                    // "· 1 added · 1 deleted · 1 renamed"
                    for (n, word, color) in [(kinds[0], "added", theme::ADDED()), (kinds[1], "deleted", theme::REMOVED()), (kinds[2], "renamed", theme::lane(0))] {
                        if n > 0 {
                            ui.label(RichText::new("·").color(theme::TEXT_DIM()));
                            ui.label(RichText::new(format!("{n} {word}")).color(color));
                        }
                    }
                });
                ui.end_row();
            }
            // Tasks: each key links to its ticket, with its title and status
            // once looked up (Jira / GitHub, through Settings → Accounts).
            if !task_rows.is_empty() {
                ui.weak("Tasks");
                ui.vertical(|ui| {
                    for (k, info) in &task_rows {
                        let info = info.as_ref();
                        ui.horizontal(|ui| {
                            match task_url(k) {
                                Some(u) => {
                                    if ui.link(RichText::new(format!("{k} ↗")).color(theme::ACCENT())).on_hover_text(format!("Open {u}")).clicked() {
                                        actions.push(Action::OpenUrl(u));
                                    }
                                }
                                None => {
                                    ui.label(RichText::new(k).color(theme::ACCENT()));
                                    if ui.link(RichText::new("Set up tasks…").small().color(theme::TEXT_DIM())).on_hover_text("Choose where this repository's tasks come from").clicked() {
                                        actions.push(Action::OpenRepoSettings);
                                    }
                                }
                            }
                            match info {
                                Some(Ok(t)) => {
                                    let (fg, bg) = match t.status_category.as_str() {
                                        "done" => (theme::ADDED(), theme::ADDED().gamma_multiply(0.15)),
                                        "indeterminate" => (theme::lane(0), theme::lane(0).gamma_multiply(0.15)),
                                        _ => (theme::TEXT_DIM(), theme::BG_DARK()),
                                    };
                                    if !t.status.is_empty() {
                                        egui::Frame::new().fill(bg).corner_radius(4.0).inner_margin(egui::Margin::symmetric(5, 1)).show(ui, |ui| {
                                            ui.label(RichText::new(&t.status).small().color(fg));
                                        });
                                    }
                                    let mut tip = t.summary.clone();
                                    if let Some(a) = &t.assignee {
                                        tip.push_str(&format!("\nAssignee: {a}"));
                                    }
                                    ui.add(egui::Label::new(RichText::new(&t.summary).color(theme::TEXT())).truncate()).on_hover_text(tip);
                                }
                                Some(Err(e)) => {
                                    ui.label(RichText::new("couldn't load").small().color(theme::ERROR())).on_hover_text(e);
                                }
                                None => {}
                            }
                        });
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
            DiffEvent::ThreadReply { id, branch, body } => actions.push(Action::ReplyThread { id, branch, body }),
            DiffEvent::ThreadResolve { id, branch, resolve } => actions.push(Action::ResolveThread { id, branch, resolve }),
            DiffEvent::ThreadNew { branch, path, line, old_side, body } => actions.push(Action::NewThread { branch, path, line, old_side, body }),
            _ => {}
        }
    }
}

/// A stash: its message, where and when it was made, Apply / Pop / Drop /
/// New branch, then what it holds (untracked files included).
fn stash_view(ui: &mut Ui, rv: &mut RepoView, st: &margin_core::git::Stash, prefs: &mut crate::diff_view::DiffPrefs, actions: &mut Vec<Action>) {
    let files: Vec<(usize, usize)> = rv.commit_diff.files.iter().map(|f| (f.added, f.removed)).collect();
    let title = format!("Stash  {}", st.title);
    let mut header = |ui: &mut Ui| {
        let now = margin_core::time_now();
        ui.horizontal(|ui| {
            ui.label(RichText::new("STASH").small().strong().color(theme::lane(3)));
            ui.label(RichText::new(&st.name).small().monospace().color(theme::TEXT_DIM()));
        });
        let big = crate::theme::content_size() + 2.0;
        ui.add(egui::Label::new(RichText::new(&st.title).strong().size(big)).selectable(true).wrap());
        ui.horizontal_wrapped(|ui| {
            let mut meta = String::new();
            if !st.branch.is_empty() {
                meta = format!("made on {} · ", st.branch);
            }
            meta += &crate::commit_list::rel_time(now, st.time);
            let (a, r) = files.iter().fold((0, 0), |(a, r), f| (a + f.0, r + f.1));
            ui.label(RichText::new(meta).color(theme::TEXT_DIM()));
            ui.label(RichText::new(format!("· {} file{}", files.len(), if files.len() == 1 { "" } else { "s" })).color(theme::TEXT_DIM()));
            if a > 0 {
                ui.label(RichText::new(format!("+{a}")).color(theme::ADDED()));
            }
            if r > 0 {
                ui.label(RichText::new(format!("−{r}")).color(theme::REMOVED()));
            }
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let name = st.name.clone();
            if crate::widgets::primary_button(ui, "Pop").on_hover_text("Apply it to the working directory and remove the stash").clicked() {
                actions.push(Action::StashPop(name.clone()));
            }
            if ui.button("Apply").on_hover_text("Apply it and keep the stash").clicked() {
                actions.push(Action::StashApply(name.clone()));
            }
            ui.menu_button("More ▾", |ui| {
                if ui.button("Apply, restoring what was staged").on_hover_text("Changes that were staged when you stashed come back staged").clicked() {
                    let n = name.clone();
                    actions.push(Action::RunGit { label: "Stash apply".into(), f: std::sync::Arc::new(move |r| r.stash_apply_index(&n).map(|_| String::new())) });
                    ui.close();
                }
                if ui.button("New branch from stash…").on_hover_text("A branch at the commit it was made on, with the stash applied (and removed)").clicked() {
                    actions.push(Action::StashBranchDialog(name.clone()));
                    ui.close();
                }
                if ui.button("Copy message").clicked() {
                    actions.push(Action::Copy(st.title.clone()));
                    ui.close();
                }
            });
            if ui.button(RichText::new("Drop…").color(theme::ERROR())).on_hover_text("Delete the stash").clicked() {
                actions.push(Action::Confirm { msg: format!("Drop the stash “{}”? It can't be recovered easily.", st.title), action: Box::new(Action::StashDrop(name)) });
            }
        });
        ui.add_space(4.0);
        ui.separator();
    };
    if rv.commit_diff.loading && rv.commit_diff.files.is_empty() {
        header(ui);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0));
            ui.weak("Loading stash…");
        });
        return;
    }
    for e in rv.commit_diff.show(ui, DiffMode::Commit, "stashdiff", prefs, Some(crate::diff_view::Header { ui: &mut header, title })) {
        match e {
            DiffEvent::ToggleFocus => actions.push(Action::ToggleFocusDiff),
            DiffEvent::OpenUrl(u) => actions.push(Action::OpenUrl(u)),
            DiffEvent::CopyPath(p) => actions.push(Action::Copy(p)),
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
