//! Virtualised diff view: all files/hunks/lines are flattened into rows and
//! only visible rows are painted, so multi-MB diffs stay smooth.
//!
//! Two layouts, switched from the bar above the diff: inline (unified) and
//! side by side (old left, new right; removed/added runs paired up).
//! Long lines wrap onto extra rows (no sideways scrolling).
//! The code text can be selected by dragging (not line numbers or signs; in
//! side by side, one side at a time) and copied with ⌘C or the context menu.
//! Within paired changed lines the changed words are highlighted; whitespace
//! changes can be ignored (git -w); ⌥↑/⌥↓ jump between changes.

use crate::app::HunkOp;
use crate::theme;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use margin_core::diff::{FileDiff, LineKind};
use std::collections::HashMap;

/// Diff display options (remembered in state.json).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffPrefs {
    /// Side by side instead of inline.
    pub split: bool,
    /// `git diff -w`: hide whitespace-only changes.
    pub ignore_ws: bool,
    /// Files start expanded in commits / staged / unstaged (Settings → Diffs).
    pub expand_commit: bool,
    pub expand_staged: bool,
    pub expand_unstaged: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    /// Working directory: an Unstaged and a Staged section, per-file buttons.
    Working,
    Commit,
}

/// Which list a file belongs to (decides its default expansion and buttons).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    Commit,
    Conflicted,
    Unstaged,
    Staged,
}

/// Working-directory diff, loaded in the background.
pub struct WorkDiff {
    pub unstaged: Vec<FileDiff>,
    pub staged: Vec<FileDiff>,
    /// Paths with merge conflicts (shown first, no diff).
    pub conflicted: Vec<String>,
    pub truncated: bool,
}

/// Something the user did in the diff that the app carries out.
pub enum DiffEvent {
    Hunk(DiffAction),
    Stage(Vec<String>),
    Unstage(Vec<String>),
    /// Asks for confirmation first.
    Discard(Vec<String>),
    CopyPath(String),
}

#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Hunk(usize, usize),
    /// "UNSTAGED (3)" / "STAGED (1)" header with stage-all buttons.
    Section(Section),
    /// (file, hunk, line, wrapped segment)
    Line(usize, usize, usize, usize),
    /// Side by side: (file, hunk, old line, new line, wrapped segment) — either side may be empty.
    Pair(usize, usize, Option<usize>, Option<usize>, usize),
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
    /// Section of each file (parallel to `files`).
    sections: Vec<Section>,
    /// Files the user expanded/collapsed by hand, by (section, path); kept
    /// across reloads of the working directory. `true` = expanded.
    toggled: HashMap<(Section, String), bool>,
    /// Expansion defaults the rows were built with.
    defaults: (bool, bool, bool),
    rows: Vec<Row>,
    /// Columns per visual row; longer lines wrap.
    cols: usize,
    /// Rows are built for this layout.
    split: bool,
    /// Changed byte ranges per (file, hunk, line), for word highlighting.
    words: HashMap<(usize, usize, usize), Vec<(usize, usize)>>,
    /// Scroll position (for next / previous change) and a pending jump.
    offset: f32,
    jump: Option<f32>,
    /// Hunk row briefly highlighted after a jump (row, start time).
    flash: Option<(usize, f64)>,
    /// Text selection in the code, and whether a drag is extending it.
    sel: Option<TextSel>,
    dragging: bool,
}

/// Which text column a selection is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Inline,
    Old,
    New,
}

/// Selection from `a` to `b` (row, column within that row's wrapped segment).
#[derive(Clone, Copy, PartialEq, Debug)]
struct TextSel {
    side: Side,
    a: (usize, usize),
    b: (usize, usize),
}

impl TextSel {
    fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.a <= self.b { (self.a, self.b) } else { (self.b, self.a) }
    }
}

/// Where rows are on screen (for mapping the pointer to text).
#[derive(Clone, Copy)]
struct Geo {
    x0: f32,
    top0: f32,
    width: f32,
    row_h: f32,
    char_w: f32,
    gutter: f32,
}

/// The selectable text of one row on one side.
struct RowText<'a> {
    /// Identity of the source line (file, hunk, line) — wrapped rows share it.
    line: (usize, usize, usize),
    text: &'a str,
    /// This row's columns of the line (tabs = 4 columns).
    from: usize,
    to: usize,
    /// x of column `from`, and the clickable text area.
    text_x: f32,
    rect: Rect,
}

impl DiffView {
    /// A commit's diff (all files in one list).
    pub fn set(&mut self, files: Vec<FileDiff>, truncated: bool) {
        self.toggled.clear();
        self.sections = vec![Section::Commit; files.len()];
        self.set_files(files, truncated);
    }

    /// The working directory: conflicts, unstaged, then staged files.
    /// Hand-made expand/collapse choices survive the reload.
    pub fn set_work(&mut self, w: WorkDiff) {
        let mut files = vec![];
        let mut sections = vec![];
        for p in &w.conflicted {
            files.push(FileDiff { old_path: Some(p.clone()), new_path: Some(p.clone()), header: vec![], hunks: vec![], binary: false, added: 0, removed: 0 });
            sections.push(Section::Conflicted);
        }
        sections.extend(std::iter::repeat_n(Section::Unstaged, w.unstaged.len()));
        sections.extend(std::iter::repeat_n(Section::Staged, w.staged.len()));
        files.extend(w.unstaged);
        files.extend(w.staged);
        self.sections = sections;
        let (offset, jump) = (self.offset, self.jump);
        self.set_files(files, w.truncated);
        // keep the scroll position while staging / unstaging
        self.offset = offset;
        self.jump = jump.or(Some(offset));
    }

    fn set_files(&mut self, files: Vec<FileDiff>, truncated: bool) {
        self.files = files;
        self.truncated = truncated;
        self.loading = false;
        self.error = None;
        self.words = word_highlights(&self.files);
        self.sel = None;
        self.dragging = false;
        self.offset = 0.0;
        self.jump = Some(0.0);
        self.rebuild();
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.sections.clear();
        self.rows.clear();
        self.error = None;
        self.loading = false;
    }

    pub fn set_error(&mut self, e: String) {
        self.clear();
        self.error = Some(e);
    }

    /// The selectable text of `row` on `side`, positioned by `geo`.
    fn row_text(&self, row: usize, side: Side, geo: &Geo) -> Option<RowText<'_>> {
        let top = geo.top0 + row as f32 * geo.row_h;
        let (fi, hi, li, seg, left, right) = match (*self.rows.get(row)?, side) {
            (Row::Line(fi, hi, li, seg), Side::Inline) => (fi, hi, li, seg, geo.x0 + geo.gutter, geo.x0 + geo.width),
            (Row::Pair(fi, hi, Some(li), _, seg), Side::Old) => (fi, hi, li, seg, geo.x0 + geo.char_w * 6.0 + 8.0, geo.x0 + geo.width / 2.0),
            (Row::Pair(fi, hi, _, Some(li), seg), Side::New) => (fi, hi, li, seg, geo.x0 + geo.width / 2.0 + geo.char_w * 6.0 + 8.0, geo.x0 + geo.width),
            _ => return None,
        };
        let text = self.files[fi].hunks[hi].lines[li].text.as_str();
        let (from, to) = *wrap_ranges(text, self.cols.max(10)).get(seg)?;
        let rect = Rect::from_min_max(pos2(left - geo.char_w * 0.5, top), pos2(right, top + geo.row_h));
        Some(RowText { line: (fi, hi, li), text, from, to, text_x: left, rect })
    }

    /// The selected code as plain text: original characters (tabs kept),
    /// wrapped rows of one line joined, lines separated by newlines.
    fn selected_text(&self, geo: Option<Geo>) -> String {
        let (Some(s), Some(geo)) = (self.sel, geo) else { return String::new() };
        let (a, b) = s.ordered();
        let mut out = String::new();
        let mut prev = None;
        for row in a.0..=b.0 {
            let Some(t) = self.row_text(row, s.side, &geo) else { continue };
            let c0 = t.from + if row == a.0 { a.1 } else { 0 };
            let c1 = if row == b.0 { t.from + b.1 } else { t.to };
            if prev.is_some() && prev != Some(t.line) {
                out.push('\n');
            }
            out.push_str(&slice_cols(t.text, c0, c1));
            prev = Some(t.line);
        }
        out
    }

    /// How far the diff is scrolled down (points; from the last frame).
    pub fn scrolled(&self) -> f32 {
        self.offset
    }

    pub fn section(&self, fi: usize) -> Option<Section> {
        self.sections.get(fi).copied()
    }

    fn is_open(&self, fi: usize) -> bool {
        let sec = self.sections.get(fi).copied().unwrap_or(Section::Commit);
        let key = (sec, self.files[fi].display_path().to_string());
        self.toggled.get(&key).copied().unwrap_or(match sec {
            // very large commits start collapsed regardless
            Section::Commit => self.defaults.0 && self.files.len() <= 200,
            Section::Staged => self.defaults.1,
            Section::Unstaged => self.defaults.2,
            Section::Conflicted => true,
        })
    }

    fn set_open(&mut self, fi: usize, open: bool) {
        let sec = self.sections.get(fi).copied().unwrap_or(Section::Commit);
        self.toggled.insert((sec, self.files[fi].display_path().to_string()), open);
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        let cols = self.cols.max(10);
        // Visual rows a line takes (tabs count as 4 columns).
        let segs = |t: &str| wrap_ranges(t, cols).len();
        let open: Vec<bool> = (0..self.files.len()).map(|fi| self.is_open(fi)).collect();
        let mut last = None;
        for (fi, f) in self.files.iter().enumerate() {
            let sec = self.sections.get(fi).copied().unwrap_or(Section::Commit);
            if matches!(sec, Section::Unstaged | Section::Staged) && last != Some(sec) {
                self.rows.push(Row::Section(sec));
            }
            last = Some(sec);
            self.rows.push(Row::File(fi));
            if !open[fi] {
                continue;
            }
            if f.binary || f.hunks.is_empty() {
                self.rows.push(Row::Note(fi));
            }
            for (hi, h) in f.hunks.iter().enumerate() {
                self.rows.push(Row::Hunk(fi, hi));
                if !self.split {
                    for (li, l) in h.lines.iter().enumerate() {
                        self.rows.extend((0..segs(&l.text)).map(|sg| Row::Line(fi, hi, li, sg)));
                    }
                    continue;
                }
                let pair = |rows: &mut Vec<Row>, old: Option<usize>, new: Option<usize>| {
                    let n = [old, new].iter().flatten().map(|&x| segs(&h.lines[x].text)).max().unwrap_or(1);
                    rows.extend((0..n).map(|sg| Row::Pair(fi, hi, old, new, sg)));
                };
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
                                pair(&mut self.rows, old, new);
                            }
                            i = k.max(i + 1);
                        }
                        LineKind::Context | LineKind::NoNewline => {
                            pair(&mut self.rows, Some(i), Some(i));
                            i += 1;
                        }
                    }
                }
            }
        }
    }

    /// `prefs` are changed from the bar above the diff (layout, whitespace);
    /// the caller reloads the diff when `ignore_ws` changes.
    pub fn show(&mut self, ui: &mut Ui, mode: DiffMode, id: &str, prefs: &mut DiffPrefs) -> Vec<DiffEvent> {
        let mut events = vec![];
        if self.loading && self.files.is_empty() {
            ui.weak("Loading diff…");
            return events;
        }
        if let Some(e) = &self.error {
            ui.colored_label(theme::ERROR, e);
            return events;
        }
        if self.files.is_empty() {
            ui.horizontal(|ui| {
                let none = if mode == DiffMode::Working { "No changes in the working directory." } else { "No changes." };
                ui.weak(if prefs.ignore_ws { "No changes (ignoring whitespace)." } else { none });
                if prefs.ignore_ws && ui.small_button("Show whitespace changes").clicked() {
                    prefs.ignore_ws = false;
                }
            });
            return events;
        }
        let defaults = (prefs.expand_commit, prefs.expand_staged, prefs.expand_unstaged);
        if defaults != self.defaults {
            self.defaults = defaults;
            self.rebuild();
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
                let collapsed_all = (0..n).all(|fi| !self.is_open(fi));
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
            for fi in 0..self.files.len() {
                self.set_open(fi, !collapse);
            }
            self.rebuild();
        }
        if self.split != *split {
            self.split = *split;
            self.rebuild();
            self.sel = None;
        }

        let font = FontId::monospace(ui.style().text_styles[&egui::TextStyle::Monospace].size);
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'm'));
        let row_h = font.size + 5.0;
        let gutter = char_w * 12.0 + 8.0;
        let mut toggle = None;

        // Wrap width in columns (minus room for the scrollbar); rows depend on it.
        let avail = ui.available_width() - 18.0;
        let text_w = if self.split { avail / 2.0 - (char_w * 6.0 + 8.0) } else { avail - gutter };
        // (one column spare: glyph widths aren't exactly `char_w` in every font)
        let cols = ((text_w / char_w).floor() as usize).saturating_sub(1).max(10);
        if cols != self.cols {
            self.cols = cols;
            self.rebuild();
            self.sel = None;
        }

        // Next / previous change = the next hunk header below / above the top of the view.
        if nav != 0 {
            let ys = self.rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Hunk(..))).map(|(i, _)| i as f32 * row_h);
            let target = if nav > 0 { ys.clone().find(|&y| y > self.offset + 1.0) } else { ys.filter(|&y| y < self.offset - 1.0).last() };
            if let Some(y) = target {
                self.jump = Some(y);
                self.flash = Some(((y / row_h).round() as usize, ui.input(|i| i.time)));
            }
        }
        // (mouse drags select text; egui only drag-scrolls on touch screens)
        let mut area = egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]);
        if let Some(y) = self.jump.take() {
            area = area.vertical_scroll_offset(y);
        }
        let ws_on = prefs.ignore_ws && mode == DiffMode::Working;
        let open: Vec<bool> = (0..self.files.len()).map(|fi| self.is_open(fi)).collect();
        let counts = |s: Section| self.sections.iter().filter(|&&x| x == s).count();
        let (n_unstaged, n_staged) = (counts(Section::Unstaged), counts(Section::Staged));
        let words = &self.words;
        let now = ui.input(|i| i.time);
        let flash = self.flash.filter(|&(_, t)| now - t < 0.8).map(|(r, t)| {
            ui.ctx().request_repaint();
            (r, 1.0 - ((now - t) / 0.8) as f32)
        });
        let mut first: Option<Geo> = None;
        let mut new_sel: Option<Option<TextSel>> = None;
        let mut drag_start = false;
        let mut auto_scroll = 0.0;
        let mut copy_now = false;
        let base_id = ui.scope_id().with(id).with("text");
        let out = area.show_rows(ui, row_h, self.rows.len(), |ui, range| {
                let visible = range.clone();
                ui.spacing_mut().item_spacing.y = 0.0;
                let width = ui.available_width();
                for i in range {
                    let row = self.rows[i];
                    let sense = if matches!(row, Row::File(_)) { Sense::click() } else { Sense::hover() };
                    let small = FontId::proportional(font.size - 1.0);
                    // A small button drawn at `x` in this row; returns (clicked, its width).
                    let button = |ui: &mut Ui, x: f32, rect: Rect, text: &str| {
                        let pad = ui.spacing().button_padding.x * 2.0 + 6.0;
                        let bw = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), small.clone(), Color32::WHITE).size().x) + pad;
                        let r = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(bw, row_h - 4.0));
                        let clicked = ui.put(r, egui::Button::new(egui::RichText::new(text).font(small.clone())).wrap_mode(egui::TextWrapMode::Extend)).clicked();
                        (clicked, bw)
                    };
                    let (rect, resp) = ui.allocate_exact_size(vec2(width, row_h), sense);
                    if first.is_none() {
                        first = Some(Geo { x0: rect.left(), top0: rect.top() - i as f32 * row_h, width: rect.width(), row_h, char_w, gutter });
                    }
                    let p = ui.painter_at(rect);
                    match row {
                        Row::Section(sec) => {
                            let (title, n) = if sec == Section::Staged { ("STAGED", n_staged) } else { ("UNSTAGED", n_unstaged) };
                            let g = p.layout_no_wrap(format!("{title} ({n})"), small.clone(), theme::TEXT_DIM);
                            let gw = g.size().x;
                            p.galley(pos2(rect.left() + 6.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT_DIM);
                            let paths = |s: Section| -> Vec<String> {
                                self.files.iter().zip(&self.sections).filter(|(_, x)| **x == s).map(|(f, _)| f.display_path().to_string()).collect()
                            };
                            let mut x = rect.left() + 18.0 + gw;
                            if sec == Section::Unstaged {
                                let (c, w) = button(ui, x, rect, "Stage all");
                                if c {
                                    events.push(DiffEvent::Stage(paths(Section::Unstaged)));
                                }
                                x += w + 6.0;
                                if button(ui, x, rect, "Discard all").0 {
                                    events.push(DiffEvent::Discard(paths(Section::Unstaged)));
                                }
                            } else if button(ui, x, rect, "Unstage all").0 {
                                events.push(DiffEvent::Unstage(paths(Section::Staged)));
                            }
                        }
                        Row::File(fi) => {
                            let f = &self.files[fi];
                            let sec = self.sections.get(fi).copied().unwrap_or(Section::Commit);
                            p.rect_filled(rect, 0.0, theme::BG_DARK);
                            let open = open[fi];
                            tri(&p, pos2(rect.left() + 10.0, rect.center().y), open, theme::TEXT_DIM);
                            let label = match (&f.old_path, &f.new_path) {
                                (Some(o), Some(n)) if o != n => format!("{o} → {n}"),
                                (None, Some(n)) => format!("{n}  (new)"),
                                (Some(o), None) => format!("{o}  (deleted)"),
                                _ => f.display_path().to_string(),
                            };
                            // Status letter: Added / Deleted / Renamed / Modified
                            let (letter, color) = match (&f.old_path, &f.new_path) {
                                _ if sec == Section::Conflicted => ("U", theme::ERROR),
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
                            // Working directory: per-file buttons on the right, and a menu.
                            let path = f.display_path().to_string();
                            if matches!(sec, Section::Unstaged | Section::Staged | Section::Conflicted) {
                                let labels: &[&str] = match sec {
                                    Section::Staged => &["Unstage"],
                                    Section::Conflicted => &["Stage"],
                                    _ => &["Discard", "Stage"],
                                };
                                let mut bx = rect.right() - 6.0;
                                for &l in labels.iter().rev() {
                                    let pad = ui.spacing().button_padding.x * 2.0 + 6.0;
                                    let bw = ui.fonts_mut(|f| f.layout_no_wrap(l.to_string(), small.clone(), Color32::WHITE).size().x) + pad;
                                    bx -= bw;
                                    if button(ui, bx, rect, l).0 {
                                        events.push(match l {
                                            "Unstage" => DiffEvent::Unstage(vec![path.clone()]),
                                            "Discard" => DiffEvent::Discard(vec![path.clone()]),
                                            _ => DiffEvent::Stage(vec![path.clone()]),
                                        });
                                    }
                                    bx -= 6.0;
                                }
                            }
                            if resp.double_clicked() && matches!(sec, Section::Unstaged | Section::Staged) {
                                events.push(if sec == Section::Staged { DiffEvent::Unstage(vec![path.clone()]) } else { DiffEvent::Stage(vec![path.clone()]) });
                            } else if resp.clicked() {
                                toggle = Some(fi);
                            }
                            resp.context_menu(|ui| {
                                match sec {
                                    Section::Unstaged | Section::Conflicted => {
                                        if ui.button("Stage").clicked() {
                                            events.push(DiffEvent::Stage(vec![path.clone()]));
                                            ui.close();
                                        }
                                        if sec == Section::Unstaged && ui.button("Discard changes…").clicked() {
                                            events.push(DiffEvent::Discard(vec![path.clone()]));
                                            ui.close();
                                        }
                                    }
                                    Section::Staged => {
                                        if ui.button("Unstage").clicked() {
                                            events.push(DiffEvent::Unstage(vec![path.clone()]));
                                            ui.close();
                                        }
                                    }
                                    Section::Commit => {}
                                }
                                if ui.button("Copy path").clicked() {
                                    events.push(DiffEvent::CopyPath(path.clone()));
                                    ui.close();
                                }
                            });
                        }
                        Row::Note(fi) => {
                            let f = &self.files[fi];
                            let msg = if self.sections.get(fi) == Some(&Section::Conflicted) {
                                "Merge conflict: resolve it in your editor, then stage the file."
                            } else if f.binary {
                                "Binary file"
                            } else {
                                "No content changes (mode / rename only)"
                            };
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
                            let ops: &[(HunkOp, &str)] = match self.sections.get(fi) {
                                _ if ws_on => &[],
                                Some(Section::Unstaged) => &[(HunkOp::Stage, "Stage hunk"), (HunkOp::Discard, "Discard")],
                                Some(Section::Staged) => &[(HunkOp::Unstage, "Unstage hunk")],
                                _ => &[],
                            };
                            for (op, text) in ops {
                                let pad = ui.spacing().button_padding.x * 2.0 + 6.0;
                                let bw = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), small.clone(), Color32::WHITE).size().x) + pad;
                                let r = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(bw, row_h - 4.0));
                                let btn = ui.put(r, egui::Button::new(egui::RichText::new(*text).font(small.clone())).wrap_mode(egui::TextWrapMode::Extend));
                                if btn.clicked() {
                                    events.push(DiffEvent::Hunk(DiffAction { file: fi, hunk: hi, op: *op }));
                                }
                                x += bw + 6.0;
                            }
                            p.text(pos2(x + 4.0, rect.center().y), Align2::LEFT_CENTER, &h.header, font.clone(), theme::TEXT_DIM);
                        }
                        Row::Line(fi, hi, li, seg) => {
                            let l = &self.files[fi].hunks[hi].lines[li];
                            let (bg, fg) = match l.kind {
                                LineKind::Added => (Some(theme::ADDED_BG), theme::TEXT),
                                LineKind::Removed => (Some(theme::REMOVED_BG), theme::TEXT),
                                LineKind::Context => (None, theme::TEXT),
                                LineKind::NoNewline => (None, theme::TEXT_DIM),
                            };
                            // colour only the code, not the line numbers
                            if let Some(bg) = bg {
                                p.rect_filled(Rect::from_min_max(pos2(rect.left() + gutter - 4.0, rect.top()), rect.max), 0.0, bg);
                            }
                            if seg == 0 {
                                let num = |n: Option<u32>| n.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".into());
                                p.text(
                                    pos2(rect.left() + 4.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    format!("{} {}", num(l.old_no), num(l.new_no)),
                                    font.clone(),
                                    theme::TEXT_DIM,
                                );
                            }
                            let seg_text = Seg { text: &l.text, seg, cols: self.cols };
                            seg_text.paint(&p, rect, rect.left() + gutter, words.get(&(fi, hi, li)), l.kind, &font, fg, char_w);
                        }
                        Row::Pair(fi, hi, old, new, seg) => {
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
                                let st = Seg { text: &l.text, seg, cols: self.cols };
                                if seg >= st.count() {
                                    // the other side wrapped further: keep the line's background
                                    if let Some(bg) = match l.kind { LineKind::Added => Some(theme::ADDED_BG), LineKind::Removed => Some(theme::REMOVED_BG), _ => None } {
                                        sp.rect_filled(Rect::from_min_max(pos2(r.left() + char_w * 6.0 + 4.0, r.top()), r.max), 0.0, bg);
                                    }
                                    continue;
                                }
                                let (bg, fg) = match l.kind {
                                    LineKind::Added => (Some(theme::ADDED_BG), theme::TEXT),
                                    LineKind::Removed => (Some(theme::REMOVED_BG), theme::TEXT),
                                    LineKind::Context => (None, theme::TEXT),
                                    LineKind::NoNewline => (None, theme::TEXT_DIM),
                                };
                                if let Some(bg) = bg {
                                    sp.rect_filled(Rect::from_min_max(pos2(r.left() + char_w * 6.0 + 4.0, r.top()), r.max), 0.0, bg);
                                }
                                if seg == 0 {
                                    let n = if is_old { l.old_no } else { l.new_no };
                                    let num = n.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".into());
                                    sp.text(pos2(r.left() + 4.0, r.center().y), Align2::LEFT_CENTER, num, font.clone(), theme::TEXT_DIM);
                                }
                                st.paint(&sp, r, r.left() + char_w * 6.0 + 8.0, words.get(&(fi, hi, li)), l.kind, &font, fg, char_w);
                            }
                            p.line_segment([pos2(rect.left() + half, rect.top()), pos2(rect.left() + half, rect.bottom())], Stroke::new(1.0, theme::BG_DARK));
                        }
                    }
                }

                // ---- text selection: drag over code text (not numbers / signs / headers)
                let Some(geo) = first else { return };
                let sides: &[Side] = if self.split { &[Side::Old, Side::New] } else { &[Side::Inline] };
                let pointer = ui.input(|i| i.pointer.interact_pos());
                let pos_of = |p: egui::Pos2, side: Side| -> (usize, usize) {
                    let row = (((p.y - geo.top0) / geo.row_h).floor().max(0.0) as usize).min(self.rows.len().saturating_sub(1));
                    let col = self.row_text(row, side, &geo).map_or(0, |t| (((p.x - t.text_x) / geo.char_w).round().max(0.0) as usize).min(t.to - t.from));
                    (row, col)
                };
                for i in visible.clone() {
                    for &side in sides {
                        let Some(t) = self.row_text(i, side, &geo) else { continue };
                        let resp = ui.interact(t.rect, base_id.with((i, side as u8)), Sense::click_and_drag());
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                        }
                        if resp.drag_started() {
                            // start where the button went down, not where the drag was recognised
                            if let Some(p) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                                let at = pos_of(p, side);
                                new_sel = Some(Some(TextSel { side, a: at, b: at }));
                                drag_start = true;
                            }
                        } else if resp.double_clicked() {
                            // the whole source line (all its wrapped rows)
                            let rows: Vec<usize> = (0..self.rows.len()).filter(|&r| self.row_text(r, side, &geo).is_some_and(|x| x.line == t.line)).collect();
                            if let (Some(&f), Some(&l)) = (rows.first(), rows.last()) {
                                let end = self.row_text(l, side, &geo).map_or(0, |x| x.to - x.from);
                                new_sel = Some(Some(TextSel { side, a: (f, 0), b: (l, end) }));
                            }
                        } else if resp.clicked() {
                            new_sel = Some(None);
                        }
                        let has_sel = self.sel.is_some_and(|s| s.side == side && s.a != s.b);
                        resp.context_menu(|ui| {
                            if ui.add_enabled(has_sel, egui::Button::new("Copy")).clicked() {
                                copy_now = true;
                                ui.close();
                            }
                            if ui.button("Copy line").clicked() {
                                let line = &self.files[t.line.0].hunks[t.line.1].lines[t.line.2].text;
                                ui.ctx().copy_text(line.clone());
                                ui.close();
                            }
                        });
                    }
                }
                // extend while the button is held (also above / below the view)
                if self.dragging {
                    if let (Some(p), Some(s)) = (pointer, self.sel) {
                        new_sel = Some(Some(TextSel { b: pos_of(p, s.side), ..s }));
                        let clip = ui.clip_rect();
                        if p.y > clip.bottom() - 4.0 {
                            auto_scroll = row_h;
                        } else if p.y < clip.top() + 4.0 {
                            auto_scroll = -row_h;
                        }
                    }
                }
                // highlight
                if let Some(s) = new_sel.unwrap_or(self.sel) {
                    let (a, b) = s.ordered();
                    for i in visible.clone() {
                        if i < a.0 || i > b.0 {
                            continue;
                        }
                        let Some(t) = self.row_text(i, s.side, &geo) else { continue };
                        let c0 = if i == a.0 { a.1 } else { 0 };
                        let c1 = if i == b.0 { b.1 } else { t.to - t.from };
                        if c1 > c0 {
                            let r = Rect::from_min_max(pos2(t.text_x + c0 as f32 * char_w, t.rect.top()), pos2(t.text_x + c1 as f32 * char_w, t.rect.bottom()));
                            ui.painter().rect_filled(r, 0.0, theme::ACCENT.gamma_multiply(0.28));
                        }
                    }
                }
            });

        self.offset = out.state.clamped_offset().y;
        if let Some(s) = new_sel {
            self.sel = s;
        }
        if drag_start {
            self.dragging = true;
        }
        if self.dragging && !ui.input(|i| i.pointer.primary_down()) {
            self.dragging = false;
        }
        if self.dragging && auto_scroll != 0.0 {
            self.jump = Some((self.offset + auto_scroll).max(0.0));
            ui.ctx().request_repaint();
        }
        // ⌘C (or Copy from the menu) copies the selected code
        let copy_key = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
        if (copy_now || (copy_key && ui.memory(|m| m.focused().is_none()))) && self.sel.is_some() {
            let t = self.selected_text(first);
            if !t.is_empty() {
                ui.ctx().copy_text(t);
            }
        }
        if let Some(fi) = toggle {
            let o = self.is_open(fi);
            self.set_open(fi, !o);
            self.rebuild();
        }
        events
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

/// One wrapped row (`seg`) of a diff line; see `wrap_ranges`.
struct Seg<'a> {
    text: &'a str,
    seg: usize,
    cols: usize,
}

/// The characters of `text` whose columns fall in `[c0, c1)` (a tab spans 4).
fn slice_cols(text: &str, c0: usize, c1: usize) -> String {
    let mut col = 0;
    let mut out = String::new();
    for ch in text.chars() {
        let w = if ch == '\t' { 4 } else { 1 };
        if col >= c0 && col < c1 {
            out.push(ch);
        }
        col += w;
    }
    out
}

/// Wrapped rows of a line as column ranges (tabs expanded to 4 columns):
/// break after the last space that fits, or mid-word only when a single
/// word is longer than the row.
fn wrap_ranges(text: &str, cols: usize) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.replace('\t', "    ").chars().collect();
    let n = chars.len();
    let mut out = vec![];
    let mut s = 0;
    while s < n {
        let mut e = (s + cols).min(n);
        if e < n && chars[e] != ' ' {
            // last space inside the row; keep rows from getting very short
            if let Some(b) = (s + cols / 3 + 1..=e).rev().find(|&b| chars[b - 1] == ' ') {
                e = b;
            }
        }
        out.push((s, e));
        s = e;
    }
    if out.is_empty() {
        out.push((0, 0));
    }
    out
}

impl Seg<'_> {
    fn count(&self) -> usize {
        wrap_ranges(self.text, self.cols).len()
    }

    /// Sign (first segment only) at `x`, text after it, changed words highlighted.
    #[allow(clippy::too_many_arguments)]
    fn paint(&self, p: &egui::Painter, rect: Rect, x: f32, words: Option<&Vec<(usize, usize)>>, kind: LineKind, font: &FontId, fg: Color32, char_w: f32) {
        // (no +/− signs: the red / green background says it)
        let text_x = x;
        let (from, to) = wrap_ranges(self.text, self.cols).get(self.seg).copied().unwrap_or((0, 0));
        if let Some(spans) = words {
            let color = if kind == LineKind::Added { theme::ADDED_WORD_BG } else { theme::REMOVED_WORD_BG };
            // Column of a byte offset (tabs are drawn as 4 spaces).
            let col = |b: usize| self.text[..b].chars().map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>();
            for &(s, e) in spans {
                let (a, b) = (col(s).max(from), col(e).min(to));
                if a < b {
                    let r = Rect::from_min_max(
                        pos2(text_x + (a - from) as f32 * char_w, rect.top() + 1.0),
                        pos2(text_x + (b - from) as f32 * char_w, rect.bottom() - 1.0),
                    );
                    p.rect_filled(r, 2.0, color);
                }
            }
        }
        let chunk: String = self.text.replace('\t', "    ").chars().skip(from).take(to - from).collect();
        p.text(pos2(text_x, rect.center().y), Align2::LEFT_CENTER, chunk, font.clone(), fg);
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

#[cfg(test)]
mod tests {
    use super::wrap_ranges;

    #[test]
    fn wraps_between_words() {
        let t = "the login request timed out";
        let r = wrap_ranges(t, 12);
        let rows: Vec<String> = r.iter().map(|&(a, b)| t.chars().skip(a).take(b - a).collect()).collect();
        assert_eq!(rows, ["the login ", "request ", "timed out"]);
        // a word longer than the row is split
        assert_eq!(wrap_ranges("abcdefghij", 4), [(0, 4), (4, 8), (8, 10)]);
        assert_eq!(wrap_ranges("", 10), [(0, 0)]);
        // tabs count as 4 columns
        assert_eq!(wrap_ranges("\tab", 10), [(0, 6)]);
    }

    #[test]
    fn slices_by_column_keeping_tabs() {
        use super::slice_cols;
        assert_eq!(slice_cols("let x = 1;", 4, 9), "x = 1");
        assert_eq!(slice_cols("\tfoo()", 0, 7), "\tfoo");
        assert_eq!(slice_cols("\tfoo()", 4, 99), "foo()");
        assert_eq!(slice_cols("한글 텍스트", 3, 6), "텍스트");
    }
}
