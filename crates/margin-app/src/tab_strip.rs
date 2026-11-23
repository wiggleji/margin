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
            let width = |i: usize| if even_w.is_nan() { natural[i] } else { even_w };
            // Drag to reorder: the dragged tab follows the cursor (where it was
            // grabbed), the others slide aside to show where it'll land.
            let drag_id = ui.scope_id().with("tab_drag");
            let dragging: Option<(usize, f32)> = ui.data(|d| d.get_temp(drag_id)).filter(|(i, _): &(usize, f32)| shown.contains(i));
            let px = ui.input(|i| i.pointer.interact_pos().map(|p| p.x));
            let left0 = full.left() + 6.0;
            let strip_w: f32 = shown.clone().map(width).sum();
            // where the dragged tab is drawn, and the slot it would take
            let lifted = dragging.and_then(|(from, grab)| {
                let px = px?;
                let w = width(from);
                let x = (px - grab).clamp(left0, left0 + strip_w - w);
                // others in order; the dragged one goes before the first whose middle is right of its middle
                let mut slot = 0;
                let mut ox = left0;
                for i in shown.clone().filter(|&i| i != from) {
                    let wi = width(i);
                    if x + w / 2.0 > ox + wi / 2.0 {
                        slot += 1;
                    }
                    ox += wi;
                }
                Some((from, x, slot))
            });
            // the display order: the dragged tab moved to its slot
            let mut order: Vec<usize> = shown.clone().collect();
            if let Some((from, _, slot)) = lifted {
                order.retain(|&i| i != from);
                order.insert(slot, from);
            }
            let mut x = left0;
            let mut lifted_rect = None;
            for &i in &order {
                let (path, _, active) = &tabs[i];
                let name = &names[i];
                let w = width(i);
                let is_lifted = lifted.is_some_and(|(f, _, _)| f == i);
                // others slide to their place (keyed by path: indices change on a move)
                let tx = if is_lifted { lifted.map_or(x, |l| l.1) } else { ui.ctx().animate_value_with_time(egui::Id::unique(("tab-x", path.as_path())), x, 0.12) };
                let rect = Rect::from_min_size(pos2(tx, full.top() + 3.0), vec2(w - 2.0, H - 3.0));
                x += w;
                let resp = ui.interact(rect, ui.scope_id().with(("tab", path.as_path())), Sense::click_and_drag());
                if is_lifted {
                    lifted_rect = Some((i, rect));
                    continue; // painted last, over the others
                }
                let hovered = resp.hovered() && dragging.is_none();
                paint_tab(ui, rect, name, *active, hovered, false, &font, i + 1 < tabs.len() && !tabs[i + 1].2 && !*active);
                // × on the active or hovered tab
                let close_rect = Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), vec2(16.0, 16.0));
                let close = ui.interact(close_rect, ui.scope_id().with(("tab_close", path.as_path())), Sense::click());
                if dragging.is_none() && (*active || hovered || close.hovered()) {
                    paint_close(ui, close_rect, close.hovered());
                }
                if resp.drag_started() {
                    let grab = resp.interact_pointer_pos().map_or(w / 2.0, |p| p.x - rect.left());
                    ui.data_mut(|d| d.insert_temp(drag_id, (i, grab)));
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
            }
            // the dragged tab, lifted: a shadow, over everything
            if let Some((i, rect)) = lifted_rect {
                let painter = ui.painter().clone().with_layer_id(egui::LayerId::new(egui::Order::Foreground, ui.scope_id().with("tab-lifted")));
                painter.rect_filled(rect.translate(vec2(0.0, 2.0)).expand(1.0), 6.0, egui::Color32::from_black_alpha(70));
                let mut sub = ui.new_child(egui::UiBuilder::new().max_rect(rect).layer_id(painter.layer_id()));
                paint_tab(&mut sub, rect, &names[i], tabs[i].2, true, true, &font, false);
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if let Some((from, _)) = dragging {
                if ui.input(|i| !i.pointer.any_down()) {
                    if let Some((_, _, slot)) = lifted {
                        let to = shown.start + slot;
                        if to != from {
                            actions.push(Action::MoveTab { from, to });
                        }
                    }
                    ui.data_mut(|d| d.remove::<(usize, f32)>(drag_id));
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

/// A tab: its background (the active one attached to the toolbar, with the
/// accent line), the name shortened in the middle to fit, a separator.
#[allow(clippy::too_many_arguments)]
fn paint_tab(ui: &mut Ui, rect: Rect, name: &str, active: bool, hovered: bool, lifted: bool, font: &egui::FontId, separator: bool) {
    let p = ui.painter();
    let fill = if active {
        theme::BG()
    } else if hovered || lifted {
        theme::HOVER()
    } else {
        theme::BG_DARK()
    };
    let radius = if lifted { egui::CornerRadius::same(5) } else { egui::CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 } };
    p.rect_filled(rect, radius, fill);
    if active {
        p.line_segment([rect.left_top() + vec2(5.0, 0.0), rect.right_top() - vec2(5.0, 0.0)], Stroke::new(2.0, theme::ACCENT()));
    } else if separator {
        // thin separator between two inactive tabs
        p.line_segment([pos2(rect.right() + 1.0, rect.top() + 6.0), pos2(rect.right() + 1.0, rect.bottom() - 6.0)], Stroke::new(1.0, theme::HOVER()));
    }
    // Name, shortened in the middle when it doesn't fit ("app…work" keeps
    // what tells two "app" tabs apart); room left for ×
    let color = if active || lifted { theme::TEXT() } else { theme::TEXT_DIM() };
    let room = (rect.width() - 12.0 - if active || hovered { 24.0 } else { 8.0 }).max(10.0);
    let mut text = name.to_string();
    let mut cells = crate::path_elide::width(name);
    let mut g = p.layout_no_wrap(text.clone(), font.clone(), color);
    while g.size().x > room && cells > 2 {
        cells -= 1;
        text = crate::path_elide::elide_name(name, cells);
        g = p.layout_no_wrap(text.clone(), font.clone(), color);
    }
    p.galley(pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0), g, color);
}

fn paint_close(ui: &Ui, r: Rect, hovered: bool) {
    let p = ui.painter();
    if hovered {
        p.rect_filled(r, 3.0, theme::BADGE());
    }
    let c = r.center();
    let s = 3.5;
    let stroke = Stroke::new(1.3, if hovered { theme::TEXT() } else { theme::TEXT_DIM() });
    p.line_segment([c - vec2(s, s), c + vec2(s, s)], stroke);
    p.line_segment([pos2(c.x - s, c.y + s), pos2(c.x + s, c.y - s)], stroke);
}
