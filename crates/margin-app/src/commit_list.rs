//! Commit list with graph lanes (virtualised; painter-drawn for speed).

use crate::app::{Action, RepoView, Sel};
use crate::icons::{self, Icon};
use crate::theme;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use margin_core::git::{Commit, ResetMode};
use margin_core::notes::NoteTarget;

/// Two text lines per commit (subject / author, see `show`).
const ROW_H: f32 = 40.0;
/// Line centres from the row's top / bottom.
const LINE1: f32 = 12.0;
const LINE2: f32 = 11.0;
const LANE_W: f32 = 12.0;
const MAX_LANES: usize = 14;

pub fn show(ui: &mut Ui, rv: &mut RepoView, actions: &mut Vec<Action>) {
    let Some(snap) = rv.snap.as_ref() else {
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };
    // Which branches are in the graph: "3 of 12 branches · Show all".
    {
        use margin_core::history::HistoryFilter;
        let f = &rv.graph_filter;
        let (shown, total) = f.counts(&snap.refs);
        let line = match &f.only {
            Some(b) => Some((format!("Showing only {b}"), "Show major branches", HistoryFilter::reset(false, f.hide_tags))),
            None if shown < total => Some((
                format!("{shown} of {total} branches shown"),
                "Show all",
                HistoryFilter::reset(true, f.hide_tags),
            )),
            None if f.show_all || !f.branches.is_empty() || !f.remotes.is_empty() => {
                Some(("All branches shown".to_string(), "Major branches only", HistoryFilter::reset(false, f.hide_tags)))
            }
            None => None,
        };
        if let Some((text, button, to)) = line {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(text).small().color(theme::TEXT_DIM))
                    .on_hover_text("Use the eye next to each branch in the sidebar to show or hide it.");
                if ui.small_button(button).clicked() {
                    actions.push(Action::SetGraphFilter(to));
                }
            });
        }
    }

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
                let detail = if st.files.is_empty() { "clean".to_string() } else { format!("{unstaged} unstaged, {staged} staged") };
                p.circle_stroke(pos2(rect.left() + 10.0, rect.center().y), 4.0, Stroke::new(1.5, theme::TEXT_DIM));
                p.text(pos2(rect.left() + 24.0, rect.top() + LINE1), Align2::LEFT_CENTER, "Working directory", font.clone(), theme::TEXT);
                p.text(pos2(rect.left() + 24.0, rect.bottom() - LINE2), Align2::LEFT_CENTER, detail, small.clone(), theme::TEXT_DIM);
                if resp.clicked() {
                    actions.push(Action::Select(Sel::Working));
                }
                continue;
            }
            if row == 1 + n_commits {
                // Reaching the end loads the next page by itself (once per load).
                p.text(rect.center(), Align2::CENTER_CENTER, if rv.loading { "Loading more commits…" } else { "Load more commits…" }, font.clone(), theme::TEXT_DIM);
                // (refresh sets `loading` right away, so this asks once)
                if !rv.loading {
                    actions.push(Action::LoadMore);
                }
                let _ = resp;
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

            // Two lines:
            //   subject ……………………………………………… [branch] [tag]
            //   author [note] [branch note] ………… [PR #12] [PROJ-1]  2h ago
            let (y1, y2) = (rect.top() + LINE1, rect.bottom() - LINE2);
            let left = rect.left() + graph_w + 4.0;
            let right = rect.right() - 8.0;
            let isz = small.size + 1.0;
            let tips = rv.tips.get(&c.id);
            let is_tip = |name: &str| tips.is_some_and(|t| t.iter().any(|x| x == name));
            let mut hover_extra = String::new();

            // ---- line 1, right: ref labels, right-aligned (at most ~60% of the row)
            let refs: Vec<(String, Color32)> = c.refs.iter().map(|r| ref_chip(r)).collect();
            let budget = ((right - left) * 0.6).max(60.0);
            let mut shown: Vec<(std::sync::Arc<egui::Galley>, Color32)> = vec![];
            let mut used = 0.0;
            for (k, (label, color)) in refs.iter().enumerate() {
                let g = p.layout_no_wrap(label.clone(), small.clone(), Color32::BLACK);
                let w = g.size().x + 10.0 + 4.0;
                let rest = refs.len() - k - 1;
                // keep room for a "+N" label when more follow
                let reserve = if rest > 0 { 30.0 } else { 0.0 };
                if used + w + reserve > budget && !shown.is_empty() {
                    break;
                }
                used += w;
                shown.push((g, *color));
            }
            let hidden = refs.len() - shown.len();
            if hidden > 0 {
                hover_extra += &format!("\n\nRefs: {}", refs.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(", "));
            }
            let more = (hidden > 0).then(|| p.layout_no_wrap(format!("+{hidden}"), small.clone(), theme::TEXT_DIM));
            let total = used + more.as_ref().map_or(0.0, |g| g.size().x + 4.0);
            let mut x = right - total + 4.0;
            let refs_left = if refs.is_empty() { right } else { x - 4.0 };
            let rp = ui.painter_at(Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.center().y)));
            for (g, color) in shown {
                let chip = Rect::from_min_size(pos2(x, y1 - g.size().y / 2.0 - 1.0), g.size() + vec2(10.0, 2.0));
                rp.rect_filled(chip, 3.0, color);
                let w = chip.width();
                rp.galley(chip.min + vec2(5.0, 1.0), g, Color32::BLACK);
                x += w + 4.0;
            }
            if let Some(g) = more {
                rp.galley(pos2(x, y1 - g.size().y / 2.0), g, theme::TEXT_DIM);
            }

            // ---- line 1, left: subject (clipped before the labels)
            let sp = ui.painter_at(Rect::from_min_max(pos2(left, rect.top()), pos2(refs_left - 8.0, rect.center().y)));
            sp.text(pos2(left + 2.0, y1), Align2::LEFT_CENTER, &c.subject, font.clone(), theme::TEXT);

            // ---- line 2, right (right to left): time, task keys, PR status
            let mut rx = right;
            let tg = p.layout_no_wrap(rel_time(now, c.time), small.clone(), theme::TEXT_DIM);
            rx -= tg.size().x;
            p.galley(pos2(rx, y2 - tg.size().y / 2.0), tg, theme::TEXT_DIM);
            rx -= 8.0;
            // Tasks: keys in the subject, and the task linked to a branch whose tip this is.
            let mut keys = margin_core::tasks::extract_keys(&c.subject);
            for b in tips.into_iter().flatten() {
                if let Some(k) = rv.notes.task_for_branch(b) {
                    if !keys.contains(&k) {
                        keys.push(k);
                    }
                }
            }
            for k in keys.iter().rev() {
                rx = outline_chip_right(&p, rx, y2, k, None, theme::lane(0), &small, isz) - 4.0;
            }
            for b in tips.into_iter().flatten().rev() {
                if let Some(pr) = rv.prs.get(b) {
                    let tone = crate::pr_view::tone_color(pr.readiness().1);
                    // [GitHub mark] #12 ● (CI) — coloured by merge readiness
                    let dot_x = rx - 6.0;
                    p.circle_filled(pos2(dot_x, y2), 2.5, ci_color(pr.ci));
                    rx = outline_chip_right(&p, rx, y2, &format!("#{}   ", pr.number), Some(Icon::Github), tone, &small, isz) - 4.0;
                    hover_extra += &format!("\n\n{}", crate::pr_view::hover_summary(pr));
                }
            }

            // ---- line 2, left: author, then note markers
            let lp = ui.painter_at(Rect::from_min_max(pos2(left, rect.center().y), pos2(rx - 6.0, rect.bottom())));
            let ag = lp.layout_no_wrap(c.author.clone(), small.clone(), theme::TEXT_DIM);
            let mut lx = left + 2.0 + ag.size().x + 6.0;
            lp.galley(pos2(left + 2.0, y2 - ag.size().y / 2.0), ag, theme::TEXT_DIM);
            if rv.has_note(&NoteTarget::Commit(c.id.clone())) {
                // Commit note (bound to this SHA)
                icons::paint(&lp, Icon::Note, Rect::from_center_size(pos2(lx + isz / 2.0, y2), vec2(isz, isz)), theme::ACCENT);
                lx += isz + 6.0;
                if let Some(t) = rv.note_title(&NoteTarget::Commit(c.id.clone())) {
                    hover_extra += &format!("\n\nCommit note: {t}");
                }
            }
            // Branch notes ride on the branch's tip (they follow rebase/amend).
            for r in &c.refs {
                let (label, _) = ref_chip(r);
                if !is_tip(&label) {
                    continue;
                }
                if let Some(title) = rv.note_title(&NoteTarget::Branch(label.clone())) {
                    let short: String = title.chars().take(32).collect();
                    let short = if short.len() < title.len() { format!("{short}…") } else { short };
                    let g = lp.layout_no_wrap(short, small.clone(), theme::ACCENT);
                    let chip = Rect::from_min_size(pos2(lx, y2 - g.size().y / 2.0 - 1.0), g.size() + vec2(isz + 11.0, 2.0));
                    lp.rect_stroke(chip, 3.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
                    icons::paint(&lp, Icon::Note, Rect::from_center_size(pos2(chip.left() + 3.0 + isz / 2.0, chip.center().y), vec2(isz, isz)), theme::ACCENT);
                    lp.galley(chip.min + vec2(isz + 6.0, 1.0), g, theme::ACCENT);
                    lx = chip.right() + 4.0;
                    hover_extra += &format!("\n\nBranch note ({label}): {title}");
                }
            }

            if resp.clicked() {
                actions.push(Action::Select(Sel::Commit(c.id.clone())));
            }
            resp.on_hover_text_at_pointer(format!("{}\n{} <{}>{hover_extra}", c.short_id(), c.author, c.email))
                .context_menu(|ui| commit_menu(ui, c, actions));
        }
    });
}

/// Outlined label (optional icon + text) whose right edge is `right`; returns its left edge.
#[allow(clippy::too_many_arguments)]
fn outline_chip_right(p: &egui::Painter, right: f32, y: f32, text: &str, icon: Option<Icon>, color: Color32, font: &FontId, isz: f32) -> f32 {
    let g = p.layout_no_wrap(text.to_string(), font.clone(), color);
    let icon_w = if icon.is_some() { isz + 3.0 } else { 0.0 };
    let size = g.size() + vec2(icon_w + 10.0, 2.0);
    let chip = Rect::from_min_size(pos2(right - size.x, y - size.y / 2.0), size);
    p.rect_stroke(chip, 3.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    if let Some(i) = icon {
        icons::paint(p, i, Rect::from_center_size(pos2(chip.left() + 3.0 + isz / 2.0, chip.center().y), vec2(isz, isz)), color);
    }
    p.galley(chip.min + vec2(5.0 + icon_w, 1.0), g, color);
    chip.left()
}

fn ci_color(ci: margin_core::github::CiState) -> Color32 {
    use margin_core::github::CiState::*;
    match ci {
        Success => theme::ADDED,
        Failure => theme::ERROR,
        Pending => theme::ACCENT,
        None => theme::TEXT_DIM,
    }
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
