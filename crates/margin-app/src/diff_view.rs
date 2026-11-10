//! Virtualised diff view: all files/hunks/lines are flattened into rows and
//! only visible rows are painted, so multi-MB diffs stay smooth.

use crate::app::HunkOp;
use crate::theme;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use margin_core::diff::{FileDiff, LineKind};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    Unstaged,
    Staged,
    Commit,
}

#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Hunk(usize, usize),
    Line(usize, usize, usize),
    Note(usize),
}

pub struct DiffAction {
    pub file: usize,
    pub hunk: usize,
    pub op: HunkOp,
}

#[derive(Default)]
pub struct DiffView {
    pub files: Vec<FileDiff>,
    pub truncated: bool,
    pub loading: bool,
    pub error: Option<String>,
    collapsed: HashSet<usize>,
    rows: Vec<Row>,
    max_chars: usize,
}

impl DiffView {
    pub fn set(&mut self, files: Vec<FileDiff>, truncated: bool) {
        self.collapsed.clear();
        if files.len() > 25 {
            // Big commits: start collapsed, like Sublime Merge.
            self.collapsed.extend(0..files.len());
        }
        self.files = files;
        self.truncated = truncated;
        self.loading = false;
        self.error = None;
        self.rebuild();
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.rows.clear();
        self.error = None;
        self.loading = false;
    }

    pub fn set_error(&mut self, e: String) {
        self.clear();
        self.error = Some(e);
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        self.max_chars = 40;
        for (fi, f) in self.files.iter().enumerate() {
            self.rows.push(Row::File(fi));
            if self.collapsed.contains(&fi) {
                continue;
            }
            if f.binary || f.hunks.is_empty() {
                self.rows.push(Row::Note(fi));
            }
            for (hi, h) in f.hunks.iter().enumerate() {
                self.rows.push(Row::Hunk(fi, hi));
                for (li, l) in h.lines.iter().enumerate() {
                    self.max_chars = self.max_chars.max(l.text.len() + 4 * l.text.matches('\t').count());
                    self.rows.push(Row::Line(fi, hi, li));
                }
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, mode: DiffMode, id: &str) -> Option<DiffAction> {
        if self.loading && self.files.is_empty() {
            ui.weak("Loading diff…");
            return None;
        }
        if let Some(e) = &self.error {
            ui.colored_label(theme::ERROR, e);
            return None;
        }
        if self.files.is_empty() {
            ui.weak("No changes.");
            return None;
        }
        if self.truncated {
            ui.colored_label(theme::ACCENT, "Diff truncated (too large). Use the terminal for the full patch.");
        }

        let font = FontId::monospace(ui.style().text_styles[&egui::TextStyle::Monospace].size);
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'm'));
        let row_h = font.size + 5.0;
        let gutter = char_w * 12.0 + 8.0;
        let mut action = None;
        let mut toggle = None;

        egui::ScrollArea::both()
            .id_salt(id)
            .auto_shrink([false, false])
            .show_rows(ui, row_h, self.rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let width = ui.available_width().max(gutter + char_w * (self.max_chars as f32 + 2.0));
                for i in range {
                    let row = self.rows[i];
                    let sense = if matches!(row, Row::File(_)) { Sense::click() } else { Sense::hover() };
                    let (rect, resp) = ui.allocate_exact_size(vec2(width, row_h), sense);
                    let p = ui.painter_at(rect);
                    match row {
                        Row::File(fi) => {
                            let f = &self.files[fi];
                            p.rect_filled(rect, 0.0, theme::BG_DARK);
                            let open = !self.collapsed.contains(&fi);
                            tri(&p, pos2(rect.left() + 10.0, rect.center().y), open, theme::TEXT_DIM);
                            let label = match (&f.old_path, &f.new_path) {
                                (Some(o), Some(n)) if o != n => format!("{o} → {n}"),
                                (None, Some(n)) => format!("{n}  (new)"),
                                (Some(o), None) => format!("{o}  (deleted)"),
                                _ => f.display_path().to_string(),
                            };
                            let g = p.layout_no_wrap(label, font.clone(), theme::TEXT);
                            let w = g.size().x;
                            p.galley(pos2(rect.left() + 22.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
                            let x = rect.left() + 34.0 + w;
                            p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, format!("+{}", f.added), font.clone(), theme::ADDED);
                            p.text(
                                pos2(x + char_w * (f.added.to_string().len() as f32 + 2.5), rect.center().y),
                                Align2::LEFT_CENTER,
                                format!("-{}", f.removed),
                                font.clone(),
                                theme::REMOVED,
                            );
                            if resp.clicked() {
                                toggle = Some(fi);
                            }
                        }
                        Row::Note(fi) => {
                            let f = &self.files[fi];
                            let msg = if f.binary { "Binary file" } else { "No content changes (mode / rename only)" };
                            p.text(pos2(rect.left() + gutter, rect.center().y), Align2::LEFT_CENTER, msg, font.clone(), theme::TEXT_DIM);
                        }
                        Row::Hunk(fi, hi) => {
                            p.rect_filled(rect, 0.0, theme::HUNK_BG);
                            let h = &self.files[fi].hunks[hi];
                            let mut x = rect.left() + 6.0;
                            let ops: &[(HunkOp, &str)] = match mode {
                                DiffMode::Unstaged => &[(HunkOp::Stage, "Stage hunk"), (HunkOp::Discard, "Discard")],
                                DiffMode::Staged => &[(HunkOp::Unstage, "Unstage hunk")],
                                DiffMode::Commit => &[],
                            };
                            let small = FontId::proportional(font.size - 1.0);
                            for (op, text) in ops {
                                let pad = ui.spacing().button_padding.x * 2.0 + 6.0;
                                let bw = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), small.clone(), Color32::WHITE).size().x) + pad;
                                let r = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(bw, row_h - 4.0));
                                let btn = ui.put(r, egui::Button::new(egui::RichText::new(*text).font(small.clone())).wrap_mode(egui::TextWrapMode::Extend));
                                if btn.clicked() {
                                    action = Some(DiffAction { file: fi, hunk: hi, op: *op });
                                }
                                x += bw + 6.0;
                            }
                            p.text(pos2(x + 4.0, rect.center().y), Align2::LEFT_CENTER, &h.header, font.clone(), theme::TEXT_DIM);
                        }
                        Row::Line(fi, hi, li) => {
                            let l = &self.files[fi].hunks[hi].lines[li];
                            let (bg, sign, fg) = match l.kind {
                                LineKind::Added => (Some(theme::ADDED_BG), "+", theme::TEXT),
                                LineKind::Removed => (Some(theme::REMOVED_BG), "-", theme::TEXT),
                                LineKind::Context => (None, " ", theme::TEXT),
                                LineKind::NoNewline => (None, "\\", theme::TEXT_DIM),
                            };
                            if let Some(bg) = bg {
                                p.rect_filled(rect, 0.0, bg);
                            }
                            let num = |n: Option<u32>| n.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".into());
                            p.text(
                                pos2(rect.left() + 4.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                format!("{} {}", num(l.old_no), num(l.new_no)),
                                font.clone(),
                                theme::TEXT_DIM,
                            );
                            let text = format!("{sign} {}", l.text.replace('\t', "    "));
                            p.text(pos2(rect.left() + gutter, rect.center().y), Align2::LEFT_CENTER, text, font.clone(), fg);
                        }
                    }
                }
            });

        if let Some(fi) = toggle {
            if !self.collapsed.remove(&fi) {
                self.collapsed.insert(fi);
            }
            self.rebuild();
        }
        action
    }
}

fn tri(p: &egui::Painter, c: egui::Pos2, open: bool, color: Color32) {
    let s = 4.0;
    let pts = if open {
        vec![pos2(c.x - s, c.y - s / 2.0), pos2(c.x + s, c.y - s / 2.0), pos2(c.x, c.y + s / 2.0 + 1.0)]
    } else {
        vec![pos2(c.x - s / 2.0, c.y - s), pos2(c.x + s / 2.0 + 1.0, c.y), pos2(c.x - s / 2.0, c.y + s)]
    };
    p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}
