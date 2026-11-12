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
const MIN_W: f32 = 110.0;
const MAX_W: f32 = 220.0;

pub fn show(ui: &mut Ui, tabs: &[(PathBuf, String, bool)], recent: &[String], actions: &mut Vec<Action>) {
    egui::Panel::top("tab_strip")
        .exact_size(H)
        .frame(egui::Frame::new().fill(theme::BG_DARK))
        .show(ui, |ui| {
            let full = ui.max_rect();
            let font = egui::TextStyle::Body.resolve(ui.style());
            let mut x = full.left() + 6.0;
            for (i, (path, name, active)) in tabs.iter().enumerate() {
                let g = ui.painter().layout_no_wrap(name.clone(), font.clone(), theme::TEXT);
                let w = (g.size().x + 44.0).clamp(MIN_W, MAX_W).min((full.right() - 40.0 - x).max(60.0));
                let rect = Rect::from_min_size(pos2(x, full.top() + 3.0), vec2(w, H - 3.0));
                let resp = ui.interact(rect, ui.scope_id().with(("tab", i)), Sense::click());
                let hovered = resp.hovered();
                let p = ui.painter();
                // Active tab shares the toolbar colour so it reads as "attached".
                let fill = if *active {
                    theme::BG
                } else if hovered {
                    theme::HOVER
                } else {
                    theme::BG_DARK
                };
                p.rect_filled(rect, egui::CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }, fill);
                if *active {
                    p.line_segment([rect.left_top() + vec2(5.0, 0.0), rect.right_top() - vec2(5.0, 0.0)], Stroke::new(2.0, theme::ACCENT));
                } else if i + 1 < tabs.len() && !tabs[i + 1].2 {
                    // thin separator between two inactive tabs
                    p.line_segment([pos2(rect.right(), rect.top() + 6.0), pos2(rect.right(), rect.bottom() - 6.0)], Stroke::new(1.0, theme::HOVER));
                }
                // Name (clipped to leave room for ×)
                let text_rect = Rect::from_min_max(rect.min + vec2(12.0, 0.0), pos2(rect.right() - 24.0, rect.bottom()));
                let color = if *active { theme::TEXT } else { theme::TEXT_DIM };
                ui.painter_at(text_rect).text(text_rect.left_center(), Align2::LEFT_CENTER, name, font.clone(), color);

                // × on the active or hovered tab
                let close_rect = Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), vec2(16.0, 16.0));
                let close = ui.interact(close_rect, ui.scope_id().with(("tab_close", i)), Sense::click());
                if *active || hovered || close.hovered() {
                    if close.hovered() {
                        p.rect_filled(close_rect, 3.0, theme::BADGE);
                    }
                    let c = close_rect.center();
                    let s = 3.5;
                    let stroke = Stroke::new(1.3, if close.hovered() { theme::TEXT } else { theme::TEXT_DIM });
                    p.line_segment([c - vec2(s, s), c + vec2(s, s)], stroke);
                    p.line_segment([pos2(c.x - s, c.y + s), pos2(c.x + s, c.y - s)], stroke);
                }
                if close.clicked() || resp.middle_clicked() {
                    actions.push(Action::CloseTab(i));
                } else if resp.clicked() {
                    actions.push(Action::SwitchTab(i));
                }
                resp.on_hover_text(path.display().to_string());
                x += w + 2.0;
            }
            // "+" → Open folder… / recent directories
            let plus = Rect::from_min_size(pos2(x + 4.0, full.top() + 5.0), vec2(22.0, H - 9.0));
            let r = ui.interact(plus, ui.scope_id().with("tab_plus"), Sense::click());
            let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r));
            if r.hovered() || open {
                ui.painter().rect_filled(plus, 4.0, theme::HOVER);
            }
            ui.painter().text(plus.center(), Align2::CENTER_CENTER, "+", egui::FontId::proportional(17.0), theme::TEXT_DIM);
            let r = if open { r } else { r.on_hover_text("Open a directory") };
            egui::Popup::menu(&r).show(|ui| {
                ui.set_min_width(240.0);
                if ui.button("Open folder…").clicked() {
                    actions.push(Action::OpenFolderDialog);
                    ui.close();
                }
                ui.separator();
                ui.label(egui::RichText::new("RECENT").small().strong().color(theme::TEXT_DIM));
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
                    atoms.push_right(egui::RichText::new(parent).small().color(theme::TEXT_DIM));
                    if ui.add(egui::Button::new(atoms).frame(false)).on_hover_text(p).clicked() {
                        actions.push(Action::OpenRepo(PathBuf::from(p)));
                        ui.close();
                    }
                }
            });
        });
}
