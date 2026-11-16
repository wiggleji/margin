//! Virtualised diff view: all files/hunks/lines are flattened into rows and
//! only visible rows are painted, so multi-MB diffs stay smooth.
//!
//! Two layouts, switched from the bar above the diff: inline (unified) and
//! side by side (old left, new right; removed/added runs paired up).
//! Long lines wrap onto extra rows (no sideways scrolling).
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
        }

        let font = FontId::monospace(ui.style().text_styles[&egui::TextStyle::Monospace].size);
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'm'));
        let row_h = font.size + 5.0;
        let gutter = char_w * 12.0 + 8.0;
        let mut toggle = None;

        // Wrap width in columns (minus room for the scrollbar); rows depend on it.
        let avail = ui.available_width() - 18.0;
        let text_w = if self.split { avail / 2.0 - (char_w * 8.0 + 8.0) } else { avail - gutter - char_w * 2.0 };
        // (one column spare: glyph widths aren't exactly `char_w` in every font)
        let cols = ((text_w / char_w).floor() as usize).saturating_sub(1).max(10);
        if cols != self.cols {
            self.cols = cols;
            self.rebuild();
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
        let out = area.show_rows(ui, row_h, self.rows.len(), |ui, range| {
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
                            let (bg, sign, fg) = match l.kind {
                                LineKind::Added => (Some(theme::ADDED_BG), "+", theme::TEXT),
                                LineKind::Removed => (Some(theme::REMOVED_BG), "-", theme::TEXT),
                                LineKind::Context => (None, " ", theme::TEXT),
                                LineKind::NoNewline => (None, "\\", theme::TEXT_DIM),
                            };
                            if let Some(bg) = bg {
                                p.rect_filled(rect, 0.0, bg);
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
                            seg_text.paint(&p, rect, rect.left() + gutter, sign, words.get(&(fi, hi, li)), l.kind, &font, fg, char_w);
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
                                        sp.rect_filled(r, 0.0, bg);
                                    }
                                    continue;
                                }
                                let (bg, sign, fg) = match l.kind {
                                    LineKind::Added => (Some(theme::ADDED_BG), "+", theme::TEXT),
                                    LineKind::Removed => (Some(theme::REMOVED_BG), "-", theme::TEXT),
                                    LineKind::Context => (None, " ", theme::TEXT),
                                    LineKind::NoNewline => (None, "\\", theme::TEXT_DIM),
                                };
                                if let Some(bg) = bg {
                                    sp.rect_filled(r, 0.0, bg);
                                }
                                if seg == 0 {
                                    let n = if is_old { l.old_no } else { l.new_no };
                                    let num = n.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".into());
                                    sp.text(pos2(r.left() + 4.0, r.center().y), Align2::LEFT_CENTER, num, font.clone(), theme::TEXT_DIM);
                                }
                                st.paint(&sp, r, r.left() + char_w * 6.0 + 8.0, sign, words.get(&(fi, hi, li)), l.kind, &font, fg, char_w);
                            }
                            p.line_segment([pos2(rect.left() + half, rect.top()), pos2(rect.left() + half, rect.bottom())], Stroke::new(1.0, theme::BG_DARK));
                        }
                    }
                }
            });

        self.offset = out.state.clamped_offset().y;
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
    fn paint(&self, p: &egui::Painter, rect: Rect, x: f32, sign: &str, words: Option<&Vec<(usize, usize)>>, kind: LineKind, font: &FontId, fg: Color32, char_w: f32) {
        let text_x = x + char_w * 2.0;
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
        if self.seg == 0 {
            p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, sign, font.clone(), fg);
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
}
