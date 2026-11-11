use crate::app::{Action, RepoView, Sel};
use crate::icons::{self, Icon};
use crate::notes_panel::NoteTab;
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use crate::branch_tree::{self, Node};
use margin_core::git::{RefInfo, RefKind};
use margin_core::notes::NoteTarget;
use margin_core::tasks::Task;
use std::collections::BTreeMap;

pub fn show(ui: &mut Ui, rv: &mut RepoView, tasks_enabled: bool, my_tasks: Option<&Result<Vec<Task>, String>>, actions: &mut Vec<Action>) {
    let Some(snap) = rv.snap.as_ref() else { return };
    let ctx = ui.ctx().clone();
    let isz = ui.text_style_height(&egui::TextStyle::Small) + 1.0;
    let note_icon = |tint| icons::image(&ctx, Icon::Note, isz, tint);
    egui::ScrollArea::vertical().id_salt("sidebar").auto_shrink([false, false]).show(ui, |ui| {
        // One line per row: truncate with "…" instead of wrapping.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.add_space(4.0);
        let n = snap.status.files.len();
        if ui.selectable_label(rv.sel == Sel::Working, format!("Working directory ({n})")).clicked() {
            actions.push(Action::Select(Sel::Working));
        }
        let repo_note = rv.has_note(&NoteTarget::Repo);
        let mut atoms = egui::Atoms::new("Repository notes");
        if repo_note {
            atoms.push_right(note_icon(theme::ACCENT));
        }
        if ui.selectable_label(false, atoms).clicked() {
            actions.push(Action::ShowNote(NoteTab::Repo));
        }
        ui.add_space(6.0);

        // ---------------------------------------------------------------- local
        // Grouped by prefix (feature/, hotfix/, projectA/…) when 2+ branches share one.
        egui::CollapsingHeader::new(RichText::new("BRANCHES").small().strong()).default_open(true).show(ui, |ui| {
            let tree = branch_tree::build(
                snap.refs.iter().filter(|r| r.kind == RefKind::Local).map(|r| (r.name.clone(), r)).collect(),
            );
            let focus = rv.focus_branch.clone();
            let open_if = |r: &&RefInfo| r.is_head || focus.as_deref() == Some(r.name.as_str());
            render_tree(ui, &tree, "local", &open_if, &mut |ui, label, r| local_row(ui, rv, r, label, isz, actions));
        });

        // ------------------------------------------------- recently re-attached
        if !rv.reattached.is_empty() {
            egui::CollapsingHeader::new(RichText::new(format!("RE-ATTACHED ({})", rv.reattached.len())).small().strong())
                .id_salt("reattached")
                .default_open(true)
                .show(ui, |ui| {
                    for e in &rv.reattached {
                        let short = |t: &str| t.strip_prefix("commit:").map(|c| c[..7.min(c.len())].to_string()).unwrap_or_else(|| t.to_string());
                        ui.horizontal(|ui| {
                            let mut atoms = egui::Atoms::new(note_icon(theme::ACCENT));
                            atoms.push_right(format!("{} → {}", short(&e.from), short(&e.to)));
                            let resp = ui.selectable_label(false, atoms).on_hover_text(format!(
                                "Moved automatically (same change, git patch-id) at {} UTC.\nClick to show the new commit.",
                                margin_core::format_utc(e.at)
                            ));
                            if resp.clicked() {
                                if let Some(c) = e.to.strip_prefix("commit:") {
                                    actions.push(Action::JumpTo(c.to_string()));
                                }
                            }
                            if ui.small_button("Undo").clicked() {
                                actions.push(Action::UndoReattach(e.id.clone()));
                            }
                            if ui.small_button("Keep").clicked() {
                                actions.push(Action::KeepReattach(e.id.clone()));
                            }
                        });
                    }
                });
        }

        // ------------------------------------------------------- detached notes
        if !rv.detached.is_empty() {
            egui::CollapsingHeader::new(RichText::new(format!("DETACHED NOTES ({})", rv.detached.len())).small().strong())
                .id_salt("detached")
                .default_open(true)
                .show(ui, |ui| {
                    for d in &rv.detached {
                        let mut atoms = egui::Atoms::new(note_icon(theme::TEXT_DIM));
                        atoms.push_right(crate::notes_panel::short_target(&d.note));
                        if !d.title.is_empty() {
                            let t: String = d.title.chars().take(28).collect();
                            atoms.push_right(RichText::new(t).small().color(theme::TEXT_DIM));
                        }
                        let open = rv.pinned_note.as_ref() == Some(&d.note);
                        let resp = ui.selectable_label(open, atoms).on_hover_text(format!(
                            "{}{}",
                            d.reason.describe(),
                            d.subject.as_ref().map(|s| format!("\nwas: {s}")).unwrap_or_default()
                        ));
                        if resp.clicked() {
                            actions.push(Action::OpenNote(d.note.clone()));
                        }
                        let t = d.note.clone();
                        resp.context_menu(|ui| {
                            if ui.button("Open").clicked() {
                                actions.push(Action::OpenNote(t.clone()));
                                ui.close();
                            }
                            if ui.button("Re-attach…").clicked() {
                                actions.push(Action::ReattachDialog(t.clone()));
                                ui.close();
                            }
                            if ui.button("Delete…").clicked() {
                                actions.push(Action::Confirm { msg: format!("Delete note {t}?"), action: Box::new(Action::DeleteNote(t.clone())) });
                                ui.close();
                            }
                        });
                    }
                });
        }

        // -------------------------------------------------------------- remotes
        let mut remotes: BTreeMap<&str, Vec<&RefInfo>> = BTreeMap::new();
        for r in snap.refs.iter().filter(|r| r.kind == RefKind::Remote) {
            let remote = r.name.split('/').next().unwrap_or("");
            remotes.entry(remote).or_default().push(r);
        }
        egui::CollapsingHeader::new(RichText::new("REMOTES").small().strong()).default_open(false).show(ui, |ui| {
            for (remote, refs) in remotes {
                egui::CollapsingHeader::new(remote).id_salt(("remote", remote)).show(ui, |ui| {
                    // Same prefix grouping as local branches, on the part after "origin/".
                    let tree = branch_tree::build(
                        refs.into_iter().map(|r| (r.name.split_once('/').map_or(r.name.clone(), |(_, b)| b.to_string()), r)).collect(),
                    );
                    render_tree(ui, &tree, remote, &|_| false, &mut |ui, label, r| {
                        let resp = ui.selectable_label(false, label);
                        if resp.clicked() {
                            actions.push(Action::JumpTo(r.target.clone()));
                        }
                        let name = r.name.clone();
                        resp.on_hover_text(&name).context_menu(|ui| {
                            if ui.button("Checkout as tracking branch").clicked() {
                                actions.push(Action::CheckoutRemote(name.clone()));
                                ui.close();
                            }
                            if ui.button("Merge into current branch").clicked() {
                                actions.push(Action::Confirm { msg: format!("Merge {name}?"), action: Box::new(Action::Merge(name.clone())) });
                                ui.close();
                            }
                        });
                    });
                });
            }
        });

        // ----------------------------------------------------------------- tags
        egui::CollapsingHeader::new(RichText::new("TAGS").small().strong()).default_open(false).show(ui, |ui| {
            for r in snap.refs.iter().filter(|r| r.kind == RefKind::Tag) {
                if ui.selectable_label(false, &r.name).clicked() {
                    actions.push(Action::JumpTo(r.target.clone()));
                }
            }
        });

        // -------------------------------------------------------------- stashes
        egui::CollapsingHeader::new(RichText::new(format!("STASHES ({})", snap.stashes.len())).small().strong())
            .id_salt("stashes")
            .default_open(false)
            .show(ui, |ui| {
                for s in &snap.stashes {
                    let resp = ui.selectable_label(false, &s.message);
                    if resp.clicked() {
                        actions.push(Action::JumpTo(s.id.clone()));
                    }
                    let name = s.name.clone();
                    resp.context_menu(|ui| {
                        for (label, a) in [
                            ("Apply", Action::StashApply(name.clone())),
                            ("Pop", Action::StashPop(name.clone())),
                        ] {
                            if ui.button(label).clicked() {
                                actions.push(a);
                                ui.close();
                            }
                        }
                        if ui.button("Drop…").clicked() {
                            actions.push(Action::Confirm { msg: format!("Drop {name}?"), action: Box::new(Action::StashDrop(name.clone())) });
                            ui.close();
                        }
                    });
                }
            });

        // ---------------------------------------------------------------- tasks
        if tasks_enabled {
            let resp = egui::CollapsingHeader::new(RichText::new("MY TASKS").small().strong()).id_salt("mytasks").default_open(false).show(ui, |ui| {
                match my_tasks {
                    None => {
                        ui.spinner();
                    }
                    Some(Err(e)) => {
                        ui.colored_label(theme::ERROR, e);
                    }
                    Some(Ok(list)) if list.is_empty() => {
                        ui.weak("Nothing assigned");
                    }
                    Some(Ok(list)) => {
                        for t in list {
                            let resp = ui
                                .selectable_label(rv.focus_task.as_deref() == Some(&t.key), format!("{}  {}", t.key, t.summary))
                                .on_hover_text(format!("{} · {}", t.status, t.issue_type));
                            if resp.clicked() {
                                actions.push(Action::FocusTask(t.key.clone()));
                            }
                            let key = t.key.clone();
                            let slug = slug(&t.summary);
                            resp.context_menu(|ui| {
                                if ui.button("Create branch for this task…").clicked() {
                                    actions.push(Action::NewBranchDialog { start: None, name: format!("feature/{key}-{slug}") });
                                    ui.close();
                                }
                                if ui.button("Open in browser").clicked() {
                                    actions.push(Action::OpenUrl(t.url.clone()));
                                    ui.close();
                                }
                            });
                        }
                        if ui.small_button("Refresh").clicked() {
                            actions.push(Action::FetchMyTasks);
                        }
                    }
                }
            });
            if resp.fully_open() && my_tasks.is_none() && !rv.loading {
                // first open: fetch lazily
                if ui.memory(|m| m.data.get_temp::<bool>(egui::Id::unique("mytasks_req")).is_none()) {
                    ui.memory_mut(|m| m.data.insert_temp(egui::Id::unique("mytasks_req"), true));
                    actions.push(Action::FetchMyTasks);
                }
            }
        }
    });
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for w in s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).take(5) {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w.to_ascii_lowercase());
    }
    out
}

/// Render a prefix tree: groups as collapsible "feature/  3" headers, leaves via `leaf`.
/// A group starts open if it contains an item matching `open_if`.
pub fn render_tree<'a>(
    ui: &mut Ui,
    nodes: &[Node<&'a RefInfo>],
    ns: &str,
    open_if: &dyn Fn(&&'a RefInfo) -> bool,
    leaf: &mut dyn FnMut(&mut Ui, &str, &'a RefInfo),
) {
    for n in nodes {
        match n {
            Node::Leaf { label, item } => leaf(ui, label, item),
            Node::Group { label, path, children, count } => {
                let mut job = egui::text::LayoutJob::default();
                let body = egui::TextStyle::Body.resolve(ui.style());
                let small = egui::TextStyle::Small.resolve(ui.style());
                job.append(label, 0.0, egui::TextFormat::simple(body, theme::TEXT));
                job.append(&format!("  {count}"), 0.0, egui::TextFormat::simple(small, theme::TEXT_DIM));
                egui::CollapsingHeader::new(job)
                    .id_salt(("branch-group", ns, path.as_str()))
                    .default_open(branch_tree::any(children, open_if))
                    .show(ui, |ui| render_tree(ui, children, ns, open_if, leaf));
            }
        }
    }
}

/// One local branch row: name (relative to its group), tracking, task, note, PR.
fn local_row(ui: &mut Ui, rv: &RepoView, r: &RefInfo, label: &str, isz: f32, actions: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    let task = rv.notes.task_for_branch(&r.name);
    let note = rv.has_note(&NoteTarget::Branch(r.name.clone()));
    let text = format!("{}{label}", if r.is_head { "● " } else { "   " });
    let track = r.track.replace("ahead ", "↑").replace("behind ", "↓").replace(", ", " ");
    let resp = ui
        .horizontal(|ui| {
            let mut rt = RichText::new(text);
            if r.is_head {
                rt = rt.strong();
            }
            let resp = ui.selectable_label(rv.focus_branch.as_deref() == Some(r.name.as_str()), rt).on_hover_text(&r.name);
            if !track.is_empty() {
                ui.label(RichText::new(track).small().color(theme::TEXT_DIM));
            }
            if let Some(t) = &task {
                ui.label(RichText::new(t).small().color(theme::ACCENT));
            }
            if note {
                ui.add(icons::image(&ctx, Icon::Note, isz, theme::ACCENT)).on_hover_text("has a branch note");
            }
            if let Some(pr) = rv.prs.get(&r.name) {
                let tone = crate::pr_view::tone_color(pr.readiness().1);
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add(icons::image(&ctx, Icon::Github, isz, tone)).on_hover_text(crate::pr_view::hover_summary(pr));
                ui.label(RichText::new(format!("#{}", pr.number)).small().color(tone)).on_hover_text(crate::pr_view::hover_summary(pr));
            }
            resp
        })
        .inner;
    if resp.clicked() {
        // Select the tip first (selection resets focus), then focus the branch.
        actions.push(Action::JumpTo(r.target.clone()));
        actions.push(Action::FocusBranch(r.name.clone()));
    }
    if resp.double_clicked() && !r.is_head {
        actions.push(Action::Checkout(r.name.clone()));
    }
    let name = r.name.clone();
    let is_head = r.is_head;
    resp.context_menu(|ui| {
        let mut item = |ui: &mut Ui, label: &str, a: Action| {
            if ui.button(label).clicked() {
                actions.push(a);
                ui.close();
            }
        };
        if !is_head {
            item(ui, "Checkout", Action::Checkout(name.clone()));
            item(ui, "Merge into current branch", Action::Confirm { msg: format!("Merge {name} into the current branch?"), action: Box::new(Action::Merge(name.clone())) });
        }
        item(ui, "New branch from here…", Action::NewBranchDialog { start: Some(name.clone()), name: String::new() });
        item(ui, "Branch notes", Action::FocusBranch(name.clone()));
        item(ui, "Link task…", Action::LinkTaskDialog { branch: name.clone() });
        if let Some(t) = &task {
            item(ui, &format!("Open {t}"), Action::FocusTask(t.clone()));
        }
        item(ui, "Rename…", Action::RenameBranchDialog(name.clone()));
        item(ui, "Copy name", Action::Copy(name.clone()));
        if !is_head {
            ui.separator();
            item(ui, "Delete…", Action::Confirm { msg: format!("Delete branch {name}? Its branch note is kept under Detached notes."), action: Box::new(Action::DeleteBranch { name: name.clone(), force: false }) });
            item(ui, "Force delete…", Action::Confirm { msg: format!("Force-delete {name}? Unmerged commits will be lost."), action: Box::new(Action::DeleteBranch { name: name.clone(), force: true }) });
        }
    });
}
