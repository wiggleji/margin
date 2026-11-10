//! Commit list with graph lanes (virtualised; painter-drawn for speed).

use crate::app::{Action, RepoView, Sel};
use crate::theme;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use margin_core::git::{Commit, ResetMode};
use margin_core::notes::NoteTarget;

const ROW_H: f32 = 22.0;
const LANE_W: f32 = 12.0;
const MAX_LANES: usize = 14;

pub fn show(ui: &mut Ui, rv: &mut RepoView, actions: &mut Vec<Action>) {
    let Some(snap) = rv.snap.as_ref() else {
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };

    // Filter cache
    let filter = rv.filter.trim().to_lowercase();
    if !filter.is_empty() && rv.filtered.as_ref().is_none_or(|(f, _)| *f != filter) {
        let idx = snap
            .commits
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.subject.to_lowercase().contains(&filter)
                    || c.author.to_lowercase().contains(&filter)
                    || c.id.starts_with(&filter)
                    || c.refs.iter().any(|r| r.to_lowercase().contains(&filter))
            })
            .map(|(i, _)| i)
            .collect();
        rv.filtered = Some((filter.clone(), idx));
    }
    let visible: Option<&Vec<usize>> = if filter.is_empty() { None } else { rv.filtered.as_ref().map(|(_, v)| v) };
    let n_commits = visible.map_or(snap.commits.len(), Vec::len);
    let has_more = snap.commits.len() >= snap.limit && visible.is_none();
    let total = 1 + n_commits + has_more as usize;

    let head = snap.status.head_oid.clone();
    let font = FontId::proportional(ui.style().text_styles[&egui::TextStyle::Body].size);
    let small = FontId::proportional(font.size - 1.5);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Keyboard navigation (↑/↓) when no text field has focus.
    let no_focus = ui.memory(|m| m.focused().is_none());
    if no_focus {
        let (up, down) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
        if up || down {
            let cur: isize = match &rv.sel {
                Sel::Working => 0,
                Sel::Commit(id) => rv
                    .row_of
                    .get(id)
                    .and_then(|&r| match visible {
                        Some(v) => v.iter().position(|&x| x == r),
                        None => Some(r),
                    })
                    .map_or(0, |p| p as isize + 1),
            };
            let next = (cur + if down { 1 } else { -1 }).clamp(0, n_commits as isize);
            if next != cur {
                rv.scroll_to_row = Some(next as usize);
                actions.push(if next == 0 {
                    Action::Select(Sel::Working)
                } else {
                    let ci = visible.map_or(next as usize - 1, |v| v[next as usize - 1]);
                    Action::Select(Sel::Commit(snap.commits[ci].id.clone()))
                });
            }
        }
    }

    let mut scroll = egui::ScrollArea::vertical().id_salt("commits").auto_shrink([false, false]);
    if let Some(r) = rv.scroll_to_row.take() {
        // Keep the target row in view.
        let y = r as f32 * (ROW_H + 0.0);
        scroll = scroll.vertical_scroll_offset((y - 200.0).max(0.0));
    }

    scroll.show_rows(ui, ROW_H, total, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let width = ui.available_width();
        for row in range {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
            let p = ui.painter_at(rect);

            if row == 0 {
                let selected = rv.sel == Sel::Working;
                bg(&p, rect, selected, resp.hovered(), false);
                let st = &snap.status;
                let staged = st.files.iter().filter(|f| f.is_staged()).count();
                let unstaged = st.files.iter().filter(|f| f.is_unstaged()).count();
                let text = if st.files.is_empty() {
                    "Working directory — clean".to_string()
                } else {
                    format!("Working directory — {unstaged} unstaged, {staged} staged")
                };
                p.circle_stroke(pos2(rect.left() + 10.0, rect.center().y), 4.0, Stroke::new(1.5, theme::TEXT_DIM));
                p.text(pos2(rect.left() + 24.0, rect.center().y), Align2::LEFT_CENTER, text, font.clone(), theme::TEXT);
                if resp.clicked() {
                    actions.push(Action::Select(Sel::Working));
                }
                continue;
            }
            if row == 1 + n_commits {
                p.text(rect.center(), Align2::CENTER_CENTER, "Load more commits…", font.clone(), theme::ACCENT);
                if resp.clicked() {
                    actions.push(Action::LoadMore);
                }
                continue;
            }

            let ci = visible.map_or(row - 1, |v| v[row - 1]);
            let c = &snap.commits[ci];
            let selected = matches!(&rv.sel, Sel::Commit(id) if *id == c.id);
            bg(&p, rect, selected, resp.hovered(), row % 2 == 0);

            // --- graph
            let graph_w = if visible.is_none() {
                draw_graph(&p, rect, rv, ci, head.as_deref() == Some(c.id.as_str()))
            } else {
                p.circle_filled(pos2(rect.left() + 10.0, rect.center().y), 3.5, theme::lane(1));
                20.0
            };

            // --- right columns
            // Drop the author column when the list is narrow.
            let date_w = 82.0;
            let author_w = if rect.width() > 560.0 { 130.0 } else { 0.0 };
            let right = rect.right() - 8.0;
            p.text(pos2(right, rect.center().y), Align2::RIGHT_CENTER, rel_time(now, c.time), small.clone(), theme::TEXT_DIM);
            let author_rect = Rect::from_min_max(pos2(right - date_w - author_w, rect.top()), pos2(right - date_w - 6.0, rect.bottom()));
            if author_w > 0.0 {
                ui.painter_at(author_rect).text(
                pos2(author_rect.left(), rect.center().y),
                Align2::LEFT_CENTER,
                &c.author,
                small.clone(),
                theme::TEXT_DIM,
            );
            }

            // --- refs + subject
            let mut x = rect.left() + graph_w + 4.0;
            let subj_clip = Rect::from_min_max(pos2(x, rect.top()), pos2(author_rect.left() - 8.0, rect.bottom()));
            let sp = ui.painter_at(subj_clip);
            if rv.has_note(&NoteTarget::Commit(c.id.clone())) {
                sp.rect_filled(Rect::from_center_size(pos2(x + 4.0, rect.center().y), vec2(6.0, 10.0)), 1.0, theme::ACCENT);
                x += 12.0;
            }
            for r in &c.refs {
                let (label, color) = ref_chip(r);
                let g = sp.layout_no_wrap(label, small.clone(), Color32::BLACK);
                let chip = Rect::from_min_size(pos2(x, rect.center().y - g.size().y / 2.0 - 1.0), g.size() + vec2(10.0, 2.0));
                sp.rect_filled(chip, 3.0, color);
                sp.galley(chip.min + vec2(5.0, 1.0), g, Color32::BLACK);
                x = chip.right() + 4.0;
            }
            sp.text(pos2(x + 2.0, rect.center().y), Align2::LEFT_CENTER, &c.subject, font.clone(), theme::TEXT);

            if resp.clicked() {
                actions.push(Action::Select(Sel::Commit(c.id.clone())));
            }
            resp.on_hover_text_at_pointer(format!("{}\n{} <{}>", c.short_id(), c.author, c.email))
                .context_menu(|ui| commit_menu(ui, c, actions));
        }
    });
}

fn bg(p: &egui::Painter, rect: Rect, selected: bool, hovered: bool, alt: bool) {
    let color = if selected {
        theme::SELECTED
    } else if hovered {
        theme::HOVER
    } else if alt {
        theme::BG_ROW_ALT
    } else {
        return;
    };
    p.rect_filled(rect, 0.0, color);
}

fn draw_graph(p: &egui::Painter, rect: Rect, rv: &RepoView, i: usize, is_head: bool) -> f32 {
    let lanes_x = |c: u16| rect.left() + 10.0 + (c as usize).min(MAX_LANES) as f32 * LANE_W;
    let Some(row) = rv.graph.get(i) else { return 20.0 };
    let (top, mid, bot) = (rect.top(), rect.center().y, rect.bottom());
    // top half: edges from previous row into this one
    for e in &row.edges_up {
        let xm = (lanes_x(e.from) + lanes_x(e.to)) / 2.0;
        p.line_segment([pos2(xm, top), pos2(lanes_x(e.to), mid)], Stroke::new(1.6, theme::lane(e.color)));
    }
    // bottom half: edges from this row into the next one
    let mut widest = row.col;
    if let Some(next) = rv.graph.get(i + 1) {
        for e in &next.edges_up {
            let xm = (lanes_x(e.from) + lanes_x(e.to)) / 2.0;
            p.line_segment([pos2(lanes_x(e.from), mid), pos2(xm, bot)], Stroke::new(1.6, theme::lane(e.color)));
            widest = widest.max(e.from).max(e.to);
        }
    }
    for e in &row.edges_up {
        widest = widest.max(e.to);
    }
    let c = pos2(lanes_x(row.col), mid);
    p.circle_filled(c, 4.0, theme::lane(row.color));
    if is_head {
        p.circle_stroke(c, 6.5, Stroke::new(1.5, theme::TEXT));
    }
    10.0 + (widest as usize).min(MAX_LANES) as f32 * LANE_W + 12.0
}

fn ref_chip(r: &str) -> (String, Color32) {
    if let Some(b) = r.strip_prefix("HEAD -> ") {
        (b.to_string(), theme::ACCENT)
    } else if r == "HEAD" {
        ("HEAD".into(), theme::ACCENT)
    } else if let Some(t) = r.strip_prefix("tag: ") {
        (t.to_string(), Color32::from_rgb(0xc8, 0xa0, 0xe8))
    } else if r.contains('/') && !r.starts_with("refs/") {
        // remote-ish (origin/main) or local with slash; good enough heuristic
        (r.to_string(), Color32::from_rgb(0x9c, 0xb8, 0xd8))
    } else {
        (r.to_string(), Color32::from_rgb(0x9a, 0xd0, 0x8c))
    }
}

fn rel_time(now: i64, t: i64) -> String {
    let d = now - t;
    match d {
        _ if d < 60 => "just now".into(),
        _ if d < 3600 => format!("{}m ago", d / 60),
        _ if d < 86_400 => format!("{}h ago", d / 3600),
        _ if d < 7 * 86_400 => format!("{}d ago", d / 86_400),
        _ => margin_core::format_utc(t)[..10].to_string(),
    }
}

fn commit_menu(ui: &mut Ui, c: &Commit, actions: &mut Vec<Action>) {
    let id = c.id.clone();
    let short = c.short_id().to_string();
    let mut item = |ui: &mut Ui, label: &str, a: Action| {
        if ui.button(label).clicked() {
            actions.push(a);
            ui.close();
        }
    };
    item(ui, "Copy SHA", Action::Copy(id.clone()));
    item(ui, "Add / edit note", Action::Select(Sel::Commit(id.clone())));
    ui.separator();
    item(ui, "New branch here…", Action::NewBranchDialog { start: Some(id.clone()), name: String::new() });
    item(
        ui,
        "Checkout (detached)",
        Action::Confirm { msg: format!("Check out {short} in detached HEAD state?"), action: Box::new(Action::CheckoutDetached(id.clone())) },
    );
    item(ui, "Cherry-pick", Action::Confirm { msg: format!("Cherry-pick {short} onto the current branch?"), action: Box::new(Action::CherryPick(id.clone())) });
    item(ui, "Revert", Action::Confirm { msg: format!("Create a commit reverting {short}?"), action: Box::new(Action::Revert(id.clone())) });
    ui.menu_button("Reset current branch to here", |ui| {
        for (label, mode) in [("Soft (keep index)", ResetMode::Soft), ("Mixed (keep worktree)", ResetMode::Mixed), ("Hard (discard all)", ResetMode::Hard)] {
            if ui.button(label).clicked() {
                actions.push(Action::Confirm {
                    msg: format!("{label}: reset current branch to {short}?"),
                    action: Box::new(Action::Reset(id.clone(), mode)),
                });
                ui.close();
            }
        }
    });
}
