//! Interactive rebase: plan what happens to each commit after a base
//! (pick, reword, edit, squash, fixup, drop), drag them into a new order,
//! then start the rebase. Shown in place of the history, like review mode.
//! The list reads like the history: newest at the top; a squash or fixup
//! folds a commit into the one below it.

use crate::app::{Action as AppAction, RepoView};
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::rebase::{self, Action, Item, Step};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

const KINDS: [(Kind, &str, char, &str); 6] = [
    (Kind::Pick, "Pick", 'p', "Keep the commit as it is"),
    (Kind::Reword, "Reword", 'r', "Keep the changes, change the message"),
    (Kind::Edit, "Edit", 'e', "Stop after it, to amend it or add commits; then Continue"),
    (Kind::Squash, "Squash", 's', "Fold into the commit below, and write the combined message"),
    (Kind::Fixup, "Fixup", 'f', "Fold into the commit below, keeping that commit's message"),
    (Kind::Drop, "Drop", 'd', "Leave the commit out"),
];

fn kind_name(k: Kind) -> &'static str {
    KINDS.iter().find(|x| x.0 == k).map_or("", |x| x.1)
}

fn kind_color(k: Kind) -> egui::Color32 {
    match k {
        Kind::Pick => theme::TEXT_DIM(),
        Kind::Reword => theme::lane(0),
        Kind::Edit => theme::lane(2),
        Kind::Squash | Kind::Fixup => theme::lane(3),
        Kind::Drop => theme::ERROR(),
    }
}

struct Row {
    item: Item,
    kind: Kind,
    /// The message for reword / squash.
    message: String,
}

pub struct Plan {
    /// The commit the rebase starts after.
    base: String,
    base_label: String,
    branch: String,
    /// Newest first, like the history.
    rows: Vec<Row>,
    sel: usize,
    /// The row being dragged.
    dragging: Option<usize>,
    /// The order as loaded (newest first).
    orig: Vec<String>,
    /// Focus this commit's message box (just switched to reword / squash).
    focus: Option<String>,
}

impl Plan {
    /// Plan the commits after `base` (or the error why it can't be done).
    pub fn new(repo: &margin_core::git::Repo, base: &str) -> Result<Plan, String> {
        let items = rebase::commits(repo, base).map_err(|e| e.0)?;
        if items.is_empty() {
            return Err("There are no commits after this one on the current branch.".into());
        }
        let base_label = repo.run(&["log", "-1", "--format=%h %s", base]).map(|s| s.trim().to_string()).unwrap_or_else(|_| base.to_string());
        let branch = repo.run(&["rev-parse", "--abbrev-ref", "HEAD"]).map(|s| s.trim().to_string()).unwrap_or_default();
        let rows: Vec<Row> = items.into_iter().rev().map(|item| Row { message: item.message.clone(), item, kind: Kind::Pick }).collect();
        let orig = rows.iter().map(|r| r.item.id.clone()).collect();
        Ok(Plan { base: base.to_string(), base_label, branch, rows, sel: 0, dragging: None, orig, focus: None })
    }

    /// The plan in rebase order (oldest first).
    fn steps(&self) -> Vec<Step> {
        self.rows
            .iter()
            .rev()
            .map(|r| Step {
                id: r.item.id.clone(),
                action: match r.kind {
                    Kind::Pick => Action::Pick,
                    Kind::Reword => Action::Reword(r.message.clone()),
                    Kind::Edit => Action::Edit,
                    Kind::Squash => Action::Squash(r.message.clone()),
                    Kind::Fixup => Action::Fixup,
                    Kind::Drop => Action::Drop,
                },
            })
            .collect()
    }

    /// The commit a squash / fixup at `i` folds into: the nearest kept one below.
    fn target(&self, i: usize) -> Option<usize> {
        (i + 1..self.rows.len()).find(|&j| self.rows[j].kind != Kind::Drop && !matches!(self.rows[j].kind, Kind::Squash | Kind::Fixup)).or_else(|| (i + 1..self.rows.len()).find(|&j| self.rows[j].kind != Kind::Drop))
    }

    fn set_kind(&mut self, i: usize, k: Kind) {
        let old = self.rows[i].kind;
        if old == k {
            return;
        }
        self.rows[i].kind = k;
        if matches!(k, Kind::Reword | Kind::Squash) {
            self.focus = Some(self.rows[i].item.id.clone());
        }
        match k {
            Kind::Reword if old != Kind::Squash => self.rows[i].message = self.rows[i].item.message.clone(),
            // the combined message: what it folds into, then this one
            Kind::Squash => {
                let into = self.target(i).map(|j| if matches!(self.rows[j].kind, Kind::Reword | Kind::Squash) { self.rows[j].message.clone() } else { self.rows[j].item.message.clone() });
                self.rows[i].message = match into {
                    Some(m) => format!("{m}\n\n{}", self.rows[i].item.message),
                    None => self.rows[i].item.message.clone(),
                };
            }
            _ => {}
        }
    }

    fn move_row(&mut self, from: usize, to: usize) {
        if from == to || to >= self.rows.len() {
            return;
        }
        let r = self.rows.remove(from);
        self.rows.insert(to, r);
        self.sel = to;
    }

    /// Anything to do: an action other than pick, or a new order.
    fn changed(&self) -> bool {
        self.rows.iter().any(|r| r.kind != Kind::Pick) || !self.rows.iter().map(|r| &r.item.id).eq(self.orig.iter())
    }
}

pub fn show(ui: &mut Ui, rv: &mut RepoView, actions: &mut Vec<AppAction>) {
    let Some(p) = rv.rebase.as_mut() else { return };
    let steps = p.steps();
    let problem = rebase::check(&steps).err().map(|e| e.0);
    let empty_msg = p.rows.iter().any(|r| matches!(r.kind, Kind::Reword | Kind::Squash) && r.message.trim().is_empty());

    egui::Panel::top("rebase_head").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            // Esc in a message box only leaves the box
            let typing = ui.memory(|m| m.focused().is_some()) || ui.data(|d| d.get_temp::<bool>(egui::Id::unique("rebase-typing"))).unwrap_or(false);
            if ui.button("‹ History").on_hover_text("Back to the history without rebasing (Esc)").clicked() || (!typing && ui.input(|i| i.key_pressed(egui::Key::Escape))) {
                actions.push(AppAction::CloseRebase);
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Interactive rebase").strong());
            ui.label(RichText::new(format!("{} · {} commit{} after", p.branch, p.rows.len(), if p.rows.len() == 1 { "" } else { "s" })).color(theme::TEXT_DIM()));
            ui.add(egui::Label::new(RichText::new(&p.base_label).monospace().color(theme::TEXT_DIM())).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = problem.is_none() && !empty_msg && p.changed();
                let b = ui.add_enabled(ok, egui::Button::new(RichText::new("Start rebase").strong()).fill(if ok { theme::ACCENT() } else { theme::BADGE() }));
                let b = if ok {
                    b.on_hover_text("Rewrite the commits by this plan. Uncommitted changes are put aside and back. If it stops (Edit, conflicts), continue or abort from the red badge.")
                } else if !p.changed() {
                    b.on_disabled_hover_text("Nothing to do yet: change an action or the order")
                } else {
                    b.on_disabled_hover_text(problem.clone().unwrap_or_else(|| "A message is empty".into()))
                };
                if b.clicked() {
                    let base = p.base.clone();
                    let steps = steps.clone();
                    actions.push(AppAction::RunGit { label: "Rebase".into(), f: Arc::new(move |r| r.rebase_interactive(&base, &steps)) });
                    actions.push(AppAction::CloseRebase);
                }
                if ui.button("Reset").on_hover_text("Back to the original order, every commit picked").clicked() {
                    let orig = p.orig.clone();
                    p.rows.sort_by_key(|r| orig.iter().position(|id| *id == r.item.id));
                    for r in &mut p.rows {
                        r.kind = Kind::Pick;
                        r.message = r.item.message.clone();
                    }
                }
            });
        });
        ui.add_space(4.0);
    });

    egui::CentralPanel::default().show(ui, |ui| {
        // summary, warnings
        let kept = p.rows.iter().filter(|r| !matches!(r.kind, Kind::Drop | Kind::Squash | Kind::Fixup)).count();
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{} commits → {kept}", p.rows.len())).small().color(theme::TEXT_DIM()));
            for (k, word) in [(Kind::Reword, "reworded"), (Kind::Edit, "to edit"), (Kind::Squash, "squashed"), (Kind::Fixup, "fixed up"), (Kind::Drop, "dropped")] {
                let n = p.rows.iter().filter(|r| r.kind == k).count();
                if n > 0 {
                    ui.label(RichText::new(format!("· {n} {word}")).small().color(kind_color(k)));
                }
            }
            ui.label(RichText::new("· drag to reorder, or Alt+↑/↓; keys P R E S F D set the action").small().color(theme::TEXT_DIM()));
        });
        let pushed = p.rows.iter().filter(|r| r.item.pushed).count();
        if pushed > 0 {
            ui.label(RichText::new(format!("⚠ {pushed} of these commits are already pushed: after the rebase the branch needs a force push, and others who have it must reset to it.")).small().color(theme::lane(2)));
        }
        if let Some(e) = &problem {
            ui.label(RichText::new(format!("⚠ {e}")).small().color(theme::ERROR()));
        }
        ui.add_space(6.0);

        // keys for the selected row (when not typing)
        let typing = ui.memory(|m| m.focused().is_some());
        if !typing && !p.rows.is_empty() {
            let (up, down, alt) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.modifiers.alt));
            if alt && up && p.sel > 0 {
                p.move_row(p.sel, p.sel - 1);
            } else if alt && down && p.sel + 1 < p.rows.len() {
                p.move_row(p.sel, p.sel + 1);
            } else if up {
                p.sel = p.sel.saturating_sub(1);
            } else if down {
                p.sel = (p.sel + 1).min(p.rows.len() - 1);
            }
            for (k, _, key, _) in KINDS {
                let key = egui::Key::from_name(&key.to_ascii_uppercase().to_string());
                if key.is_some_and(|key| ui.input(|i| i.key_pressed(key) && i.modifiers.is_none())) {
                    let s = p.sel;
                    p.set_kind(s, k);
                }
            }
        }

        let mut rects: Vec<egui::Rect> = vec![];
        let mut kind_change = None;
        let mut select = None;
        let mut drag_start = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let n = p.rows.len();
            for i in 0..n {
                let target_label = matches!(p.rows[i].kind, Kind::Squash | Kind::Fixup).then(|| p.target(i).map(|j| p.rows[j].item.id[..7].to_string())).flatten();
                let r = &mut p.rows[i];
                let selected = p.sel == i;
                let dragged = p.dragging == Some(i);
                let top = ui.cursor().min;
                let w = ui.available_width();
                let bg = ui.painter().add(egui::Shape::Noop);
                let inner = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(top + egui::vec2(6.0, 4.0), egui::vec2(w - 12.0, f32::INFINITY))), |ui| {
                        ui.horizontal(|ui| {
                            // drag handle
                            let (hr, _) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::hover());
                            let h = ui.interact(hr, egui::Id::unique(("rebase-drag", r.item.id.as_str())), egui::Sense::drag());
                            for dy in [-4.0, 0.0, 4.0] {
                                for dx in [-2.5, 2.5] {
                                    ui.painter().circle_filled(hr.center() + egui::vec2(dx, dy), 1.2, if h.hovered() || dragged { theme::TEXT() } else { theme::TEXT_DIM() });
                                }
                            }
                            if h.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            if h.drag_started() {
                                drag_start = Some(i);
                            }
                            // action
                            let col = kind_color(r.kind);
                            ui.menu_button(RichText::new(format!("{:<6} ▾", kind_name(r.kind))).monospace().color(col), |ui| {
                                for (k, name, key, help) in KINDS {
                                    if ui.add(egui::Button::new(name).shortcut_text(key.to_ascii_uppercase().to_string())).on_hover_text(help).clicked() {
                                        kind_change = Some((i, k));
                                        ui.close();
                                    }
                                }
                            });
                            ui.label(RichText::new(&r.item.id[..7]).monospace().color(theme::TEXT_DIM()));
                            let subject = RichText::new(&r.item.subject);
                            let subject = match r.kind {
                                Kind::Drop => subject.strikethrough().color(theme::TEXT_DIM()),
                                Kind::Squash | Kind::Fixup => subject.color(theme::TEXT_DIM()),
                                _ => subject,
                            };
                            ui.add(egui::Label::new(subject).truncate().selectable(false));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(crate::commit_list::rel_time(margin_core::time_now(), r.item.time)).small().color(theme::TEXT_DIM()));
                                if r.item.pushed {
                                    ui.label(RichText::new("pushed").small().color(theme::lane(2))).on_hover_text("On a remote branch already");
                                }
                                if let Some(t) = &target_label {
                                    ui.label(RichText::new(format!("↓ into {t}")).small().color(theme::lane(3)));
                                }
                            });
                        });
                        // the message, for reword and squash
                        if matches!(r.kind, Kind::Reword | Kind::Squash) {
                            ui.horizontal(|ui| {
                                ui.add_space(22.0);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(if r.kind == Kind::Squash { "Message of the combined commit" } else { "New message" }).small().color(theme::TEXT_DIM()));
                                    let rows = r.message.lines().count().clamp(2, 12);
                                    let te = ui.add(egui::TextEdit::multiline(&mut r.message).desired_rows(rows).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                                    if p.focus.as_deref() == Some(r.item.id.as_str()) {
                                        te.request_focus();
                                        p.focus = None;
                                    }
                                });
                            });
                        }
                    })
                    .response;
                let rect = egui::Rect::from_min_size(top, egui::vec2(w, inner.rect.height() + 8.0));
                ui.advance_cursor_after_rect(rect);
                let resp = ui.interact(rect, egui::Id::unique(("rebase-row", r.item.id.as_str())), egui::Sense::click());
                if resp.clicked() {
                    select = Some(i);
                }
                let fill = if dragged {
                    Some(theme::HOVER())
                } else if selected {
                    Some(theme::SELECTED())
                } else if resp.hovered() {
                    Some(theme::HOVER())
                } else {
                    None
                };
                if let Some(c) = fill {
                    ui.painter().set(bg, egui::Shape::rect_filled(rect, 4.0, c));
                }
                rects.push(rect);
                ui.add_space(2.0);
            }
            ui.add_space(8.0);
            ui.label(RichText::new(format!("after {}", p.base_label)).small().color(theme::TEXT_DIM()));
        });

        if let Some((i, k)) = kind_change {
            p.set_kind(i, k);
            p.sel = i;
        }
        if let Some(i) = select {
            p.sel = i;
        }
        if let Some(i) = drag_start {
            p.dragging = Some(i);
            p.sel = i;
        }
        let typing_now = ui.memory(|m| m.focused().is_some());
        ui.data_mut(|d| d.insert_temp(egui::Id::unique("rebase-typing"), typing_now));
        // dragging: a line where it'll go; drop on release
        if let Some(from) = p.dragging {
            let y = ui.input(|i| i.pointer.hover_pos()).map(|p| p.y);
            let to = y.map(|y| rects.iter().position(|r| y < r.center().y).unwrap_or(rects.len()));
            if let (Some(to), Some(first)) = (to, rects.first()) {
                let ly = if to < rects.len() { rects[to].top() - 1.0 } else { rects.last().unwrap().bottom() + 1.0 };
                ui.painter().hline(first.x_range(), ly, egui::Stroke::new(2.0, theme::ACCENT()));
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if ui.input(|i| !i.pointer.any_down()) {
                if let Some(to) = to {
                    let to = if to > from { to - 1 } else { to };
                    p.move_row(from, to);
                }
                p.dragging = None;
            }
        }
    });
}
