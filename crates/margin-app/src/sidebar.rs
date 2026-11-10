use crate::app::{Action, RepoView, Sel};
use crate::notes_panel::NoteTab;
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::git::RefKind;
use margin_core::notes::NoteTarget;
use margin_core::tasks::Task;
use std::collections::BTreeMap;

pub fn show(ui: &mut Ui, rv: &mut RepoView, tasks_enabled: bool, my_tasks: Option<&Result<Vec<Task>, String>>, actions: &mut Vec<Action>) {
    let Some(snap) = rv.snap.as_ref() else { return };
    egui::ScrollArea::vertical().id_salt("sidebar").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        let n = snap.status.files.len();
        if ui.selectable_label(rv.sel == Sel::Working, format!("Working directory ({n})")).clicked() {
            actions.push(Action::Select(Sel::Working));
        }
        let repo_note = rv.has_note(&NoteTarget::Repo);
        if ui.selectable_label(false, format!("Repository notes{}", if repo_note { "  ●" } else { "" })).clicked() {
            actions.push(Action::ShowNote(NoteTab::Repo));
        }
        ui.add_space(6.0);

        // ---------------------------------------------------------------- local
        egui::CollapsingHeader::new(RichText::new("BRANCHES").small().strong()).default_open(true).show(ui, |ui| {
            for r in snap.refs.iter().filter(|r| r.kind == RefKind::Local) {
                let task = rv.notes.task_for_branch(&r.name);
                let note = rv.has_note(&NoteTarget::Branch(r.name.clone()));
                let mut text = String::new();
                text += if r.is_head { "● " } else { "   " };
                text += &r.name;
                let track = r.track.replace("ahead ", "↑").replace("behind ", "↓").replace(", ", " ");
                let resp = ui
                    .horizontal(|ui| {
                        let mut rt = RichText::new(text);
                        if r.is_head {
                            rt = rt.strong();
                        }
                        let resp = ui.selectable_label(rv.focus_branch.as_deref() == Some(r.name.as_str()), rt);
                        if !track.is_empty() {
                            ui.label(RichText::new(track).small().color(theme::TEXT_DIM));
                        }
                        if let Some(t) = &task {
                            ui.label(RichText::new(t).small().color(theme::ACCENT));
                        }
                        if let Some(pr) = rv.prs.get(&r.name) {
                            let tone = crate::pr_view::tone_color(pr.readiness().1);
                            ui.label(RichText::new(format!("#{}", pr.number)).small().color(tone))
                                .on_hover_text(crate::pr_view::hover_summary(pr));
                        }
                        if note {
                            ui.label(RichText::new("●").small().color(theme::ACCENT)).on_hover_text("has notes");
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
                let task = task.clone();
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
                    item(ui, "Copy name", Action::Copy(name.clone()));
                    if !is_head {
                        ui.separator();
                        item(ui, "Delete…", Action::Confirm { msg: format!("Delete branch {name}?"), action: Box::new(Action::DeleteBranch { name: name.clone(), force: false }) });
                        item(ui, "Force delete…", Action::Confirm { msg: format!("Force-delete {name}? Unmerged commits will be lost."), action: Box::new(Action::DeleteBranch { name: name.clone(), force: true }) });
                    }
                });
            }
        });

        // -------------------------------------------------------------- remotes
        let mut remotes: BTreeMap<&str, Vec<&margin_core::git::RefInfo>> = BTreeMap::new();
        for r in snap.refs.iter().filter(|r| r.kind == RefKind::Remote) {
            let remote = r.name.split('/').next().unwrap_or("");
            remotes.entry(remote).or_default().push(r);
        }
        egui::CollapsingHeader::new(RichText::new("REMOTES").small().strong()).default_open(false).show(ui, |ui| {
            for (remote, refs) in remotes {
                egui::CollapsingHeader::new(remote).id_salt(("remote", remote)).show(ui, |ui| {
                    for r in refs {
                        let short = r.name.split_once('/').map_or(r.name.as_str(), |(_, b)| b);
                        let resp = ui.selectable_label(false, short);
                        if resp.clicked() {
                            actions.push(Action::JumpTo(r.target.clone()));
                        }
                        let name = r.name.clone();
                        resp.context_menu(|ui| {
                            if ui.button("Checkout as tracking branch").clicked() {
                                actions.push(Action::CheckoutRemote(name.clone()));
                                ui.close();
                            }
                            if ui.button("Merge into current branch").clicked() {
                                actions.push(Action::Confirm { msg: format!("Merge {name}?"), action: Box::new(Action::Merge(name.clone())) });
                                ui.close();
                            }
                        });
                    }
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
