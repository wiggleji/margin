//! Virtualised diff view: all files/hunks/lines are flattened into rows and
//! only visible rows are painted, so multi-MB diffs stay smooth.
//!
//! Two layouts, switched from the bar above the diff: inline (unified) and
//! side by side (old left, new right; removed/added runs paired up).
//! Within paired changed lines the changed words are highlighted; whitespace
//! changes can be ignored (git -w); ⌥↑/⌥↓ jump between changes.

use crate::app::HunkOp;
use crate::theme;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use margin_core::diff::{FileDiff, LineKind};
use std::collections::{HashMap, HashSet};

/// Diff display options (remembered in state.json).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffPrefs {
    /// Side by side instead of inline.
    pub split: bool,
    /// `git diff -w`: hide whitespace-only changes.
    pub ignore_ws: bool,
}

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
    /// Changed byte ranges per (file, hunk, line), for word highlighting.
    words: HashMap<(usize, usize, usize), Vec<(usize, usize)>>,
    /// Scroll position (for next / previous change) and a pending jump.
    offset: f32,
    jump: Option<f32>,
    /// Hunk row briefly highlighted after a jump (row, start time).
    flash: Option<(usize, f64)>,
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
        self.words = word_highlights(&self.files);
        self.offset = 0.0;
        self.jump = Some(0.0);
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

    /// `prefs` are changed from the bar above the diff (layout, whitespace);
    /// the caller reloads the diff when `ignore_ws` changes.
    pub fn show(&mut self, ui: &mut Ui, mode: DiffMode, id: &str, prefs: &mut DiffPrefs) -> Option<DiffAction> {
        if self.loading && self.files.is_empty() {
            ui.weak("Loading diff…");
            return None;
        }
        if let Some(e) = &self.error {
            ui.colored_label(theme::ERROR, e);
            return None;
        }
        if self.files.is_empty() {
            ui.horizontal(|ui| {
                ui.weak(if prefs.ignore_ws { "No changes (ignoring whitespace)." } else { "No changes." });
                if prefs.ignore_ws && ui.small_button("Show whitespace changes").clicked() {
                    prefs.ignore_ws = false;
                }
            });
            return None;
        }
        let split = &mut prefs.split;
        if self.truncated {
            ui.colored_label(theme::ACCENT, "Diff truncated (too large). Use the terminal for the full patch.");
        }

        // ---- bar: file count, collapse/expand all, next/previous change, whitespace, layout
        let mut all = None;
        let mut nav: i32 = 0;
        let mut ws = None;
        let ignore_ws = prefs.ignore_ws;
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
                ui.add_space(6.0);
                let t = egui::RichText::new("Ignore whitespace").small();
                if ui.selectable_label(ignore_ws, t).on_hover_text("Hide changes that only touch spaces, tabs or indentation (git diff -w)").clicked() {
                    ws = Some(!ignore_ws);
                }
                ui.add_space(4.0);
                let alt = if cfg!(target_os = "macos") { "⌥" } else { "Alt+" };
                if ui.small_button("↓").on_hover_text(format!("Next change ({alt}↓)")).clicked() {
                    nav = 1;
                }
                if ui.small_button("↑").on_hover_text(format!("Previous change ({alt}↑)")).clicked() {
                    nav = -1;
                }
            });
        });
        if let Some(w) = ws {
            prefs.ignore_ws = w;
        }
        let split = &mut prefs.split;
        // ⌥↓ / ⌥↑ when no text field has focus
        if ui.memory(|m| m.focused().is_none()) {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown) {
                    nav = 1;
                }
                if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp) {
                    nav = -1;
                }
            });
        }
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

        // Next / previous change = the next hunk header below / above the top of the view.
        if nav != 0 {
            let ys = self.rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Hunk(..))).map(|(i, _)| i as f32 * row_h);
            let target = if nav > 0 { ys.clone().find(|&y| y > self.offset + 1.0) } else { ys.filter(|&y| y < self.offset - 1.0).last() };
            if let Some(y) = target {
                self.jump = Some(y);
                self.flash = Some(((y / row_h).round() as usize, ui.input(|i| i.time)));
            }
        }
        let mut area = egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]);
        if let Some(y) = self.jump.take() {
            area = area.vertical_scroll_offset(y);
        }
        let ws_on = prefs.ignore_ws && mode != DiffMode::Commit;
        let words = &self.words;
        let now = ui.input(|i| i.time);
        let flash = self.flash.filter(|&(_, t)| now - t < 0.8).map(|(r, t)| {
            ui.ctx().request_repaint();
            (r, 1.0 - ((now - t) / 0.8) as f32)
        });
        let out = area.show_rows(ui, row_h, self.rows.len(), |ui, range| {
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
                            if let Some((_, a)) = flash.filter(|&(r, _)| r == i) {
                                p.rect_filled(rect, 0.0, theme::ACCENT.gamma_multiply(0.35 * a));
                            }
                            let h = &self.files[fi].hunks[hi];
                            let mut x = rect.left() + 6.0;
                            // Hunks of a `-w` diff don't match the file: no staging from them.
                            let ops: &[(HunkOp, &str)] = match mode {
                                _ if ws_on => &[],
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
                            let text_x = rect.left() + gutter + char_w * 2.0;
                            if let Some(sp) = words.get(&(fi, hi, li)) {
                                paint_words(&p, &l.text, sp, text_x, rect, char_w, l.kind);
                            }
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
                                let text_x = r.left() + char_w * 8.0 + 8.0;
                                if let Some(w) = words.get(&(fi, hi, li)) {
                                    paint_words(&sp, &l.text, w, text_x, r, char_w, l.kind);
                                }
                                let text = format!("{sign} {}", l.text.replace('\t', "    "));
                                sp.text(pos2(r.left() + char_w * 6.0 + 8.0, r.center().y), Align2::LEFT_CENTER, text, font.clone(), fg);
                            }
                            p.line_segment([pos2(rect.left() + half, rect.top()), pos2(rect.left() + half, rect.bottom())], Stroke::new(1.0, theme::BG_DARK));
                        }
                    }
                }
            });

        self.offset = out.state.clamped_offset().y;
        if let Some(fi) = toggle {
            if !self.collapsed.remove(&fi) {
                self.collapsed.insert(fi);
            }
            self.rebuild();
        }
        action
    }
}

/// Word changes for each removed/added line pair (runs paired in order, as
/// in the side-by-side layout).
fn word_highlights(files: &[FileDiff]) -> HashMap<(usize, usize, usize), Vec<(usize, usize)>> {
    let mut out = HashMap::new();
    for (fi, f) in files.iter().enumerate() {
        for (hi, h) in f.hunks.iter().enumerate() {
            let lines = &h.lines;
            let mut i = 0;
            while i < lines.len() {
                if lines[i].kind != LineKind::Removed {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < lines.len() && lines[j].kind == LineKind::Removed {
                    j += 1;
                }
                let mut k = j;
                while k < lines.len() && lines[k].kind == LineKind::Added {
                    k += 1;
                }
                for n in 0..(j - i).min(k - j) {
                    if let Some((a, b)) = margin_core::diff::word_changes(&lines[i + n].text, &lines[j + n].text) {
                        out.insert((fi, hi, i + n), a);
                        out.insert((fi, hi, j + n), b);
                    }
                }
                i = k.max(i + 1);
            }
        }
    }
    out
}

/// Brighter background behind the changed words of a line drawn at `text_x`.
fn paint_words(p: &egui::Painter, text: &str, spans: &[(usize, usize)], text_x: f32, rect: Rect, char_w: f32, kind: LineKind) {
    let color = if kind == LineKind::Added { theme::ADDED_WORD_BG } else { theme::REMOVED_WORD_BG };
    // Column of a byte offset (tabs are drawn as 4 spaces).
    let col = |b: usize| text[..b].chars().map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>() as f32;
    for &(s, e) in spans {
        let r = Rect::from_min_max(pos2(text_x + col(s) * char_w, rect.top() + 1.0), pos2(text_x + col(e) * char_w, rect.bottom() - 1.0));
        p.rect_filled(r, 2.0, color);
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
