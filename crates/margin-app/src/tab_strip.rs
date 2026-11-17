//! Tab strip above the toolbar: one tab per open directory.
//! A view over `MarginApp::tabs()`; switching/closing go through the same
//! actions as the directory dropdown and ⌘P "Change directory".
//! "+" opens another directory: a folder picker, or one of the recent ones.
//! (Worktrees are listed in the sidebar, next to branches.)

use crate::app::Action;
use crate::theme;
use eframe::egui::{self, Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use std::path::PathBuf;

const H: f32 = 28.0;
/// Width a tab shrinks to before the rest move into the "⋯ +N" menu.
const MIN_W: f32 = 76.0;
const MAX_W: f32 = 220.0;
const PLUS_W: f32 = 34.0;
const MORE_W: f32 = 52.0;

/// Tab labels: the folder name, plus its parent folder when two open tabs
/// share a name ("app · work", "app · oss").
fn labels(tabs: &[(PathBuf, String, bool)]) -> Vec<String> {
    tabs.iter()
        .map(|(path, name, _)| {
            let dup = tabs.iter().filter(|(_, n, _)| n == name).count() > 1;
            match path.parent().and_then(|p| p.file_name()).filter(|_| dup) {
                Some(parent) => format!("{name} · {}", parent.to_string_lossy()),
                None => name.clone(),
            }
        })
        .collect()
}

/// Which tabs fit (a contiguous run that includes the active one) and their
/// width; the others go in the overflow menu.
fn fit(natural: &[f32], active: usize, avail: f32) -> (std::ops::Range<usize>, f32) {
    let n = natural.len();
    let total: f32 = natural.iter().sum();
    if total <= avail || n == 0 {
        return (0..n, f32::NAN); // NAN: natural widths
    }
    if MIN_W * n as f32 <= avail {
        return (0..n, avail / n as f32);
    }
    // not even at the minimum: as many as fit next to the "⋯ +N" button
    let k = (((avail - MORE_W) / MIN_W).floor() as usize).clamp(1, n);
    let start = active.saturating_sub(k - 1).min(n - k);
    (start..start + k, ((avail - MORE_W) / k as f32).max(MIN_W))
}

pub fn show(ui: &mut Ui, tabs: &[(PathBuf, String, bool)], recent: &[String], actions: &mut Vec<Action>) {
    egui::Panel::top("tab_strip")
        .exact_size(H)
        .frame(egui::Frame::new().fill(theme::BG_DARK()))
        .show(ui, |ui| {
            let full = ui.max_rect();
            let font = egui::TextStyle::Body.resolve(ui.style());
            let names = labels(tabs);
            let natural: Vec<f32> = names.iter().map(|n| (ui.painter().layout_no_wrap(n.clone(), font.clone(), theme::TEXT()).size().x + 44.0).clamp(MIN_W, MAX_W)).collect();
            let active = tabs.iter().position(|t| t.2).unwrap_or(0);
            let avail = full.width() - 6.0 - PLUS_W;
            let (shown, even_w) = fit(&natural, active, avail);
            let mut x = full.left() + 6.0;
            // (tab index, its rect) for drag-to-reorder
            let mut rects: Vec<(usize, Rect)> = vec![];
            let drag_id = ui.scope_id().with("tab_drag");
            let dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id));
            for i in shown.clone() {
                let (path, _, active) = &tabs[i];
                let name = &names[i];
                let w = if even_w.is_nan() { natural[i] } else { even_w };
                let rect = Rect::from_min_size(pos2(x, full.top() + 3.0), vec2(w - 2.0, H - 3.0));
                rects.push((i, rect));
                let resp = ui.interact(rect, ui.scope_id().with(("tab", i)), Sense::click_and_drag());
                let hovered = resp.hovered();
                let p = ui.painter();
                // Active tab shares the toolbar colour so it reads as "attached".
                let fill = if *active {
                    theme::BG()
                } else if hovered || dragging == Some(i) {
                    theme::HOVER()
                } else {
                    theme::BG_DARK()
                };
                p.rect_filled(rect, egui::CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }, fill);
                if *active {
                    p.line_segment([rect.left_top() + vec2(5.0, 0.0), rect.right_top() - vec2(5.0, 0.0)], Stroke::new(2.0, theme::ACCENT()));
                } else if i + 1 < tabs.len() && !tabs[i + 1].2 {
                    // thin separator between two inactive tabs
                    p.line_segment([pos2(rect.right() + 1.0, rect.top() + 6.0), pos2(rect.right() + 1.0, rect.bottom() - 6.0)], Stroke::new(1.0, theme::HOVER()));
                }
                // Name, shortened in the middle when it doesn't fit ("app…work"
                // keeps what tells two "app" tabs apart); room left for ×
                let color = if *active { theme::TEXT() } else { theme::TEXT_DIM() };
                // (× shows only on the active or hovered tab: others use its space)
                let room = (rect.width() - 12.0 - if *active || hovered { 24.0 } else { 8.0 }).max(10.0);
                let mut text = name.clone();
                let mut cells = crate::path_elide::width(name);
                let mut g = p.layout_no_wrap(text.clone(), font.clone(), color);
                while g.size().x > room && cells > 2 {
                    cells -= 1;
                    text = crate::path_elide::elide_name(name, cells);
                    g = p.layout_no_wrap(text.clone(), font.clone(), color);
                }
                p.galley(pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0), g, color);

                // × on the active or hovered tab
                let close_rect = Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), vec2(16.0, 16.0));
                let close = ui.interact(close_rect, ui.scope_id().with(("tab_close", i)), Sense::click());
                if *active || hovered || close.hovered() {
                    if close.hovered() {
                        p.rect_filled(close_rect, 3.0, theme::BADGE());
                    }
                    let c = close_rect.center();
                    let s = 3.5;
                    let stroke = Stroke::new(1.3, if close.hovered() { theme::TEXT() } else { theme::TEXT_DIM() });
                    p.line_segment([c - vec2(s, s), c + vec2(s, s)], stroke);
                    p.line_segment([pos2(c.x - s, c.y + s), pos2(c.x + s, c.y - s)], stroke);
                }
                if resp.drag_started() {
                    ui.data_mut(|d| d.insert_temp(drag_id, i));
                }
                if close.clicked() || resp.middle_clicked() {
                    actions.push(Action::CloseTab(i));
                } else if resp.clicked() {
                    actions.push(Action::SwitchTab(i));
                }
                let key = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
                let nth = if i < 8 { format!("\n{key}{} to switch here · drag to reorder", i + 1) } else { "\ndrag to reorder".into() };
                if dragging.is_none() {
                    resp.on_hover_text(format!("{}{nth}", crate::app::tilde(path)));
                }
                x += w;
            }
            // Drag to reorder: a marker where the tab would land; dropped on release.
            if let Some(from) = dragging {
                let px = ui.input(|i| i.pointer.interact_pos().map(|p| p.x));
                let to = px.map(|px| {
                    rects.iter().find(|(_, r)| px < r.center().x).map_or_else(|| rects.last().map_or(from, |(i, _)| *i), |(i, _)| if *i > from { i - 1 } else { *i })
                });
                if let (Some(to), Some(px)) = (to, px) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    if to != from {
                        let mx = rects.iter().find(|(_, r)| px < r.center().x).map_or_else(|| rects.last().map_or(px, |(_, r)| r.right()), |(_, r)| r.left());
                        ui.painter().line_segment([pos2(mx, full.top() + 5.0), pos2(mx, full.bottom() - 3.0)], Stroke::new(2.0, theme::ACCENT()));
                    }
                    if ui.input(|i| i.pointer.any_released()) {
                        if to != from {
                            actions.push(Action::MoveTab { from, to });
                        }
                        ui.data_mut(|d| d.remove::<usize>(drag_id));
                    }
                } else if ui.input(|i| !i.pointer.any_down()) {
                    ui.data_mut(|d| d.remove::<usize>(drag_id));
                }
            }
            // "⋯ +N": the tabs that don't fit
            let hidden: Vec<usize> = (0..tabs.len()).filter(|i| !shown.contains(i)).collect();
            if !hidden.is_empty() {
                let more = Rect::from_min_size(pos2(x + 4.0, full.top() + 5.0), vec2(MORE_W - 8.0, H - 9.0));
                let r = ui.interact(more, ui.scope_id().with("tab_more"), Sense::click());
                let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r));
                ui.painter().rect_filled(more, 4.0, if r.hovered() || open { theme::HOVER() } else { theme::BG_DARK() });
                ui.painter().text(more.center(), Align2::CENTER_CENTER, format!("⋯ +{}", hidden.len()), font.clone(), theme::TEXT_DIM());
                let r = if open { r } else { r.on_hover_text("More open tabs") };
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(200.0);
                    for &i in &hidden {
                        if ui.add(egui::Button::new(names[i].as_str()).frame(false)).on_hover_text(tabs[i].0.display().to_string()).clicked() {
                            actions.push(Action::SwitchTab(i));
                            ui.close();
                        }
                    }
                });
                x += MORE_W;
            }
            // "+" → Open folder… / recent directories
            let plus = Rect::from_min_size(pos2(x + 4.0, full.top() + 5.0), vec2(PLUS_W - 12.0, H - 9.0));
            let r = ui.interact(plus, ui.scope_id().with("tab_plus"), Sense::click());
            let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r));
            if r.hovered() || open {
                ui.painter().rect_filled(plus, 4.0, theme::HOVER());
            }
            ui.painter().text(plus.center(), Align2::CENTER_CENTER, "+", egui::FontId::proportional(17.0), theme::TEXT_DIM());
            let r = if open { r } else { r.on_hover_text("Open a directory") };
            egui::Popup::menu(&r).show(|ui| {
                ui.set_min_width(240.0);
                if ui.button("Open folder…").clicked() {
                    actions.push(Action::OpenFolderDialog);
                    ui.close();
                }
                ui.separator();
                ui.label(egui::RichText::new("RECENT").small().strong().color(theme::TEXT_DIM()));
                if recent.is_empty() {
                    ui.weak("No recent directories");
                }
                for p in recent {
                    let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
                    let parent = std::path::Path::new(p).parent().map(crate::app::tilde).unwrap_or_default();
                    // Keep the menu narrow: "…/src/work" for long parent paths.
                    let parent = match parent.char_indices().rev().nth(32) {
                        Some((i, _)) => format!("…{}", &parent[i + parent[i..].chars().next().map_or(0, char::len_utf8)..]),
                        None => parent,
                    };
                    let mut atoms = egui::Atoms::new(name);
                    atoms.push_right(egui::Atom::grow());
                    atoms.push_right(egui::RichText::new(parent).small().color(theme::TEXT_DIM()));
                    if ui.add(egui::Button::new(atoms).frame(false)).on_hover_text(p).clicked() {
                        actions.push(Action::OpenRepo(PathBuf::from(p)));
                        ui.close();
                    }
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::{MIN_W, fit, labels};
    use std::path::PathBuf;

    #[test]
    fn same_names_get_their_parent() {
        let t = |p: &str| (PathBuf::from(p), PathBuf::from(p).file_name().unwrap().to_string_lossy().into_owned(), false);
        let l = labels(&[t("/w/app"), t("/oss/app"), t("/w/web")]);
        assert_eq!(l, ["app · w", "app · oss", "web"]);
    }

    #[test]
    fn tabs_shrink_then_overflow() {
        let nat = [150.0; 6];
        // room for all at natural width
        assert_eq!(fit(&nat, 0, 1000.0).0, 0..6);
        // shrink evenly
        let (r, w) = fit(&nat, 0, 600.0);
        assert_eq!((r, w), (0..6, 100.0));
        // overflow: the active tab stays visible
        let (r, w) = fit(&nat, 5, 300.0);
        assert!(r.contains(&5) && w >= MIN_W, "{r:?} {w}");
        assert_eq!(r.len(), 3);
    }
}
