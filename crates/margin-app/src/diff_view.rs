//! Virtualised diff view: all files/hunks/lines are flattened into rows and
//! only visible rows are painted, so multi-MB diffs stay smooth.
//!
//! Two layouts, switched from the bar above the diff: inline (unified) and
//! side by side (old left, new right; removed/added runs paired up).

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
    /// Side by side: (file, hunk, old line, new line) — either side may be empty.
    Pair(usize, usize, Option<usize>, Option<usize>),
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
    /// Rows are built for this layout.
    split: bool,
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
                for l in &h.lines {
                    self.max_chars = self.max_chars.max(l.text.len() + 4 * l.text.matches('\t').count());
                }
                if !self.split {
                    self.rows.extend((0..h.lines.len()).map(|li| Row::Line(fi, hi, li)));
                    continue;
                }
                // Side by side: context on both sides; a run of removed lines
                // followed by added lines is paired up row by row.
                let lines = &h.lines;
                let mut i = 0;
                while i < lines.len() {
                    match lines[i].kind {
                        LineKind::Removed | LineKind::Added => {
                            let mut j = i;
                            while j < lines.len() && lines[j].kind == LineKind::Removed {
                                j += 1;
                            }
                            let mut k = j;
                            while k < lines.len() && lines[k].kind == LineKind::Added {
                                k += 1;
                            }
                            for n in 0..(j - i).max(k - j) {
                                let old = (i + n < j).then_some(i + n);
                                let new = (j + n < k).then_some(j + n);
                                self.rows.push(Row::Pair(fi, hi, old, new));
                            }
                            i = k.max(i + 1);
                        }
                        LineKind::Context | LineKind::NoNewline => {
                            self.rows.push(Row::Pair(fi, hi, Some(i), Some(i)));
                            i += 1;
                        }
                    }
                }
            }
        }
    }

    /// `split`: side-by-side layout (toggled from the bar above the diff).
    pub fn show(&mut self, ui: &mut Ui, mode: DiffMode, id: &str, split: &mut bool) -> Option<DiffAction> {
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

        // ---- bar: file count, collapse/expand all, layout
        let mut all = None;
        ui.horizontal(|ui| {
            let (a, r) = self.files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
            let n = self.files.len();
            ui.label(egui::RichText::new(format!("{n} file{}", if n == 1 { "" } else { "s" })).small().color(theme::TEXT_DIM));
            ui.label(egui::RichText::new(format!("+{a}")).small().color(theme::ADDED));
            ui.label(egui::RichText::new(format!("−{r}")).small().color(theme::REMOVED));
            if n > 1 {
                let collapsed_all = self.collapsed.len() == n;
                if ui.small_button(if collapsed_all { "Expand all" } else { "Collapse all" }).clicked() {
                    all = Some(!collapsed_all);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if layout_button(ui, true, *split).on_hover_text("Side by side").clicked() {
                    *split = true;
                }
                if layout_button(ui, false, !*split).on_hover_text("Inline").clicked() {
                    *split = false;
                }
            });
        });
        if let Some(collapse) = all {
            self.collapsed.clear();
            if collapse {
                self.collapsed.extend(0..self.files.len());
            }
            self.rebuild();
        }
        if self.split != *split {
            self.split = *split;
            self.rebuild();
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
                let width = if self.split {
                    // Each half gets half the visible width (long lines are clipped),
                    // so old and new always sit next to each other.
                    ui.available_width()
                } else {
                    ui.available_width().max(gutter + char_w * (self.max_chars as f32 + 2.0))
                };
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
                            // Status letter: Added / Deleted / Renamed / Modified
                            let (letter, color) = match (&f.old_path, &f.new_path) {
                                (None, Some(_)) => ("A", theme::ADDED),
                                (Some(_), None) => ("D", theme::REMOVED),
                                (Some(o), Some(n)) if o != n => ("R", theme::lane(0)),
                                _ => ("M", theme::ACCENT),
                            };
                            let lr = Rect::from_center_size(pos2(rect.left() + 28.0, rect.center().y), vec2(char_w + 6.0, row_h - 6.0));
                            p.rect_filled(lr, 3.0, color.gamma_multiply(0.25));
                            p.text(lr.center(), Align2::CENTER_CENTER, letter, font.clone(), color);
                            let g = p.layout_no_wrap(label, font.clone(), theme::TEXT);
                            let w = g.size().x;
                            p.galley(pos2(lr.right() + 6.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
                            let x = lr.right() + 18.0 + w;
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
                        Row::Pair(fi, hi, old, new) => {
                            let lines = &self.files[fi].hunks[hi].lines;
                            let half = rect.width() / 2.0;
                            let sides = [
                                (Rect::from_min_size(rect.min, vec2(half, row_h)), old, true),
                                (Rect::from_min_size(rect.min + vec2(half, 0.0), vec2(half, row_h)), new, false),
                            ];
                            for (r, li, is_old) in sides {
                                let sp = ui.painter_at(r);
                                let Some(li) = li else {
                                    // nothing on this side: dim filler
                                    sp.rect_filled(r, 0.0, theme::BG_DARK);
                                    continue;
                                };
                                let l = &lines[li];
                                let (bg, sign, fg) = match l.kind {
                                    LineKind::Added => (Some(theme::ADDED_BG), "+", theme::TEXT),
                                    LineKind::Removed => (Some(theme::REMOVED_BG), "-", theme::TEXT),
                                    LineKind::Context => (None, " ", theme::TEXT),
                                    LineKind::NoNewline => (None, "\\", theme::TEXT_DIM),
                                };
                                if let Some(bg) = bg {
                                    sp.rect_filled(r, 0.0, bg);
                                }
                                let n = if is_old { l.old_no } else { l.new_no };
                                let num = n.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".into());
                                sp.text(pos2(r.left() + 4.0, r.center().y), Align2::LEFT_CENTER, num, font.clone(), theme::TEXT_DIM);
                                let text = format!("{sign} {}", l.text.replace('\t', "    "));
                                sp.text(pos2(r.left() + char_w * 6.0 + 8.0, r.center().y), Align2::LEFT_CENTER, text, font.clone(), fg);
                            }
                            p.line_segment([pos2(rect.left() + half, rect.top()), pos2(rect.left() + half, rect.bottom())], Stroke::new(1.0, theme::BG_DARK));
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

/// Small icon toggle: three lines (inline) or two columns of lines (side by side).
fn layout_button(ui: &mut Ui, split: bool, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 18.0), Sense::click());
    let p = ui.painter();
    if on || resp.hovered() {
        p.rect_filled(rect, 3.0, if on { theme::SELECTED } else { theme::HOVER });
    }
    let c = if on { theme::TEXT } else { theme::TEXT_DIM };
    let b = Rect::from_center_size(rect.center(), vec2(14.0, 11.0));
    let st = Stroke::new(1.2, c);
    p.rect_stroke(b, 1.5, st, egui::StrokeKind::Inside);
    let ys = [b.top() + 3.5, b.center().y, b.bottom() - 3.5];
    if split {
        p.line_segment([pos2(b.center().x, b.top()), pos2(b.center().x, b.bottom())], st);
        for y in ys {
            p.line_segment([pos2(b.left() + 2.5, y), pos2(b.center().x - 2.0, y)], Stroke::new(1.0, c));
            p.line_segment([pos2(b.center().x + 2.0, y), pos2(b.right() - 2.5, y)], Stroke::new(1.0, c));
        }
    } else {
        for y in ys {
            p.line_segment([pos2(b.left() + 2.5, y), pos2(b.right() - 2.5, y)], Stroke::new(1.0, c));
        }
    }
    resp
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
