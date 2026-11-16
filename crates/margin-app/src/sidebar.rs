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
        let n_local = snap.refs.iter().filter(|r| r.kind == RefKind::Local).count();
        section(ui, &format!("BRANCHES ({n_local})"), "branches", true, |ui| {
            // refs arrive newest-first (for-each-ref --sort=-committerdate)
            let tree = branch_tree::build(
                snap.refs.iter().filter(|r| r.kind == RefKind::Local).map(|r| (r.name.clone(), r)).collect(),
            );
            let focus = rv.focus_branch.clone();
            let open_if = |r: &&RefInfo| r.is_head || focus.as_deref() == Some(r.name.as_str());
            render_tree(ui, &tree, "local", 0, &open_if, &mut |ui, label, depth, r| local_row(ui, rv, r, label, depth, isz, actions));
        });

        // ------------------------------------------------------------ worktrees
        // Listed like branches (by the branch they have checked out); only
        // when the repository has more than one.
        if snap.worktrees.len() > 1 {
            section(ui, &format!("WORKTREES ({})", snap.worktrees.len()), "worktrees", true, |ui| {
                for w in &snap.worktrees {
                    worktree_row(ui, rv, w, isz, actions);
                }
            });
        }

        // ------------------------------------------------- recently re-attached
        if !rv.reattached.is_empty() {
            list_section(ui, &format!("RE-ATTACHED ({})", rv.reattached.len()), "reattached", true, |ui| {
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
            list_section(ui, &format!("DETACHED NOTES ({})", rv.detached.len()), "detached", true, |ui| {
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
        let n_remotes = remotes.len();
        section(ui, &format!("REMOTES ({n_remotes})"), "remotes", false, |ui| {
            for (remote, refs) in remotes {
                // Each remote is a group row like "feature/", one level deep.
                if !group_row(ui, ("remote", remote), remote, refs.len(), 0, false) {
                    continue;
                }
                {
                    // Same prefix grouping as local branches, on the part after "origin/".
                    let tree = branch_tree::build(
                        refs.into_iter().map(|r| (r.name.split_once('/').map_or(r.name.clone(), |(_, b)| b.to_string()), r)).collect(),
                    );
                    render_tree(ui, &tree, remote, 1, &|_| false, &mut |ui, label, depth, r| {
                        let hidden = rv.graph_filter.hides(&r.name, true);
                        let resp = ui.horizontal(|ui| {
                            gutter(ui, depth, None);
                            let t = if hidden { RichText::new(label).color(theme::TEXT_DIM) } else { RichText::new(label) };
                            let resp = ui.selectable_label(false, t);
                            if hidden {
                                ui.label(RichText::new("hidden").small().color(theme::TEXT_DIM));
                            }
                            resp
                        }).inner;
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
                            ui.separator();
                            let mut f = rv.graph_filter.clone();
                            f.toggle(&name, true);
                            if ui.button(if hidden { "Show in history" } else { "Hide in history" }).clicked() {
                                actions.push(Action::SetGraphFilter(f));
                                ui.close();
                            }
                        });
                    });
                }
            }
        });

        // ----------------------------------------------------------------- tags
        let n_tags = snap.refs.iter().filter(|r| r.kind == RefKind::Tag).count();
        list_section(ui, &format!("TAGS ({n_tags})"), "tags", false, |ui| {
            for r in snap.refs.iter().filter(|r| r.kind == RefKind::Tag) {
                if ui.selectable_label(false, &r.name).clicked() {
                    actions.push(Action::JumpTo(r.target.clone()));
                }
            }
        });

        // -------------------------------------------------------------- stashes
        list_section(ui, &format!("STASHES ({})", snap.stashes.len()), "stashes", false, |ui| {
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
            let open = list_section(ui, "MY TASKS", "mytasks", false, |ui| {
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
            if open && my_tasks.is_none() && !rv.loading {
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

/// Indent step per tree level (points) — shallow, like Sublime Merge.
const STEP: f32 = 12.0;
/// Width of the triangle / current-branch-dot column.
const SLOT: f32 = 12.0;

/// Leading space for a row at `depth` plus the triangle column. Leaves use the
/// column for the ● current-branch marker, so names align with group names.
fn gutter(ui: &mut Ui, depth: usize, dot: Option<egui::Color32>) {
    ui.spacing_mut().item_spacing.x = 4.0;
    if depth > 0 {
        ui.add_space(depth as f32 * STEP);
    }
    let h = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(SLOT, h), egui::Sense::hover());
    if let Some(c) = dot {
        ui.painter().circle_filled(rect.center(), 3.0, c);
    }
}

/// A collapsible row: triangle in the gutter, then `contents`. Returns whether
/// it is open; the state is remembered for the session.
fn toggle_row(ui: &mut Ui, id: egui::Id, depth: usize, default_open: bool, contents: impl FnOnce(&mut Ui)) -> bool {
    let mut open = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || default_open));
    let resp = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // Labels are text-selectable by default and would swallow the click.
            ui.style_mut().interaction.selectable_labels = false;
            if depth > 0 {
                ui.add_space(depth as f32 * STEP);
            }
            let h = ui.text_style_height(&egui::TextStyle::Body);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(SLOT, h), egui::Sense::hover());
            let c = rect.center();
            let pts = if open {
                vec![egui::pos2(c.x - 4.0, c.y - 2.0), egui::pos2(c.x + 4.0, c.y - 2.0), egui::pos2(c.x, c.y + 3.0)]
            } else {
                vec![egui::pos2(c.x - 2.0, c.y - 4.0), egui::pos2(c.x + 3.0, c.y), egui::pos2(c.x - 2.0, c.y + 4.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(pts, theme::TEXT_DIM, egui::Stroke::NONE));
            // Match the inner padding of the selectable branch labels so group
            // names, headings and branch names share one left edge.
            ui.add_space((ui.spacing().button_padding.x - ui.spacing().item_spacing.x).max(0.0) + 3.0);
            contents(ui);
        })
        .response
        .interact(egui::Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    open
}

/// A prefix group ("feature/  12").
fn group_row(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, label: &str, count: usize, depth: usize, default_open: bool) -> bool {
    toggle_row(ui, egui::Id::unique(("tree-group", id)), depth, default_open, |ui| {
        ui.label(label);
        ui.label(RichText::new(count.to_string()).small().color(theme::TEXT_DIM));
    })
}

/// Section heading ("BRANCHES (34)"). Its rows use depth 0, so top-level
/// branch names line up under the heading text.
fn section(ui: &mut Ui, title: &str, id: &str, default_open: bool, body: impl FnOnce(&mut Ui)) -> bool {
    let open = toggle_row(ui, egui::Id::unique(("section", id)), 0, default_open, |ui| {
        ui.label(RichText::new(title).small().strong());
    });
    if open {
        body(ui);
    }
    open
}

/// A section whose rows are plain widgets: pad them past the triangle column
/// so they line up with the heading text (tree rows draw their own gutter).
fn list_section(ui: &mut Ui, title: &str, id: &str, default_open: bool, body: impl FnOnce(&mut Ui)) -> bool {
    section(ui, title, id, default_open, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(SLOT + 4.0);
            ui.vertical(|ui| body(ui));
        });
    })
}

/// Render a prefix tree; groups are compact collapsible rows.
/// A group starts open if it contains an item matching `open_if`.
pub fn render_tree<'a>(
    ui: &mut Ui,
    nodes: &[Node<&'a RefInfo>],
    ns: &str,
    depth: usize,
    open_if: &dyn Fn(&&'a RefInfo) -> bool,
    leaf: &mut dyn FnMut(&mut Ui, &str, usize, &'a RefInfo),
) {
    for n in nodes {
        match n {
            Node::Leaf { label, item } => leaf(ui, label, depth, item),
            Node::Group { label, path, children, count } => {
                if group_row(ui, (ns, path.as_str()), label, *count, depth, branch_tree::any(children, open_if)) {
                    render_tree(ui, children, ns, depth + 1, open_if, leaf);
                }
            }
        }
    }
}

/// Task key, note icon and PR badge shown after a branch name.
fn branch_extras(ui: &mut Ui, rv: &RepoView, r: &RefInfo, task: &Option<String>, note: bool, isz: f32) {
    let ctx = ui.ctx().clone();
    if let Some(t) = task {
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
}

/// Width `branch_extras` will take (so the name can be truncated first).
fn extras_width(ui: &Ui, rv: &RepoView, r: &RefInfo, task: &Option<String>, note: bool, isz: f32) -> f32 {
    let gap = ui.spacing().item_spacing.x;
    let small = egui::TextStyle::Small.resolve(ui.style());
    let text_w = |t: String| ui.painter().layout_no_wrap(t, small.clone(), theme::TEXT).size().x;
    let mut w = 0.0;
    if let Some(t) = task {
        w += text_w(t.clone()) + gap;
    }
    if note {
        w += isz + gap;
    }
    if let Some(pr) = rv.prs.get(&r.name) {
        w += isz + 2.0 + text_w(format!("#{}", pr.number)) + gap;
    }
    w
}

/// Sublime Merge-style ahead/behind pill: "1849↓ │ 1↑" (behind first), or
/// "gone" when the upstream branch was deleted. Width 0 when nothing to show.
struct Badge {
    galleys: Vec<std::sync::Arc<egui::Galley>>,
    gone: bool,
    tip: String,
}

impl Badge {
    const PAD: f32 = 5.0;

    fn new(ui: &Ui, r: &RefInfo) -> Badge {
        let (ahead, behind) = r.ahead_behind();
        let gone = r.upstream_gone();
        let segs: Vec<String> = if gone {
            vec!["gone".into()]
        } else {
            [(behind > 0).then(|| format!("{behind}↓")), (ahead > 0).then(|| format!("{ahead}↑"))].into_iter().flatten().collect()
        };
        let small = egui::TextStyle::Small.resolve(ui.style());
        let color = if gone { theme::TEXT_DIM } else { theme::TEXT };
        let galleys = segs.into_iter().map(|t| ui.painter().layout_no_wrap(t, small.clone(), color)).collect();
        let up = r.upstream.clone().unwrap_or_default();
        let tip = if gone {
            format!("Upstream {up} no longer exists on the remote")
        } else {
            format!("{behind} behind, {ahead} ahead of {up}\n(as of the last fetch)")
        };
        Badge { galleys, gone, tip }
    }

    fn width(&self) -> f32 {
        self.galleys.iter().map(|g| g.size().x + 2.0 * Self::PAD).sum()
    }

    fn show(self, ui: &mut Ui) {
        if self.galleys.is_empty() {
            return;
        }
        let h = self.galleys[0].size().y + 4.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(self.width(), h), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 3.0, if self.gone { theme::BADGE.gamma_multiply(0.6) } else { theme::BADGE });
        let mut x = rect.left();
        for (i, g) in self.galleys.into_iter().enumerate() {
            if i > 0 {
                p.line_segment([egui::pos2(x, rect.top() + 3.0), egui::pos2(x, rect.bottom() - 3.0)], egui::Stroke::new(1.0, theme::BG));
            }
            let w = g.size().x + 2.0 * Self::PAD;
            p.galley(egui::pos2(x + Self::PAD, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
            x += w;
        }
        resp.on_hover_text(self.tip);
    }
}

/// One local branch row: [● gutter] name  task note PR …… [behind↓│ahead↑]
fn local_row(ui: &mut Ui, rv: &RepoView, r: &RefInfo, label: &str, depth: usize, isz: f32, actions: &mut Vec<Action>) {
    let task = rv.notes.task_for_branch(&r.name);
    let note = rv.has_note(&NoteTarget::Branch(r.name.clone()));
    let hidden = rv.graph_filter.hides(&r.name, false);
    // Checked out in another worktree: git refuses to check it out here.
    let elsewhere = rv.snap.as_ref().and_then(|s| s.worktree_for(&r.name)).map(|w| w.wt.path.clone());
    let resp = ui
        .horizontal(|ui| {
            // ● for the current branch sits in the triangle column
            gutter(ui, depth, r.is_head.then_some(theme::ACCENT));
            let badge = Badge::new(ui, r);
            let bw = badge.width();
            // Left part gets what the badge leaves, so long names truncate before it.
            let left_w = (ui.available_width() - if bw > 0.0 { bw + 8.0 } else { 0.0 }).max(40.0);
            let h = ui.spacing().interact_size.y;
            // Reserve room for task / note / PR so only the *name* truncates.
            let small = egui::TextStyle::Small.resolve(ui.style());
            let hidden_w = if hidden { ui.painter().layout_no_wrap("hidden".into(), small, theme::TEXT_DIM).size().x + ui.spacing().item_spacing.x } else { 0.0 };
            let extras_w = extras_width(ui, rv, r, &task, note, isz) + hidden_w + if elsewhere.is_some() { isz + ui.spacing().item_spacing.x } else { 0.0 };
            let name_w = (left_w - extras_w).max(40.0);
            let resp = ui
                .allocate_ui_with_layout(egui::vec2(left_w, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let mut rt = RichText::new(label);
                    if r.is_head {
                        rt = rt.strong();
                    }
                    // Upstream deleted (e.g. merged and removed on the server): dimmed.
                    if r.upstream_gone() || hidden {
                        rt = rt.color(theme::TEXT_DIM);
                    }
                    let resp = ui
                        .allocate_ui_with_layout(egui::vec2(name_w, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.selectable_label(rv.focus_branch.as_deref() == Some(r.name.as_str()), rt).on_hover_text(&r.name)
                        })
                        .inner;
                    if hidden {
                        ui.label(RichText::new("hidden").small().color(theme::TEXT_DIM)).on_hover_text("Hidden in the history graph (right-click → Show in history)");
                    }
                    if let Some(p) = &elsewhere {
                        ui.add(icons::image(ui.ctx(), Icon::FileDirectory, isz, theme::TEXT_DIM))
                            .on_hover_text(format!("Checked out in worktree {}\nDouble-click to open it", p.display()));
                    }
                    branch_extras(ui, rv, r, &task, note, isz);
                    resp
                })
                .inner;
            if bw > 0.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    badge.show(ui);
                });
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
        match &elsewhere {
            Some(p) => actions.push(Action::OpenRepo(p.clone())),
            None => actions.push(Action::Checkout(r.name.clone())),
        }
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
        if let Some(p) = &elsewhere {
            item(ui, "Open worktree", Action::OpenRepo(p.clone()));
        } else if !is_head {
            item(ui, "Checkout", Action::Checkout(name.clone()));
        }
        if !is_head {
            item(ui, "Merge into current branch", Action::Confirm { msg: format!("Merge {name} into the current branch?"), action: Box::new(Action::Merge(name.clone())) });
        }
        item(ui, "New branch from here…", Action::NewBranchDialog { start: Some(name.clone()), name: String::new() });
        item(ui, "Branch notes", Action::FocusBranch(name.clone()));
        item(ui, "Link task…", Action::LinkTaskDialog { branch: name.clone() });
        if let Some(t) = &task {
            item(ui, &format!("Open {t}"), Action::FocusTask(t.clone()));
        }
        item(ui, "Rename…", Action::RenameBranchDialog(name.clone()));
        ui.separator();
        let mut f = rv.graph_filter.clone();
        if f.only.is_some() || !hidden {
            let mut solo = f.clone();
            solo.only = if f.only.as_deref() == Some(name.as_str()) { None } else { Some(name.clone()) };
            let label = if solo.only.is_some() { "Show only this branch in history" } else { "Show all branches in history" };
            item(ui, label, Action::SetGraphFilter(solo));
        }
        f.toggle(&name, false);
        item(ui, if hidden { "Show in history" } else { "Hide in history" }, Action::SetGraphFilter(f));
        item(ui, "Copy name", Action::Copy(name.clone()));
        if !is_head {
            ui.separator();
            item(ui, "Delete…", Action::Confirm { msg: format!("Delete branch {name}? Its branch note is kept under Detached notes."), action: Box::new(Action::DeleteBranch { name: name.clone(), force: false }) });
            item(ui, "Force delete…", Action::Confirm { msg: format!("Force-delete {name}? Unmerged commits will be lost."), action: Box::new(Action::DeleteBranch { name: name.clone(), force: true }) });
        }
    });
}

/// A worktree row, shaped like a branch row: ● when it's this tab, the
/// branch it has checked out, and its folder name dimmed on the right.
fn worktree_row(ui: &mut Ui, rv: &RepoView, w: &crate::jobs::WorktreeInfo, isz: f32, actions: &mut Vec<Action>) {
    let wt = &w.wt;
    let folder = wt.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| wt.path.display().to_string());
    let label = match (&wt.branch, &wt.head) {
        (Some(b), _) => b.clone(),
        (None, Some(h)) => format!("detached at {}", &h[..7.min(h.len())]),
        (None, None) => "detached".into(),
    };
    let resp = ui
        .horizontal(|ui| {
            gutter(ui, 0, w.current.then_some(theme::ACCENT));
            ui.add(icons::image(ui.ctx(), Icon::FileDirectory, isz, if w.current { theme::ACCENT } else { theme::TEXT_DIM }));
            let mut detail = folder.clone();
            if wt.prunable {
                detail = format!("{detail} · missing");
            } else if wt.locked {
                detail = format!("{detail} · locked");
            }
            let small = egui::TextStyle::Small.resolve(ui.style());
            let dw = ui.painter().layout_no_wrap(detail.clone(), small, theme::TEXT_DIM).size().x;
            let name_w = (ui.available_width() - dw - 12.0).max(40.0);
            let mut rt = RichText::new(&label);
            if w.current {
                rt = rt.strong();
            }
            let focused = wt.branch.is_some() && rv.focus_branch.as_deref() == wt.branch.as_deref();
            let resp = ui
                .allocate_ui_with_layout(egui::vec2(name_w, ui.spacing().interact_size.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.selectable_label(focused, rt)
                })
                .inner;
            ui.label(RichText::new(detail).small().color(theme::TEXT_DIM));
            resp
        })
        .inner;
    let path = wt.path.clone();
    let hover = format!(
        "{}{}",
        path.display(),
        if w.current { "\n(this tab)" } else { "\nDouble-click to open in a tab" }
    );
    let resp = resp.on_hover_text(hover);
    if resp.clicked() {
        // Like a branch row: show its tip in the history.
        if let Some(b) = &wt.branch {
            if let Some(r) = rv.snap.as_ref().and_then(|s| s.refs.iter().find(|r| r.kind == RefKind::Local && &r.name == b)) {
                actions.push(Action::JumpTo(r.target.clone()));
                actions.push(Action::FocusBranch(b.clone()));
            }
        } else if let Some(h) = &wt.head {
            actions.push(Action::JumpTo(h.clone()));
        }
    }
    if resp.double_clicked() && !w.current {
        actions.push(Action::OpenRepo(path.clone()));
    }
    let (main, current, prunable) = (wt.main, w.current, wt.prunable);
    resp.context_menu(|ui| {
        let mut item = |ui: &mut Ui, label: &str, a: Action| {
            if ui.button(label).clicked() {
                actions.push(a);
                ui.close();
            }
        };
        if !current && !prunable {
            item(ui, "Open in tab", Action::OpenRepo(path.clone()));
        }
        if !prunable {
            item(ui, "Reveal in Finder", Action::OpenPath { path: path.clone(), terminal: false });
            item(ui, "Open in Terminal", Action::OpenPath { path: path.clone(), terminal: true });
        }
        item(ui, "Copy path", Action::Copy(path.display().to_string()));
        if prunable {
            ui.separator();
            item(ui, "Prune missing worktrees", Action::RunGit { label: "Prune worktrees".into(), f: std::sync::Arc::new(|r| r.prune_worktrees()) });
        } else if !main && !current {
            ui.separator();
            let p = path.display().to_string();
            let remove = |force: bool| {
                let p = p.clone();
                Action::RunGit { label: "Remove worktree".into(), f: std::sync::Arc::new(move |r| r.remove_worktree(&p, force).map(|_| String::new())) }
            };
            item(ui, "Remove…", Action::Confirm { msg: format!("Remove the worktree folder {p}?\n(Refused if it has uncommitted changes.)"), action: Box::new(remove(false)) });
            item(ui, "Force remove…", Action::Confirm { msg: format!("Force-remove {p}? Uncommitted changes in it are lost."), action: Box::new(remove(true)) });
        }
    });
}
