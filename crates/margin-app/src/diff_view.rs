//! Virtualised diff view: all files/hunks/lines are flattened into rows and
//! only visible rows are painted, so multi-MB diffs stay smooth.
//!
//! Two layouts, switched from the bar above the diff: inline (unified) and
//! side by side (old left, new right; removed/added runs paired up).
//! Long lines wrap onto extra rows (no sideways scrolling).
//! Layout of the scrolled page: an optional header (commit details), then
//! the toolbar, then the rows. The toolbar sticks to the top once the header
//! has scrolled away (showing the commit title), so nothing ever resizes.
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
    /// Focus mode is on (the diff has the whole window); not saved.
    pub focused: bool,
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

/// Id of the card for a new comment (not on GitHub yet).
const DRAFT: &str = "draft";

/// A PR review thread to show inside the diff, on `line` of `path`.
#[derive(Clone, PartialEq, Debug)]
pub struct InlineThread {
    /// GraphQL node id, and the branch whose PR it's on (for reply / resolve).
    pub id: String,
    pub branch: String,
    pub path: String,
    pub line: u64,
    pub old_side: bool,
    pub resolved: bool,
    pub outdated: bool,
    pub pr: u64,
    /// (author, body, date, url), oldest first.
    pub comments: Vec<(String, String, String, String)>,
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
    OpenUrl(String),
    /// Reply to a review thread / resolve or reopen it.
    ThreadReply { id: String, branch: String, body: String },
    ThreadResolve { id: String, branch: String, resolve: bool },
    /// Start a new review thread on a line of this commit.
    ThreadNew { branch: String, path: String, line: u64, old_side: bool, body: String },
    /// The focus button: give the diff the whole window, or bring the rest back.
    ToggleFocus,
}

#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Hunk(usize, usize),
    /// "UNSTAGED (3)" / "STAGED (1)" header with stage-all buttons.
    Section(Section),
    /// Space for a PR review thread under its line (the card is painted over it).
    Thread,
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
    /// Height of the header above the toolbar (measured last frame).
    header_h: f32,
    /// ↑/↓ clicked in the toolbar, applied next frame.
    pending_nav: i32,
    /// PR review threads shown under their lines (commit diffs).
    threads: Vec<InlineThread>,
    /// Rows each thread takes (measured at `thread_w`), which are open, and
    /// where they sit: (thread, first row, rows).
    thread_h: Vec<usize>,
    thread_open: Vec<bool>,
    thread_w: f32,
    thread_at: Vec<(usize, usize, usize)>,
    /// Thread being replied to (id) and the draft; threads with a request in flight.
    reply: Option<String>,
    reply_text: String,
    reply_focus: bool,
    thread_busy: std::collections::HashSet<String>,
    /// Where new review comments go: (branch, PR number). None: no "+" on lines.
    comment_target: Option<(String, u64)>,
    /// Scroll smoothly to this offset (next / previous change).
    glide_to: Option<f32>,
    /// Row of the change last jumped to, and the viewport height.
    nav_row: Option<usize>,
    view_h: f32,
}

/// Content scrolled above the diff (commit details), and the short title
/// shown in the toolbar once it has scrolled away.
pub struct Header<'a> {
    pub ui: &'a mut dyn FnMut(&mut Ui),
    pub title: String,
}

/// What the toolbar was used for this frame.
#[derive(Default)]
struct BarOut {
    all: Option<bool>,
    nav: i32,
    ws: Option<bool>,
    split: Option<bool>,
    focus: bool,
}

/// The toolbar above the rows: counts, collapse/expand all, ↑↓, whitespace,
/// layout; `title` (faded in by `title_a`) once the header is out of view.
#[allow(clippy::too_many_arguments)]
fn diff_bar(ui: &mut Ui, n: usize, added: usize, removed: usize, collapsed_all: bool, prefs: &DiffPrefs, title: Option<&str>, title_a: f32, loading: bool, focused: bool) -> BarOut {
    let mut out = BarOut::default();
    // Stuck under a scrolled-away header: the commit title takes the place of
    // the counts (they're in the header); it fades in.
    let stuck = title.is_some() && title_a > 0.01;
    // A narrow column gets shorter labels instead of pushing past its edge.
    let narrow = ui.available_width() < 420.0 * (ui.style().text_styles[&egui::TextStyle::Body].size / 13.0).max(1.0);
    ui.horizontal_centered(|ui| {
        if !stuck {
            let files = if narrow { format!("{n}") } else { format!("{n} file{}", if n == 1 { "" } else { "s" }) };
            ui.label(egui::RichText::new(files).small().color(theme::TEXT_DIM())).on_hover_text(format!("{n} changed file{}", if n == 1 { "" } else { "s" }));
            // only the sides that changed
            if added > 0 {
                ui.label(egui::RichText::new(format!("+{added}")).small().color(theme::ADDED()));
            }
            if removed > 0 {
                ui.label(egui::RichText::new(format!("−{removed}")).small().color(theme::REMOVED()));
            }
        }
        let fold_tip = if collapsed_all { "Expand all files" } else { "Collapse all files" };
        if n > 1 && !stuck && crate::widgets::icon_button(ui, crate::widgets::Glyph::Fold { fold: !collapsed_all }, fold_tip).clicked() {
            out.all = Some(!collapsed_all);
        }
        if loading {
            ui.add(egui::Spinner::new().size(12.0));
        }
        // controls on the right; the title fills the space between (no shifting)
        let right = ui.available_width();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            use crate::widgets::{Glyph, Seg, icon_button, segmented, toggle_chip};
            let key = if cfg!(target_os = "macos") { "⌘⇧↩" } else { "Ctrl+Shift+Enter" };
            let tip = if focused { format!("Show the branches and history again ({key})") } else { format!("Focus on the diff: hide the branches and history ({key})") };
            if icon_button(ui, Glyph::Focus { on: focused }, &tip).clicked() {
                out.focus = true;
            }
            ui.add_space(2.0);
            // (right-to-left: the segments are laid out left to right inside)
            let layouts = [Seg::glyph(Glyph::Inline).tip("Inline"), Seg::glyph(Glyph::SideBySide).tip("Side by side")];
            if let Some(i) = segmented(ui, &layouts, Some(prefs.split as usize)) {
                out.split = Some(i == 1);
            }
            ui.add_space(4.0);
            if toggle_chip(ui, prefs.ignore_ws, if narrow { "Ignore ws" } else { "Ignore whitespace" })
                .on_hover_text("Hide changes that only touch spaces, tabs or indentation (git diff -w)")
                .clicked()
            {
                out.ws = Some(!prefs.ignore_ws);
            }
            ui.add_space(4.0);
            let alt = if cfg!(target_os = "macos") { "⌥" } else { "Alt+" };
            if icon_button(ui, Glyph::ArrowDown, &format!("Next change ({alt}↓)")).clicked() {
                out.nav = 1;
            }
            if icon_button(ui, Glyph::ArrowUp, &format!("Previous change ({alt}↑)")).clicked() {
                out.nav = -1;
            }
            if let (Some(t), true) = (title, stuck) {
                let w = (ui.available_width() - 8.0).max(0.0);
                if w > 40.0 && right > 0.0 {
                    ui.add_space(8.0);
                    // left-aligned in the space left of the controls
                    ui.allocate_ui_with_layout(egui::vec2(w, ui.available_height()), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(t).strong().color(theme::TEXT().gamma_multiply(title_a))).truncate());
                    });
                }
            }
        });
    });
    out
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
        self.threads.clear();
        self.thread_at.clear();
        self.reply = None;
        self.reply_text.clear();
        self.sel = None;
        self.dragging = false;
        self.nav_row = None;
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

    /// PR review threads for this diff (commit view). Rebuilds when they change.
    pub fn set_threads(&mut self, mut t: Vec<InlineThread>) {
        // a new comment being written stays
        if let Some(d) = self.threads.iter().find(|x| x.id == DRAFT) {
            t.push(d.clone());
        }
        if t == self.threads {
            return;
        }
        // open by default, unless resolved or outdated (those fold to one line);
        // a thread already shown keeps its state unless it was just resolved / reopened
        let open = |n: &InlineThread| {
            let old = self.threads.iter().position(|o| o.id == n.id && !n.id.is_empty() && o.resolved == n.resolved);
            old.map_or(!(n.resolved || n.outdated), |i| self.thread_open[i])
        };
        self.thread_open = t.iter().map(open).collect();
        if self.reply.as_ref().is_some_and(|id| !t.iter().any(|n| &n.id == id)) {
            self.reply = None;
        }
        self.threads = t;
        self.thread_w = 0.0; // re-measure
        self.thread_h.clear();
        self.rebuild();
    }

    /// A reply / resolve / new comment for thread `id` finished (the PR
    /// reloads after a success, bringing a new thread in).
    pub fn thread_done(&mut self, id: &str, ok: bool) {
        self.thread_busy.remove(id);
        if ok && self.reply.as_deref() == Some(id) {
            self.reply = None;
            self.reply_text.clear();
        }
        if ok && id == DRAFT {
            self.drop_draft();
        }
        self.thread_w = 0.0; // re-measure
    }

    /// Lines can get new review comments, posted to this branch's PR.
    pub fn set_comment_target(&mut self, target: Option<(String, u64)>) {
        if target.is_none() {
            self.drop_draft();
        }
        self.comment_target = target;
    }

    /// Open a new-comment card under a line.
    fn open_draft(&mut self, path: String, line: u64, old_side: bool) {
        let Some((branch, pr)) = self.comment_target.clone() else { return };
        self.drop_draft();
        self.threads.push(InlineThread { id: DRAFT.into(), branch, path, line, old_side, resolved: false, outdated: false, pr, comments: vec![] });
        self.thread_open.push(true);
        self.reply = Some(DRAFT.into());
        self.reply_text.clear();
        self.reply_focus = true;
        self.thread_h.clear(); // re-measure
        self.rebuild();
    }

    fn drop_draft(&mut self) {
        if let Some(i) = self.threads.iter().position(|t| t.id == DRAFT) {
            self.threads.remove(i);
            self.thread_open.remove(i);
            self.thread_h.clear();
            if self.reply.as_deref() == Some(DRAFT) {
                self.reply = None;
                self.reply_text.clear();
            }
            self.rebuild();
        }
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
        self.place_threads();
    }

    /// Insert each review thread's rows right after the (last wrapped row of
    /// the) line it's on.
    fn place_threads(&mut self) {
        self.thread_at.clear();
        if self.threads.is_empty() {
            return;
        }
        let rows = std::mem::take(&mut self.rows);
        let mut placed = vec![false; self.threads.len()];
        // (file, old line no, new line no) of the source line a row shows
        let line_of = |r: &Row| -> Option<(usize, Option<u32>, Option<u32>)> {
            let ln = |fi: usize, hi: usize, li: usize| &self.files[fi].hunks[hi].lines[li];
            match *r {
                Row::Line(fi, hi, li, _) => {
                    let l = ln(fi, hi, li);
                    Some((fi, l.old_no.filter(|_| l.kind != LineKind::Added), l.new_no.filter(|_| l.kind != LineKind::Removed)))
                }
                Row::Pair(fi, hi, o, n, _) => Some((fi, o.and_then(|o| ln(fi, hi, o).old_no), n.and_then(|n| ln(fi, hi, n).new_no))),
                _ => None,
            }
        };
        let same_line = |a: &Row, b: &Row| match (a, b) {
            (Row::Line(f, h, l, _), Row::Line(f2, h2, l2, _)) => (f, h, l) == (f2, h2, l2),
            (Row::Pair(f, h, o, n, _), Row::Pair(f2, h2, o2, n2, _)) => (f, h, o, n) == (f2, h2, o2, n2),
            _ => false,
        };
        let mut out = Vec::with_capacity(rows.len() + 16);
        for (i, r) in rows.iter().enumerate() {
            out.push(*r);
            if rows.get(i + 1).is_some_and(|n| same_line(r, n)) {
                continue; // not the last wrapped row of this line yet
            }
            let Some((fi, old, new)) = line_of(r) else { continue };
            let path = self.files[fi].display_path();
            for (ti, t) in self.threads.iter().enumerate() {
                if placed[ti] || t.path != path {
                    continue;
                }
                let at = if t.old_side { old } else { new };
                if at.map(u64::from) == Some(t.line) {
                    placed[ti] = true;
                    let n = self.thread_h.get(ti).copied().unwrap_or(1).max(1);
                    self.thread_at.push((ti, out.len(), n));
                    out.extend((0..n).map(|_| Row::Thread));
                }
            }
        }
        self.rows = out;
    }

    /// `prefs` are changed from the bar above the diff (layout, whitespace);
    /// the caller reloads the diff when `ignore_ws` changes.
    pub fn show(&mut self, ui: &mut Ui, mode: DiffMode, id: &str, prefs: &mut DiffPrefs, header: Option<Header<'_>>) -> Vec<DiffEvent> {
        let mut events = vec![];
        let mut header = header;
        // Nothing to scroll: header (if any), then the message.
        if self.files.is_empty() || self.error.is_some() {
            if let Some(h) = header.as_mut() {
                (h.ui)(ui);
            }
        }
        if self.loading && self.files.is_empty() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.weak("Loading diff…");
            });
            return events;
        }
        if let Some(e) = &self.error {
            ui.colored_label(theme::ERROR(), e);
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
        if self.truncated {
            ui.colored_label(theme::ACCENT(), "Diff truncated (too large). Use the terminal for the full patch.");
        }

        let mut nav: i32 = std::mem::take(&mut self.pending_nav);
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
        if self.split != *split {
            self.split = *split;
            self.rebuild();
            self.sel = None;
        }

        let font = FontId::monospace(ui.style().text_styles[&egui::TextStyle::Monospace].size);
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'm'));
        let row_h = font.size + 5.0;
        // Page layout (content coordinates): header, toolbar slot, rows.
        let bar_h = ui.spacing().interact_size.y + 8.0;
        let head_h = if header.is_some() { self.header_h } else { 0.0 };
        let rows_y = head_h + bar_h;
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
        // Review thread cards: measure their height at this width (rows reserved under the line).
        let card_left = if self.split { 8.0 } else { gutter };
        let card_w = (avail - card_left - 6.0).max(220.0);
        if !self.threads.is_empty() && ((card_w - self.thread_w).abs() > 0.5 || self.thread_h.len() != self.threads.len()) {
            self.thread_w = card_w;
            self.thread_h = self
                .threads
                .iter()
                .enumerate()
                .map(|(i, t)| ((layout_thread(ui, t, card_w, self.thread_open[i], self.thread_state(t)).h + 6.0) / row_h).ceil() as usize)
                .collect();
            self.rebuild();
        }
        let mut thread_toggle: Option<usize> = None;
        let mut thread_act: Option<(usize, CardAct)> = None;
        let mut new_comment: Option<(String, u64, bool)> = None;
        let mut reply_text = std::mem::take(&mut self.reply_text);
        let reply_focus = std::mem::take(&mut self.reply_focus);

        // Next / previous change = the next hunk header below / above the top of the view.
        // (a hunk lands just under the sticky toolbar)
        if nav != 0 {
            let ys = self.rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Hunk(..))).map(|(i, _)| (i, rows_y + i as f32 * row_h - bar_h));
            // The change we're on: the last one jumped to, while it's still in
            // view (the page may not scroll far enough to bring it to the top).
            let on = self.nav_row.filter(|&r| {
                let y = rows_y + r as f32 * row_h;
                y >= self.offset - 1.0 && y <= self.offset + self.view_h
            });
            let target = match (nav > 0, on) {
                (true, Some(r)) => ys.clone().find(|&(i, _)| i > r),
                (false, Some(r)) => ys.filter(|&(i, _)| i < r).next_back(),
                (true, None) => ys.clone().find(|&(_, y)| y > self.offset + 1.0),
                (false, None) => ys.filter(|&(_, y)| y < self.offset - 1.0).next_back(),
            };
            if let Some((i, y)) = target {
                self.nav_row = Some(i);
                self.glide_to = Some(y.max(0.0));
                self.flash = Some((i, ui.input(|i| i.time)));
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
        // toolbar inputs (filled in while drawing the sticky toolbar)
        let n_files = self.files.len();
        let (added, removed) = self.files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        let collapsed_all = (0..n_files).all(|fi| !open[fi]);
        let stuck_id = ui.scope_id().with(id).with("stuck");
        let mut bar_out = BarOut::default();
        let mut measured_head = None;
        let loading = self.loading;
        let n_rows = self.rows.len();
        let prefs_now = *prefs;
        let mut glide = self.glide_to.take();
        let out = area.show_viewport(ui, |page, vp| {
                let top = page.max_rect().top();
                let x0 = page.max_rect().left();
                let page_w = page.available_width();
                page.set_min_height(rows_y + n_rows as f32 * row_h + 4.0);
                if let Some(y) = glide.take() {
                    page.scroll_with_delta_animation(vec2(0.0, vp.min.y - y), egui::style::ScrollAnimation::duration(0.18));
                }
                // ---- header (scrolls away with the page)
                if let Some(h) = header.as_mut() {
                    if vp.min.y < head_h + 1.0 || head_h == 0.0 {
                        let r = page.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(x0, top), vec2(page_w, f32::INFINITY))), |ui| (h.ui)(ui)).response.rect;
                        measured_head = Some(r.height());
                    }
                }
                // ---- rows in view
                let first_row = (((vp.min.y - rows_y) / row_h).floor().max(0.0) as usize).min(n_rows);
                let last_row = ((((vp.max.y - rows_y) / row_h).ceil().max(0.0) as usize) + 1).min(n_rows);
                let range = first_row..last_row;
                let visible = range.clone();
                let rows_rect = Rect::from_min_size(pos2(x0, top + rows_y + first_row as f32 * row_h), vec2(page_w, (last_row - first_row) as f32 * row_h + 1.0));
                let mut rows_ui = page.new_child(egui::UiBuilder::new().max_rect(rows_rect).layout(egui::Layout::top_down(egui::Align::Min)));
                let ui = &mut rows_ui;
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
                        let clicked = row_button(ui, r, base_id.with(("row-btn", i, text)), text, &small).clicked();
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
                            let g = p.layout_no_wrap(format!("{title} ({n})"), small.clone(), theme::TEXT_DIM());
                            let gw = g.size().x;
                            p.galley(pos2(rect.left() + 6.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT_DIM());
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
                            p.rect_filled(rect, 0.0, theme::BG_DARK());
                            let open = open[fi];
                            tri(&p, pos2(rect.left() + 10.0, rect.center().y), open, theme::TEXT_DIM());
                            // An icon for what happened to the file (named on hover),
                            // except the usual case, modified, which needs none.
                            use crate::icons::Icon;
                            let status = match (&f.old_path, &f.new_path) {
                                _ if sec == Section::Conflicted => Some((Icon::Alert, "Conflict", theme::ERROR())),
                                (None, Some(_)) => Some((Icon::DiffAdded, "Added", theme::ADDED())),
                                (Some(_), None) => Some((Icon::DiffRemoved, "Deleted", theme::REMOVED())),
                                (Some(o), Some(n)) if o != n => Some((Icon::DiffRenamed, "Renamed", theme::lane(0))),
                                _ => None,
                            };
                            let mut x = rect.left() + 22.0;
                            if let Some((icon, word, color)) = status {
                                let sz = (row_h - 6.0).clamp(10.0, 16.0);
                                let ir = Rect::from_min_size(pos2(x, rect.center().y - sz / 2.0), vec2(sz, sz));
                                crate::icons::paint(&p, icon, ir, color);
                                ui.interact(ir.expand(2.0), base_id.with(("file-status", i)), Sense::hover()).on_hover_text(word);
                                x = ir.right() + 8.0;
                            }
                            // Room for the path: up to the line counts and the buttons
                            // on the right; longer paths are shortened in the middle
                            // (`src/…/login.rs`, `too…long.png`), full on hover.
                            let row_buttons: &[&str] = match sec {
                                Section::Staged => &["Unstage"],
                                Section::Conflicted => &["Stage"],
                                Section::Unstaged => &["Discard", "Stage"],
                                _ => &[],
                            };
                            let btn_pad = ui.spacing().button_padding.x * 2.0 + 6.0;
                            let buttons_w: f32 = row_buttons.iter().map(|l| ui.fonts_mut(|f| f.layout_no_wrap(l.to_string(), small.clone(), Color32::WHITE).size().x) + btn_pad + 6.0).sum();
                            let count_cells: usize = [(f.added, 1), (f.removed, 1)].iter().filter(|(n, _)| *n > 0).map(|(n, _)| n.to_string().len() + 2).sum();
                            let avail = rect.right() - 6.0 - buttons_w - count_cells as f32 * char_w - 18.0 - x;
                            let max_cells = (avail / char_w).floor().max(6.0) as usize;
                            let full = match (&f.old_path, &f.new_path) {
                                (Some(o), Some(n)) if o != n => format!("{o} → {n}"),
                                _ => f.display_path().to_string(),
                            };
                            let elided = crate::path_elide::width(&full) + 2 > max_cells;
                            let fit = |p: &str, m: usize| crate::path_elide::elide_path(p, m);
                            // path: the folder dim, the file name bright (renames: old → new)
                            let mut job = egui::text::LayoutJob::default();
                            let path_part = |job: &mut egui::text::LayoutJob, path: &str, strike: bool| {
                                let (dir, name) = path.rsplit_once('/').map_or(("", path), |(d, n)| (d, n));
                                let fmt = |c: Color32| egui::TextFormat {
                                    font_id: font.clone(),
                                    color: c,
                                    strikethrough: if strike { Stroke::new(1.0, c) } else { Stroke::NONE },
                                    ..Default::default()
                                };
                                if !dir.is_empty() {
                                    job.append(&format!("{dir}/"), 0.0, fmt(theme::TEXT_DIM()));
                                }
                                job.append(name, 0.0, fmt(if strike { theme::TEXT_DIM() } else { theme::TEXT() }));
                            };
                            match (&f.old_path, &f.new_path) {
                                (Some(o), Some(n)) if o != n => {
                                    let half = max_cells.saturating_sub(5) / 2;
                                    path_part(&mut job, &fit(o, half), false);
                                    job.append("  →  ", 0.0, egui::TextFormat { font_id: font.clone(), color: theme::TEXT_DIM(), ..Default::default() });
                                    path_part(&mut job, &fit(n, half), false);
                                }
                                (Some(o), None) => path_part(&mut job, &fit(o, max_cells), true),
                                _ => path_part(&mut job, &fit(f.display_path(), max_cells), false),
                            }
                            let g = p.layout_job(job);
                            let w = g.size().x;
                            p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, theme::TEXT());
                            // line counts, only the sides that changed
                            let mut x = x + w + 14.0;
                            for (n, sign, color) in [(f.added, "+", theme::ADDED()), (f.removed, "−", theme::REMOVED())] {
                                if n > 0 {
                                    let g = p.layout_no_wrap(format!("{sign}{n}"), font.clone(), color);
                                    let gw = g.size().x;
                                    p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
                                    x += gw + char_w;
                                }
                            }
                            // Working directory: per-file buttons on the right, and a menu.
                            let path = f.display_path().to_string();
                            if matches!(sec, Section::Unstaged | Section::Staged | Section::Conflicted) {
                                let labels = row_buttons;
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
                            let resp = if elided { resp.on_hover_text(&full) } else { resp };
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
                        Row::Thread => {} // the card is painted over these rows below
                        Row::Note(fi) => {
                            let f = &self.files[fi];
                            let msg = if self.sections.get(fi) == Some(&Section::Conflicted) {
                                "Merge conflict: resolve it in your editor, then stage the file."
                            } else if f.binary {
                                "Binary file"
                            } else {
                                "No content changes (mode / rename only)"
                            };
                            p.text(pos2(rect.left() + gutter, rect.center().y), Align2::LEFT_CENTER, msg, font.clone(), theme::TEXT_DIM());
                        }
                        Row::Hunk(fi, hi) => {
                            p.rect_filled(rect, 0.0, theme::HUNK_BG());
                            if let Some((_, a)) = flash.filter(|&(r, _)| r == i) {
                                p.rect_filled(rect, 0.0, theme::ACCENT().gamma_multiply(0.35 * a));
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
                                let btn = row_button(ui, r, base_id.with(("hunk-btn", i, *text)), text, &small);
                                if btn.clicked() {
                                    events.push(DiffEvent::Hunk(DiffAction { file: fi, hunk: hi, op: *op }));
                                }
                                x += bw + 6.0;
                            }
                            // "Lines 78–81  fn login()" instead of "@@ -78,3 +78,4 @@ fn login()"
                            let (range, ctx) = hunk_label(&h.header);
                            let g = p.layout_no_wrap(range, small.clone(), theme::TEXT_DIM());
                            let gw = g.size().x;
                            p.galley(pos2(x + 4.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT_DIM());
                            if !ctx.is_empty() {
                                p.text(pos2(x + 4.0 + gw + 12.0, rect.center().y), Align2::LEFT_CENTER, ctx, font.clone(), theme::TEXT_DIM().gamma_multiply(0.8));
                            }
                        }
                        Row::Line(fi, hi, li, seg) => {
                            let l = &self.files[fi].hunks[hi].lines[li];
                            let (bg, fg) = match l.kind {
                                LineKind::Added => (Some(theme::ADDED_BG()), theme::TEXT()),
                                LineKind::Removed => (Some(theme::REMOVED_BG()), theme::TEXT()),
                                LineKind::Context => (None, theme::TEXT()),
                                LineKind::NoNewline => (None, theme::TEXT_DIM()),
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
                                    theme::TEXT_DIM(),
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
                                    sp.rect_filled(r, 0.0, theme::BG_DARK());
                                    continue;
                                };
                                let l = &lines[li];
                                let st = Seg { text: &l.text, seg, cols: self.cols };
                                if seg >= st.count() {
                                    // the other side wrapped further: keep the line's background
                                    if let Some(bg) = match l.kind { LineKind::Added => Some(theme::ADDED_BG()), LineKind::Removed => Some(theme::REMOVED_BG()), _ => None } {
                                        sp.rect_filled(Rect::from_min_max(pos2(r.left() + char_w * 6.0 + 4.0, r.top()), r.max), 0.0, bg);
                                    }
                                    continue;
                                }
                                let (bg, fg) = match l.kind {
                                    LineKind::Added => (Some(theme::ADDED_BG()), theme::TEXT()),
                                    LineKind::Removed => (Some(theme::REMOVED_BG()), theme::TEXT()),
                                    LineKind::Context => (None, theme::TEXT()),
                                    LineKind::NoNewline => (None, theme::TEXT_DIM()),
                                };
                                if let Some(bg) = bg {
                                    sp.rect_filled(Rect::from_min_max(pos2(r.left() + char_w * 6.0 + 4.0, r.top()), r.max), 0.0, bg);
                                }
                                if seg == 0 {
                                    let n = if is_old { l.old_no } else { l.new_no };
                                    let num = n.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".into());
                                    sp.text(pos2(r.left() + 4.0, r.center().y), Align2::LEFT_CENTER, num, font.clone(), theme::TEXT_DIM());
                                }
                                st.paint(&sp, r, r.left() + char_w * 6.0 + 8.0, words.get(&(fi, hi, li)), l.kind, &font, fg, char_w);
                            }
                            p.line_segment([pos2(rect.left() + half, rect.top()), pos2(rect.left() + half, rect.bottom())], Stroke::new(1.0, theme::BG_DARK()));
                        }
                    }
                }

                // ---- text selection: drag over code text (not numbers / signs / headers)
                if let Some(geo) = first {
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
                            ui.painter().rect_filled(r, 0.0, theme::ACCENT().gamma_multiply(0.28));
                        }
                    }
                }
                }
                // ---- "+" on the hovered line's number: start a new review comment
                if let (Some(geo), true, Some(hp)) = (first.as_ref(), self.comment_target.is_some() && !self.dragging, page.input(|i| i.pointer.hover_pos())) {
                    let ri = ((hp.y - geo.top0) / geo.row_h).floor();
                    let inside = ri >= 0.0 && (ri as usize) < self.rows.len() && hp.x >= geo.x0 && hp.x <= geo.x0 + geo.width && page.clip_rect().contains(hp);
                    let ri = ri.max(0.0) as usize;
                    let ln = |fi: usize, hi: usize, li: usize| &self.files[fi].hunks[hi].lines[li];
                    // (file, line number, old side, x where the code starts)
                    let target = if !inside {
                        None
                    } else {
                        match self.rows[ri] {
                            Row::Line(fi, hi, li, _) => {
                                let l = ln(fi, hi, li);
                                let old = l.kind == LineKind::Removed;
                                let n = if old { l.old_no } else { l.new_no };
                                n.filter(|_| l.kind != LineKind::NoNewline).map(|n| (fi, n, old, geo.x0 + geo.gutter - 4.0))
                            }
                            Row::Pair(fi, hi, o, nw, _) => {
                                let right = hp.x > geo.x0 + geo.width / 2.0;
                                let side_x = if right { geo.x0 + geo.width / 2.0 } else { geo.x0 };
                                (if right { nw } else { o }).and_then(|li| {
                                    let l = ln(fi, hi, li);
                                    let n = if right { l.new_no } else { l.old_no };
                                    n.filter(|_| l.kind != LineKind::NoNewline).map(|n| (fi, n, !right, side_x + geo.char_w * 6.0 + 4.0))
                                })
                            }
                            _ => None,
                        }
                    };
                    if let Some((fi, n, old, code_x)) = target {
                        let sz = (geo.row_h - 2.0).min(18.0);
                        let y = geo.top0 + ri as f32 * geo.row_h + (geo.row_h - sz) / 2.0;
                        let r = Rect::from_min_size(pos2(code_x - sz - 3.0, y), vec2(sz, sz));
                        let resp = page.interact(r, base_id.with("new-comment"), Sense::click());
                        let fill = if resp.hovered() { theme::ACCENT() } else { theme::ACCENT().gamma_multiply(0.85) };
                        page.painter().rect_filled(r, 4.0, fill);
                        let fg = if theme::cur().dark { theme::BG_DARK() } else { Color32::WHITE };
                        let (c, k) = (r.center(), sz * 0.28);
                        page.painter().line_segment([pos2(c.x - k, c.y), pos2(c.x + k, c.y)], Stroke::new(1.6, fg));
                        page.painter().line_segment([pos2(c.x, c.y - k), pos2(c.x, c.y + k)], Stroke::new(1.6, fg));
                        if resp.hovered() {
                            page.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.on_hover_text("Comment on this line").clicked() {
                            new_comment = Some((self.files[fi].display_path().to_string(), u64::from(n), old));
                        }
                    }
                }
                // ---- review thread cards, over the rows reserved for them
                for &(ti, first_r, n) in &self.thread_at {
                    if first_r + n < first_row || first_r > last_row {
                        continue;
                    }
                    let t = &self.threads[ti];
                    let open_t = self.thread_open[ti];
                    let state = self.thread_state(t);
                    let card = layout_thread(page, t, card_w, open_t, state);
                    let y0 = top + rows_y + first_r as f32 * row_h + 3.0;
                    let rect = Rect::from_min_size(pos2(x0 + card_left, y0), vec2(card_w, n as f32 * row_h - 6.0));
                    let p = page.painter();
                    let muted = t.resolved || t.outdated;
                    p.rect_filled(rect, 6.0, theme::BG_DARK());
                    p.rect_stroke(rect, 6.0, Stroke::new(1.0, theme::HOVER()), egui::StrokeKind::Inside);
                    let bar = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
                    p.rect_filled(bar, egui::CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, if muted { theme::TEXT_DIM() } else { theme::ACCENT() });
                    // GitHub mark + PR number, top right
                    let isz = 14.0;
                    let pr_g = p.layout_no_wrap(format!("#{}", t.pr), egui::TextStyle::Small.resolve(page.style()), theme::TEXT_DIM());
                    let pr_x = rect.right() - 10.0 - pr_g.size().x;
                    crate::icons::paint(p, crate::icons::Icon::Github, Rect::from_min_size(pos2(pr_x - isz - 4.0, rect.top() + 9.0), vec2(isz, isz)), theme::TEXT_DIM());
                    p.galley(pos2(pr_x, rect.top() + 9.0), pr_g, theme::TEXT_DIM());
                    let clip = p.with_clip_rect(rect.shrink2(vec2(0.0, 1.0)).intersect(page.clip_rect()));
                    for (x, y, g, color) in &card.items {
                        clip.galley(pos2(rect.left() + x, rect.top() + y), g.clone(), *color);
                    }
                    // click the header line to fold / unfold
                    let head = Rect::from_min_size(rect.min, vec2(rect.width() - 80.0, card.header_h));
                    let hr = page.interact(head, base_id.with(("thread-head", ti)), Sense::click());
                    if t.id == DRAFT {
                        // (a new comment has no thread to fold yet)
                    } else if hr.hovered() {
                        page.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if t.id != DRAFT && hr.on_hover_text(if open_t { "Fold this thread" } else { "Show this thread" }).clicked() {
                        thread_toggle = Some(ti);
                    }
                    // footer links: Reply · Resolve · Open on GitHub
                    for (k, (lx, ly, lw, lh)) in &card.acts {
                        let lr = Rect::from_min_size(pos2(rect.left() + lx, rect.top() + ly), vec2(*lw, *lh)).expand2(vec2(3.0, 2.0));
                        let resp = page.interact(lr, base_id.with(("thread-act", ti, *k as u8)), Sense::click());
                        if resp.hovered() {
                            page.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            page.painter().rect_filled(lr, 3.0, theme::HOVER().gamma_multiply(0.6));
                            // (repaint the label over the hover fill)
                            if let Some((_, _, g, c)) = card.items.iter().find(|(x, y, ..)| x == lx && y == ly) {
                                page.painter().galley(pos2(rect.left() + lx, rect.top() + ly), g.clone(), *c);
                            }
                        }
                        let tip = match k {
                            CardAct::Link => t.comments.first().map_or(String::new(), |c| c.3.clone()),
                            CardAct::Reply => "Reply to this thread".into(),
                            CardAct::Resolve if t.resolved => "Reopen this conversation".into(),
                            CardAct::Resolve => "Mark this conversation as resolved".into(),
                            _ => String::new(),
                        };
                        if resp.on_hover_text(tip).clicked() {
                            thread_act = Some((ti, *k));
                        }
                    }
                    // reply box: a text field and Comment / Cancel
                    if let Some((ex, ey, ew, eh)) = card.editor {
                        let er = Rect::from_min_size(pos2(rect.left() + ex, rect.top() + ey), vec2(ew, eh));
                        let busy = state.busy;
                        let te_id = base_id.with(("thread-reply", ti));
                        // ":ta…" → emoji dropdown (takes ↑↓ ↵ Esc while open)
                        let emoji_open = crate::emoji::keys(page, te_id, &reply_text);
                        let te = egui::TextEdit::multiline(&mut reply_text)
                            .id(te_id)
                            .hint_text(if t.id == DRAFT { "Leave a comment…  (⌘↩ sends, : for emoji)" } else { "Reply…  (⌘↩ sends, : for emoji)" })
                            .font(egui::TextStyle::Body)
                            .desired_width(ew)
                            .margin(vec2(8.0, 6.0))
                            .background_color(page.visuals().panel_fill)
                            .interactive(!busy);
                        let r = page.put(er, te);
                        crate::emoji::dropdown(page, te_id, er, &mut reply_text, emoji_open);
                        if reply_focus {
                            r.request_focus();
                        }
                        let send = r.has_focus() && page.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
                        // (Esc takes the focus away before we look, so check both)
                        let cancel = (r.has_focus() || r.lost_focus()) && page.input(|i| i.key_pressed(egui::Key::Escape));
                        let br = Rect::from_min_size(pos2(er.left(), er.bottom() + 8.0), vec2(ew, 26.0));
                        let mut bui = page.new_child(egui::UiBuilder::new().max_rect(br).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        let can = !busy && !reply_text.trim().is_empty();
                        let comment = bui.add_enabled_ui(can, |ui| crate::widgets::primary_button(ui, if busy { "Sending…" } else { "Comment" })).inner.clicked();
                        let cancel_b = bui.add_enabled(!busy, egui::Button::new("Cancel")).clicked();
                        if (comment || send) && can {
                            thread_act = Some((ti, CardAct::Send));
                        } else if cancel || cancel_b {
                            thread_act = Some((ti, CardAct::Cancel));
                        }
                    }
                }
                // ---- toolbar: in its slot under the header, then stuck to the top
                let stuck = header.is_some() && vp.min.y > head_h + 0.5;
                let title_a = page.ctx().animate_bool_with_time(stuck_id, stuck, 0.15);
                let bar_y = head_h.max(vp.min.y);
                let bar_rect = Rect::from_min_size(pos2(x0, top + bar_y), vec2(page_w, bar_h));
                page.painter().rect_filled(bar_rect, 0.0, page.visuals().panel_fill);
                if title_a > 0.0 {
                    // soft edge under the stuck toolbar
                    page.painter().line_segment([bar_rect.left_bottom(), bar_rect.right_bottom()], Stroke::new(1.0, theme::BG_DARK().gamma_multiply(title_a)));
                }
                let mut bar_ui = page.new_child(egui::UiBuilder::new().max_rect(bar_rect.shrink2(vec2(0.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                let title = header.as_ref().map(|h| h.title.as_str());
                bar_out = diff_bar(&mut bar_ui, n_files, added, removed, collapsed_all, &prefs_now, title, title_a, loading, prefs_now.focused);
            });
        if let Some(h) = measured_head {
            if (h - self.header_h).abs() > 0.5 {
                self.header_h = h;
                ui.ctx().request_repaint();
            }
        }
        self.reply_text = reply_text;
        if let Some((path, line, old)) = new_comment {
            self.open_draft(path, line, old);
            ui.ctx().request_repaint();
        }
        if let Some((ti, act)) = thread_act {
            let t = &self.threads[ti];
            match act {
                CardAct::Link => {
                    if let Some(c) = t.comments.first() {
                        events.push(DiffEvent::OpenUrl(c.3.clone()));
                    }
                }
                CardAct::Reply => {
                    if self.reply.as_ref() != Some(&t.id) {
                        self.reply = Some(t.id.clone());
                        self.reply_text.clear();
                    }
                    self.reply_focus = true;
                }
                CardAct::Cancel if t.id == DRAFT => self.drop_draft(),
                CardAct::Cancel => {
                    self.reply = None;
                    self.reply_text.clear();
                }
                CardAct::Send => {
                    self.thread_busy.insert(t.id.clone());
                    let body = self.reply_text.trim().to_string();
                    events.push(if t.id == DRAFT {
                        DiffEvent::ThreadNew { branch: t.branch.clone(), path: t.path.clone(), line: t.line, old_side: t.old_side, body }
                    } else {
                        DiffEvent::ThreadReply { id: t.id.clone(), branch: t.branch.clone(), body }
                    });
                }
                CardAct::Resolve => {
                    self.thread_busy.insert(t.id.clone());
                    events.push(DiffEvent::ThreadResolve { id: t.id.clone(), branch: t.branch.clone(), resolve: !t.resolved });
                }
            }
            self.thread_w = 0.0; // re-measure
            ui.ctx().request_repaint();
        }
        if let Some(ti) = thread_toggle {
            self.thread_open[ti] = !self.thread_open[ti];
            self.thread_w = 0.0; // re-measure next frame
            ui.ctx().request_repaint();
        }
        // apply toolbar actions
        if let Some(collapse) = bar_out.all {
            for fi in 0..self.files.len() {
                self.set_open(fi, !collapse);
            }
            self.rebuild();
        }
        if let Some(w) = bar_out.ws {
            prefs.ignore_ws = w;
        }
        if let Some(sp) = bar_out.split {
            prefs.split = sp;
        }
        if bar_out.focus {
            events.push(DiffEvent::ToggleFocus);
        }
        if bar_out.nav != 0 {
            // handled next frame (same code path as ⌥↑/⌥↓)
            self.pending_nav = bar_out.nav;
            ui.ctx().request_repaint();
        }

        self.offset = out.state.clamped_offset().y;
        self.view_h = out.inner_rect.height();
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

/// What a click on a thread card does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CardAct {
    Reply,
    Resolve,
    Link,
    Send,
    Cancel,
}

/// A thread's interaction state, which changes its card: the reply box is
/// open, and/or a request is in flight.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ThreadState {
    replying: bool,
    busy: bool,
}

/// A small button in a diff row, drawn to fit inside the row exactly (an
/// egui button would be taller than a row with a large font and spill into
/// the next one). Discarding throws work away, so those buttons look
/// different from Stage / Unstage: red text on a faint red tint.
fn row_button(ui: &mut Ui, r: Rect, id: egui::Id, text: &str, font: &FontId) -> egui::Response {
    let resp = ui.interact(r, id, Sense::click());
    let danger = text.starts_with("Discard");
    let hot = resp.hovered() || resp.has_focus();
    // Neutral buttons: the page colour with a thin outline, so they stand
    // out from the (darker) row in light and dark themes alike.
    let (fg, bg, edge) = match (danger, hot) {
        (true, false) => (theme::ERROR(), theme::ERROR().gamma_multiply(0.12), Color32::TRANSPARENT),
        (true, true) => (theme::ERROR(), theme::ERROR().gamma_multiply(0.24), Color32::TRANSPARENT),
        (false, false) => (theme::TEXT(), ui.visuals().panel_fill, theme::TEXT_DIM().gamma_multiply(0.35)),
        (false, true) => (theme::TEXT(), theme::HOVER(), theme::TEXT_DIM().gamma_multiply(0.6)),
    };
    let p = ui.painter();
    p.rect_filled(r, 4.0, bg);
    p.rect_stroke(r, 4.0, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    if resp.has_focus() {
        p.rect_stroke(r, 4.0, Stroke::new(1.0, theme::ACCENT()), egui::StrokeKind::Inside);
    }
    p.text(r.center(), Align2::CENTER_CENTER, text, font.clone(), fg);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// "@@ -78,3 +78,4 @@ fn login()" → ("Lines 78–81", "fn login()"): the new
/// side's lines (the old side's for a pure deletion) and the context git found.
fn hunk_label(header: &str) -> (String, String) {
    let parse = |s: &str| -> Option<(u64, u64)> {
        let s = &s[1..];
        let (a, n) = s.split_once(',').unwrap_or((s, "1"));
        Some((a.parse().ok()?, n.parse().ok()?))
    };
    let mut parts = header.splitn(4, ' ');
    let (Some("@@"), Some(old), Some(new)) = (parts.next(), parts.next(), parts.next()) else {
        return (header.to_string(), String::new());
    };
    let ctx = parts.next().unwrap_or("").trim_start_matches("@@").trim().to_string();
    let (start, n) = match (parse(old), parse(new)) {
        (_, Some((s, n))) if n > 0 => (s, n),
        (Some((s, n)), _) => (s, n),
        _ => return (header.to_string(), String::new()),
    };
    let range = if n <= 1 { format!("Line {start}") } else { format!("Lines {start}–{}", start + n - 1) };
    (range, ctx)
}

/// A laid-out review thread card: galleys at (x, y) inside the card, its
/// height, the height of the clickable header line, the footer links' rects,
/// and the reply box's rect (when replying).
struct ThreadCard {
    items: Vec<(f32, f32, std::sync::Arc<egui::Galley>, Color32)>,
    h: f32,
    header_h: f32,
    acts: Vec<(CardAct, (f32, f32, f32, f32))>,
    editor: Option<(f32, f32, f32, f32)>,
}

impl DiffView {
    fn thread_state(&self, t: &InlineThread) -> ThreadState {
        ThreadState { replying: self.reply.as_ref() == Some(&t.id), busy: self.thread_busy.contains(&t.id) }
    }
}

fn layout_thread(ui: &Ui, t: &InlineThread, w: f32, open: bool, state: ThreadState) -> ThreadCard {
    let pad = 12.0;
    let body_font = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    let p = ui.painter();
    let mut items = vec![];
    let mut y = 9.0;
    let n = t.comments.len();
    let status = match (t.resolved, t.outdated) {
        (true, _) => "Resolved",
        (false, true) => "Outdated",
        _ => "Review",
    };
    // header: "Review · 3 comments" (folded: also the first comment's start)
    let mut head = format!("{status} · {n} comment{}", if n == 1 { "" } else { "s" });
    let draft = t.id == DRAFT;
    if draft {
        let side = if t.old_side { " (before)" } else { "" };
        head = format!("New comment · {}:{}{side}", t.path.rsplit('/').next().unwrap_or(&t.path), t.line);
    } else if !open {
        if let Some((a, b, _, _)) = t.comments.first() {
            let first: String = b.lines().next().unwrap_or("").chars().take(80).collect();
            head = format!("▸ {head} · {a}: {first}");
        }
    } else {
        head = format!("▾ {head}");
    }
    // one line, cut short before the GitHub mark + PR number on the right
    let mut job = egui::text::LayoutJob::simple(head, small.clone(), theme::TEXT_DIM(), (w - pad - 90.0).max(60.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let hg = p.layout_job(job);
    let header_h = y + hg.size().y + 6.0;
    items.push((pad, y, hg.clone(), theme::TEXT_DIM()));
    y += hg.size().y + 8.0;
    if !open {
        return ThreadCard { items, h: header_h - 2.0, header_h, acts: vec![], editor: None };
    }
    for (i, (author, body, date, _)) in t.comments.iter().enumerate() {
        if i > 0 {
            y += 6.0;
        }
        let mut job = egui::text::LayoutJob::default();
        job.append(author, 0.0, egui::TextFormat { font_id: body_font.clone(), color: theme::TEXT(), ..Default::default() });
        job.append(&format!("   {date}"), 0.0, egui::TextFormat { font_id: small.clone(), color: theme::TEXT_DIM(), ..Default::default() });
        let ag = p.layout_job(job);
        items.push((pad, y, ag.clone(), theme::TEXT()));
        y += ag.size().y + 2.0;
        let bg = p.layout(body.trim().to_string(), body_font.clone(), theme::TEXT(), (w - pad * 2.0).max(80.0));
        items.push((pad, y, bg.clone(), theme::TEXT()));
        y += bg.size().y;
    }
    if !draft {
        y += 8.0;
    }
    // footer: Reply · Resolve / Unresolve · Open on GitHub ↗
    let ThreadState { replying, busy } = state;
    let mut acts = vec![];
    let mut x = pad;
    let mut fh: f32 = 0.0;
    let mut link_item = |label: &str, k: Option<CardAct>, color: Color32, items: &mut Vec<_>, x: &mut f32| {
        let g = p.layout_no_wrap(label.to_string(), small.clone(), color);
        if let Some(k) = k {
            acts.push((k, (*x, y, g.size().x, g.size().y)));
        }
        items.push((*x, y, g.clone(), color));
        fh = fh.max(g.size().y);
        *x += g.size().x + 18.0;
    };
    let can_write = !t.id.is_empty() && !draft;
    if busy {
        link_item("Working…", None, theme::TEXT_DIM(), &mut items, &mut x);
    } else if can_write {
        if !replying {
            link_item("Reply", Some(CardAct::Reply), theme::ACCENT(), &mut items, &mut x);
        }
        link_item(if t.resolved { "Unresolve" } else { "Resolve" }, Some(CardAct::Resolve), theme::ACCENT(), &mut items, &mut x);
    }
    if !t.comments.is_empty() {
        link_item("Open on GitHub ↗", Some(CardAct::Link), theme::ACCENT(), &mut items, &mut x);
    }
    if fh > 0.0 {
        y += fh + 10.0;
    }
    // reply box (stays while sending, so the text isn't lost on failure)
    let mut editor = None;
    if replying {
        let row = ui.fonts_mut(|f| f.row_height(&body_font));
        let eh = row * 3.0 + 12.0;
        editor = Some((pad, y, w - pad * 2.0, eh));
        y += eh + 8.0 + 26.0 + 10.0;
    }
    ThreadCard { items, h: y, header_h, acts, editor }
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
            let color = if kind == LineKind::Added { theme::ADDED_WORD_BG() } else { theme::REMOVED_WORD_BG() };
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

#[cfg(test)]
mod hunk_label_tests {
    use super::hunk_label;

    #[test]
    fn labels() {
        assert_eq!(hunk_label("@@ -78,3 +78,4 @@ line 76 value = 228;"), ("Lines 78–81".into(), "line 76 value = 228;".into()));
        assert_eq!(hunk_label("@@ -1 +1 @@"), ("Line 1".into(), String::new()));
        // all lines removed: the old side's range
        assert_eq!(hunk_label("@@ -5,3 +4,0 @@ fn x()"), ("Lines 5–7".into(), "fn x()".into()));
        assert_eq!(hunk_label("garbage"), ("garbage".into(), String::new()));
    }
}
