//! Insights: who changed what, where and when, from the history (see
//! `margin_core::insights`). Shown in place of the history, like review
//! mode: a range (30 days by default) on the current branch or every branch,
//! then Overview, Directories, Contributors and Hotspots. A folder narrows
//! Contributors to it ("who knows this?"); a person narrows Directories to
//! them ("where do they work?").

use crate::app::{Action, RepoView};
use crate::theme;
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use margin_core::insights::{CommitStat, Filter, Stats, aggregate};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq)]
pub enum Range {
    Days30,
    Days90,
    Year,
    All,
}

impl Range {
    pub fn days(self) -> Option<i64> {
        match self {
            Range::Days30 => Some(30),
            Range::Days90 => Some(90),
            Range::Year => Some(365),
            Range::All => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Overview,
    Folders,
    People,
    Hotspots,
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Commits,
    Lines,
    Files,
    Days,
    Last,
}

pub struct Insights {
    pub range: Range,
    pub all_branches: bool,
    tab: Tab,
    filter: Filter,
    sort: Sort,
    /// Folders shown open in the tree.
    open: HashSet<String>,
    /// The commits of the range (None: loading).
    pub data: Option<Result<Vec<CommitStat>, String>>,
    pub generation: u64,
    /// Stats for `filter` (recomputed when it or the data changes).
    stats: Option<(Filter, Stats)>,
    /// When the data was read (for "this week" in the chart).
    pub now: i64,
}

impl Insights {
    pub fn new(generation: u64) -> Insights {
        Insights {
            range: Range::Days30,
            all_branches: false,
            tab: Tab::Overview,
            filter: Filter { skip_generated: true, ..Default::default() },
            sort: Sort::Commits,
            open: HashSet::new(),
            data: None,
            generation,
            stats: None,
            now: margin_core::time_now(),
        }
    }

    /// New data arrived: the stats are computed again.
    pub fn set_data(&mut self, data: Result<Vec<CommitStat>, String>) {
        self.data = Some(data);
        self.stats = None;
        self.now = margin_core::time_now();
    }

    fn weeks(&self, commits: &[CommitStat]) -> usize {
        match self.range.days() {
            Some(d) => ((d + 6) / 7) as usize,
            None => commits.last().map_or(4, |c| (((self.now - c.time).max(0) / (7 * 86_400)) as usize + 1).clamp(4, 260)),
        }
    }

    fn stats(&mut self) -> Option<&Stats> {
        let Some(Ok(commits)) = &self.data else { return None };
        if self.stats.as_ref().is_none_or(|(f, _)| *f != self.filter) {
            let s = aggregate(commits, &self.filter, self.now, self.weeks(commits), &margin_core::local_offset);
            self.stats = Some((self.filter.clone(), s));
        }
        self.stats.as_ref().map(|(_, s)| s)
    }
}

/// A person's colour (stable by name).
fn person_color(name: &str) -> Color32 {
    let h = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    theme::lane((h % 8) as u16)
}

fn avatar(ui: &mut Ui, name: &str) {
    let sz = ui.text_style_height(&egui::TextStyle::Body).max(14.0);
    let (r, _) = ui.allocate_exact_size(vec2(sz, sz), Sense::hover());
    ui.painter().circle_filled(r.center(), sz / 2.0, person_color(name));
    let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    ui.painter().text(r.center(), Align2::CENTER_CENTER, initial, FontId::proportional(sz * 0.6), Color32::WHITE);
}

fn bar(ui: &mut Ui, frac: f32, w: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(w, 8.0), Sense::hover());
    let fill = Rect::from_min_size(r.min, vec2((w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { 2.0 } else { 0.0 }), 8.0));
    ui.painter().rect_filled(fill, 2.0, theme::ACCENT().gamma_multiply(0.55));
}

fn k(n: u64) -> String {
    if n >= 10_000 {
        let s = format!("{:.1}k", n as f64 / 1000.0);
        s.replace(".0k", "k")
    } else {
        n.to_string()
    }
}

fn lines(ui: &mut Ui, a: u64, r: u64) {
    ui.label(RichText::new(format!("+{}", k(a))).color(theme::ADDED()));
    ui.label(RichText::new(format!("−{}", k(r))).color(theme::REMOVED()));
}

/// A clickable table row: the whole width, hover / selected fill; cells are
/// laid out left to right at the given widths. Returns its response.
fn row(ui: &mut Ui, id: egui::Id, selected: bool, widths: &[f32], mut cell: impl FnMut(&mut Ui, usize)) -> egui::Response {
    let h = ui.spacing().interact_size.y + 6.0;
    let w = widths.iter().sum::<f32>().max(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    let fill = if selected {
        Some(theme::SELECTED())
    } else if resp.hovered() {
        Some(theme::HOVER())
    } else {
        None
    };
    if let Some(c) = fill {
        ui.painter().rect_filled(rect, 3.0, c);
    }
    let mut x = rect.left() + 6.0;
    for (i, cw) in widths.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(*cw - 8.0, h));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(Layout::left_to_right(Align::Center)));
        c.set_clip_rect(r.intersect(ui.clip_rect()));
        c.style_mut().interaction.selectable_labels = false;
        cell(&mut c, i);
        x += cw;
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Column headers (some sortable).
fn header(ui: &mut Ui, widths: &[f32], titles: &[(&str, Option<Sort>)], sort: &mut Sort) {
    let h = ui.spacing().interact_size.y;
    let w = widths.iter().sum::<f32>().max(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let mut x = rect.left() + 6.0;
    for (i, cw) in widths.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(*cw - 8.0, h));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(Layout::left_to_right(Align::Center)));
        if let Some((t, s)) = titles.get(i) {
            let on = s.is_some_and(|s| s == *sort);
            let text = RichText::new(if on { format!("{t} ↓") } else { t.to_string() }).small().color(if on { theme::ACCENT() } else { theme::TEXT_DIM() });
            match s {
                Some(s) => {
                    if c.add(egui::Label::new(text).sense(Sense::click())).on_hover_text("Sort by this").clicked() {
                        *sort = *s;
                    }
                }
                None => {
                    c.label(text);
                }
            }
        }
        x += cw;
    }
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, theme::BG_DARK()));
}

pub fn show(ui: &mut Ui, rv: &mut RepoView, actions: &mut Vec<Action>) {
    let branch = rv.current_branch().map(str::to_string).unwrap_or_else(|| "HEAD".into());
    let repo_name = rv.repo.name();
    let Some(s) = rv.insights.as_mut() else { return };

    egui::Panel::top("insights_head").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let typing = ui.memory(|m| m.focused().is_some());
            if ui.button("‹ History").on_hover_text("Back to the history (Esc)").clicked() || (!typing && ui.input(|i| i.key_pressed(egui::Key::Escape))) {
                actions.push(Action::CloseInsights);
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Insights").strong());
            ui.label(RichText::new(format!("· {repo_name}")).color(theme::TEXT_DIM()));
            if s.data.is_none() {
                ui.spinner();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                use crate::widgets::{Seg, segmented};
                // (right to left: these appear in reverse order)
                let scopes = [Seg::new(None, branch.clone()).tip("This branch's history"), Seg::new(None, "All branches").tip("Every local and remote branch and tag")];
                if let Some(i) = segmented(ui, &scopes, Some(s.all_branches as usize))
                    && (i == 1) != s.all_branches
                {
                    s.all_branches = i == 1;
                    actions.push(Action::LoadInsights);
                }
                ui.add_space(6.0);
                let ranges = [(Range::Days30, "30 days"), (Range::Days90, "90 days"), (Range::Year, "1 year"), (Range::All, "All time")];
                let segs: Vec<Seg> = ranges.iter().map(|(_, l)| Seg::new(None, *l)).collect();
                let cur = ranges.iter().position(|(r, _)| *r == s.range);
                if let Some(i) = segmented(ui, &segs, cur)
                    && ranges[i].0 != s.range
                {
                    s.range = ranges[i].0;
                    actions.push(Action::LoadInsights);
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            use crate::widgets::{Seg, segmented};
            let tabs = [(Tab::Overview, "Overview"), (Tab::Folders, "Directories"), (Tab::People, "Contributors"), (Tab::Hotspots, "Hotspots")];
            let segs: Vec<Seg> = tabs.iter().map(|(_, l)| Seg::new(None, *l)).collect();
            if let Some(i) = segmented(ui, &segs, tabs.iter().position(|(t, _)| *t == s.tab)) {
                s.tab = tabs[i].0;
            }
            ui.add_space(8.0);
            // the filters, as chips to take off
            if let Some(p) = s.filter.path.clone()
                && chip(ui, "Folder", &format!("{p}/"))
            {
                s.filter.path = None;
            }
            if let Some(a) = s.filter.author.clone()
                && chip(ui, "Person", &a)
            {
                s.filter.author = None;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.checkbox(&mut s.filter.skip_generated, "Ignore generated files")
                    .on_hover_text("Leave out lock files, vendored and generated code (package-lock.json, Cargo.lock, vendor/, *.min.js, schema.rb, …): their line counts drown the rest");
            });
        });
        ui.add_space(4.0);
    });

    egui::CentralPanel::default().show(ui, |ui| {
        match &s.data {
            None => {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Reading the history…");
                });
                return;
            }
            Some(Err(e)) => {
                ui.colored_label(theme::ERROR(), format!("Couldn't read the history: {e}"));
                return;
            }
            Some(Ok(c)) if c.len() >= margin_core::insights::MAX_COMMITS => {
                ui.label(RichText::new(format!("Only the newest {} commits are counted.", margin_core::insights::MAX_COMMITS)).small().color(theme::TEXT_DIM()));
            }
            _ => {}
        }
        let tab = s.tab;
        let mut sort = s.sort;
        let mut open = std::mem::take(&mut s.open);
        let filter = s.filter.clone();
        let now = s.now;
        let mut pick: Option<Pick> = None;
        let Some(st) = s.stats() else { return };
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            if st.commits == 0 {
                ui.add_space(10.0);
                ui.weak(if filter.path.is_some() || filter.author.is_some() { "No commits match these filters in this range." } else { "No commits in this range." });
                return;
            }
            match tab {
                Tab::Overview => overview(ui, st, &mut pick),
                Tab::Folders => folders(ui, st, &filter, &mut open, &mut pick),
                Tab::People => people(ui, st, &filter, &mut sort, now, &mut pick),
                Tab::Hotspots => hotspots(ui, st, &mut pick),
            }
            ui.add_space(20.0);
        });
        s.sort = sort;
        s.open = open;
        match pick {
            // a folder (from the overview, or chosen again in the tree): who works there
            Some(Pick::Folder(p, go)) => {
                s.filter.path = Some(p);
                if go {
                    s.tab = Tab::People;
                }
            }
            Some(Pick::Person(a)) => {
                s.filter.author = Some(a);
                s.tab = Tab::Folders;
            }
            Some(Pick::File(p)) => {
                s.filter.path = Some(p);
                s.tab = Tab::People;
            }
            None => {}
        }
    });
}

enum Pick {
    /// a folder; and whether to go to the people who work there
    Folder(String, bool),
    Person(String),
    File(String),
}

/// A filter chip: "Folder src/api/ ×". True when × is clicked.
fn chip(ui: &mut Ui, what: &str, value: &str) -> bool {
    let mut close = false;
    egui::Frame::new().fill(theme::ACCENT().gamma_multiply(0.15)).corner_radius(10.0).inner_margin(egui::Margin { left: 8, right: 2, top: 1, bottom: 1 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(what).small().color(theme::ACCENT()));
            ui.label(RichText::new(value).monospace().color(theme::ACCENT()));
            close = ui.add(egui::Button::new(RichText::new("×").color(theme::ACCENT())).frame(false)).on_hover_text(format!("Clear the {}", what.to_lowercase())).clicked();
        });
    });
    close
}

fn stat_tile(ui: &mut Ui, w: f32, label: &str, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(theme::BG_DARK()).corner_radius(6.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(w - 24.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(label.to_uppercase()).small().color(theme::TEXT_DIM()));
            ui.horizontal(|ui| body(ui));
        });
    });
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title.to_uppercase()).small().strong().color(theme::TEXT_DIM()));
    ui.add_space(2.0);
}

fn overview(ui: &mut Ui, st: &Stats, pick: &mut Option<Pick>) {
    let w = ui.available_width();
    let tile_w = ((w - 3.0 * 8.0) / 4.0).max(120.0);
    let big_size = ui.text_style_height(&egui::TextStyle::Heading) + 4.0;
    let big = |t: String| RichText::new(t).size(big_size).strong();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        stat_tile(ui, tile_w, "Commits", |ui| {
            ui.label(big(st.commits.to_string()));
        });
        stat_tile(ui, tile_w, "People", |ui| {
            ui.label(big(st.people.len().to_string()));
        });
        stat_tile(ui, tile_w, "Files changed", |ui| {
            ui.label(big(st.files.to_string()));
        });
        stat_tile(ui, tile_w, "Lines", |ui| {
            ui.label(big(format!("+{}", k(st.added))).color(theme::ADDED()));
            ui.label(big(format!("−{}", k(st.removed))).color(theme::REMOVED()));
        });
    });

    section(ui, "Commits per week");
    week_chart(ui, &st.weeks);

    section(ui, "When work happens");
    heatmap(ui, &st.hours);
    ui.label(RichText::new("Commit times in your time zone.").small().color(theme::TEXT_DIM()));

    // busiest folders (two levels deep) and most active people, side by side
    let half = (ui.available_width() - 16.0) / 2.0;
    let wide = half > 300.0;
    let mut tops: Vec<(&String, usize)> = st.folders.iter().filter(|(p, _)| p.matches('/').count() <= 1).map(|(p, f)| (p, f.commits)).collect();
    tops.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    // (a parent with only one child says the same: keep the deeper one)
    let tops: Vec<(&String, usize)> = tops.iter().filter(|(p, n)| !tops.iter().any(|(q, m)| m == n && q.starts_with(&format!("{p}/")))).take(6).copied().collect();
    let fmax = tops.first().map_or(1, |t| t.1).max(1);
    let pmax = st.people.first().map_or(1, |p| p.1.commits).max(1);
    let mut lists = |ui: &mut Ui, which: usize, w: f32| {
        if which == 0 {
            section(ui, "Busiest folders");
            for (p, n) in &tops {
                let r = row(ui, egui::Id::unique(("ins-top-folder", p.as_str())), false, &[w * 0.5, w * 0.35, w * 0.15], |ui, i| match i {
                    0 => {
                        ui.add(egui::Label::new(RichText::new(format!("{p}/")).monospace()).truncate());
                    }
                    1 => bar(ui, *n as f32 / fmax as f32, ui.available_width()),
                    _ => {
                        ui.label(n.to_string());
                    }
                });
                if r.on_hover_text("Who works here").clicked() {
                    *pick = Some(Pick::Folder(p.to_string(), true));
                }
            }
        } else {
            section(ui, "Most active");
            for (who, p) in st.people.iter().take(6) {
                let r = row(ui, egui::Id::unique(("ins-top-person", who.as_str())), false, &[w * 0.5, w * 0.35, w * 0.15], |ui, i| match i {
                    0 => {
                        avatar(ui, who);
                        ui.add(egui::Label::new(who.as_str()).truncate());
                    }
                    1 => bar(ui, p.commits as f32 / pmax as f32, ui.available_width()),
                    _ => {
                        ui.label(p.commits.to_string());
                    }
                });
                if r.on_hover_text("Where they work").clicked() {
                    *pick = Some(Pick::Person(who.clone()));
                }
            }
        }
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(half);
                lists(ui, 0, half);
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.set_width(half);
                lists(ui, 1, half);
            });
        });
    } else {
        let w = ui.available_width();
        lists(ui, 0, w);
        lists(ui, 1, w);
    }
}

fn week_chart(ui: &mut Ui, weeks: &[usize]) {
    let w = ui.available_width().clamp(300.0, 900.0);
    let h = 130.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter_at(rect);
    let small = FontId::proportional(10.5);
    let left = 30.0;
    let bottom = 16.0;
    let plot = Rect::from_min_max(pos2(rect.left() + left, rect.top() + 6.0), pos2(rect.right(), rect.bottom() - bottom));
    let max = weeks.iter().copied().max().unwrap_or(0).max(1);
    // a few round gridlines that the data reaches
    let step = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000].into_iter().find(|s| max / s <= 4).unwrap_or(1000);
    let mut v = 0;
    while v <= max {
        let y = plot.bottom() - plot.height() * v as f32 / max as f32;
        p.hline(plot.x_range(), y, Stroke::new(1.0, theme::BG_DARK()));
        p.text(pos2(plot.left() - 6.0, y), Align2::RIGHT_CENTER, v.to_string(), small.clone(), theme::TEXT_DIM());
        v += step;
    }
    let n = weeks.len().max(1);
    let bw = plot.width() / n as f32;
    let hover = ui.input(|i| i.pointer.hover_pos()).filter(|h| plot.contains(*h));
    for (i, &c) in weeks.iter().enumerate() {
        let bh = plot.height() * c as f32 / max as f32;
        let r = Rect::from_min_max(pos2(plot.left() + i as f32 * bw + 1.0, plot.bottom() - bh), pos2(plot.left() + (i + 1) as f32 * bw - 1.0, plot.bottom()));
        let last = i + 1 == n;
        let on = hover.is_some_and(|h| h.x >= r.left() - 1.0 && h.x < r.right() + 1.0);
        p.rect_filled(r, 1.5, if last || on { theme::ACCENT() } else { theme::ACCENT().gamma_multiply(0.45) });
        if on {
            let ago = n - 1 - i;
            let when = match ago {
                0 => "this week".to_string(),
                1 => "last week".to_string(),
                a => format!("{a} weeks ago"),
            };
            egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::unique("ins-week-tip"), egui::PopupAnchor::Pointer).show(|ui| {
                ui.label(format!("{c} commit{} · {when}", if c == 1 { "" } else { "s" }));
            });
        }
    }
    p.text(pos2(plot.left(), rect.bottom()), Align2::LEFT_BOTTOM, format!("{n} weeks ago"), small.clone(), theme::TEXT_DIM());
    p.text(pos2(plot.right(), rect.bottom()), Align2::RIGHT_BOTTOM, "this week", small, theme::TEXT_DIM());
}

fn heatmap(ui: &mut Ui, hours: &[[usize; 24]; 7]) {
    let label_w = 34.0;
    let avail = ui.available_width().clamp(300.0, 760.0);
    let cell = ((avail - label_w) / 24.0 - 2.0).clamp(9.0, 24.0);
    let w = label_w + 24.0 * (cell + 2.0);
    let h = 14.0 + 7.0 * (cell + 2.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter_at(rect);
    let small = FontId::proportional(10.5);
    let max = hours.iter().flatten().copied().max().unwrap_or(0).max(1);
    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    for hr in (0..24).step_by(6) {
        p.text(pos2(rect.left() + label_w + hr as f32 * (cell + 2.0), rect.top()), Align2::LEFT_TOP, hr.to_string(), small.clone(), theme::TEXT_DIM());
    }
    let hover = resp.hover_pos();
    for (d, row) in hours.iter().enumerate() {
        let y = rect.top() + 14.0 + d as f32 * (cell + 2.0);
        p.text(pos2(rect.left(), y + cell / 2.0), Align2::LEFT_CENTER, days[d], small.clone(), theme::TEXT_DIM());
        for (hr, &v) in row.iter().enumerate() {
            let r = Rect::from_min_size(pos2(rect.left() + label_w + hr as f32 * (cell + 2.0), y), vec2(cell, cell));
            let c = if v == 0 { theme::BG_DARK() } else { theme::ACCENT().gamma_multiply(0.2 + 0.8 * v as f32 / max as f32) };
            p.rect_filled(r, 2.0, c);
            if hover.is_some_and(|h| r.contains(h)) {
                egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::unique("ins-heat-tip"), egui::PopupAnchor::Pointer).show(|ui| {
                    ui.label(format!("{} {hr}:00 · {v} commit{}", days[d], if v == 1 { "" } else { "s" }));
                });
            }
        }
    }
}

fn folders(ui: &mut Ui, st: &Stats, filter: &Filter, open: &mut HashSet<String>, pick: &mut Option<Pick>) {
    ui.label(
        RichText::new("Click a folder to open it; click it again to see who works there. Folders where one person made 80% or more of the changes are marked.")
            .small()
            .color(theme::TEXT_DIM()),
    );
    ui.add_space(4.0);
    let w = ui.available_width().max(720.0);
    let widths = [w * 0.34, w * 0.13, w * 0.08, w * 0.14, w * 0.25, w * 0.06];
    let mut dummy = Sort::Commits;
    header(ui, &widths, &[("Folder", None), ("", None), ("Commits", None), ("Lines", None), ("Main contributor", None), ("People", None)], &mut dummy);
    let max = st.folders.values().map(|f| f.commits).max().unwrap_or(1).max(1);
    // when narrowed to a folder, the tree starts there
    let root = filter.path.clone().filter(|p| st.folders.contains_key(p));
    let kids = |parent: Option<&str>| -> Vec<&String> {
        let mut v: Vec<&String> = st
            .folders
            .keys()
            .filter(|d| match parent {
                Some(p) => d.strip_prefix(p).and_then(|r| r.strip_prefix('/')).is_some_and(|r| !r.contains('/')),
                None => !d.contains('/'),
            })
            .collect();
        v.sort_by(|a, b| st.folders[*b].commits.cmp(&st.folders[*a].commits).then(a.cmp(b)));
        v
    };
    let mut stack: Vec<(&String, usize)> = match &root {
        Some(r) => vec![(st.folders.get_key_value(r).map(|(k, _)| k).unwrap(), 0)],
        None => kids(None).into_iter().rev().map(|d| (d, 0)).collect(),
    };
    if let Some(r) = &root {
        open.insert(r.clone());
    }
    let mut n = 0;
    while let Some((d, depth)) = stack.pop() {
        n += 1;
        if n > 2000 {
            break;
        }
        let f = &st.folders[d];
        let children = kids(Some(d));
        let is_open = open.contains(d);
        let name = if depth == 0 { d.as_str() } else { d.rsplit('/').next().unwrap_or(d) };
        let owner = f.owner();
        let selected = filter.path.as_deref() == Some(d.as_str());
        let resp = row(ui, egui::Id::unique(("ins-folder", d.as_str())), selected, &widths, |ui, i| match i {
            0 => {
                ui.add_space(depth as f32 * 16.0);
                let tri = if children.is_empty() { "  " } else if is_open { "▾ " } else { "▸ " };
                ui.label(RichText::new(tri).color(theme::TEXT_DIM()));
                ui.add(egui::Label::new(RichText::new(format!("{name}/")).monospace()).truncate());
            }
            1 => bar(ui, f.commits as f32 / max as f32, ui.available_width()),
            2 => {
                ui.label(f.commits.to_string());
            }
            3 => lines(ui, f.added, f.removed),
            4 => {
                if let Some((who, share)) = owner {
                    avatar(ui, who);
                    ui.add(egui::Label::new(who).truncate());
                    ui.label(RichText::new(format!("{:.0}%", share * 100.0)).color(theme::TEXT_DIM()));
                    if share >= 0.8 && f.commits >= 8 && filter.author.is_none() {
                        egui::Frame::new().fill(theme::lane(2).gamma_multiply(0.18)).corner_radius(4.0).inner_margin(egui::Margin::symmetric(5, 0)).show(ui, |ui| {
                            ui.label(RichText::new("mostly one person").small().color(theme::lane(2)));
                        })
                        .response
                        .on_hover_text("One person made most of the changes here: few others know this code");
                    }
                }
            }
            _ => {
                ui.label(f.people.len().to_string());
            }
        });
        if resp.clicked() {
            if !children.is_empty() && !selected {
                if is_open && depth > 0 {
                    open.remove(d);
                } else {
                    open.insert(d.clone());
                }
            }
            // chosen again: who works there
            *pick = Some(Pick::Folder(d.clone(), selected));
        }
        if is_open {
            for c in children.into_iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    }
}

fn ago(now: i64, t: i64) -> String {
    let d = (now - t).max(0) / 86_400;
    match d {
        0 => "today".into(),
        1 => "yesterday".into(),
        d if d < 45 => format!("{d}d ago"),
        d => format!("{} mo ago", (d + 15) / 30),
    }
}

fn people(ui: &mut Ui, st: &Stats, filter: &Filter, sort: &mut Sort, now: i64, pick: &mut Option<Pick>) {
    ui.label(RichText::new("Click a person to see where they work. People are counted once each, as the repository's .mailmap names them.").small().color(theme::TEXT_DIM()));
    ui.add_space(4.0);
    let w = ui.available_width().max(760.0);
    let widths = [w * 0.22, w * 0.07, w * 0.1, w * 0.13, w * 0.07, w * 0.08, w * 0.22, w * 0.11];
    header(
        ui,
        &widths,
        &[("Person", None), ("Commits", Some(Sort::Commits)), ("", None), ("Lines", Some(Sort::Lines)), ("Files", Some(Sort::Files)), ("Active days", Some(Sort::Days)), ("Works mostly in", None), ("Last commit", Some(Sort::Last))],
        sort,
    );
    let mut rows: Vec<&(String, margin_core::insights::PersonStat)> = st.people.iter().collect();
    let key = |p: &margin_core::insights::PersonStat| -> i64 {
        match sort {
            Sort::Commits => p.commits as i64,
            Sort::Lines => (p.added + p.removed) as i64,
            Sort::Files => p.files as i64,
            Sort::Days => p.active_days as i64,
            Sort::Last => p.last,
        }
    };
    rows.sort_by(|a, b| key(&b.1).cmp(&key(&a.1)).then(a.0.cmp(&b.0)));
    let max = st.people.iter().map(|p| p.1.commits).max().unwrap_or(1).max(1);
    for (who, p) in rows {
        let selected = filter.author.as_deref() == Some(who.as_str());
        let resp = row(ui, egui::Id::unique(("ins-person", who.as_str())), selected, &widths, |ui, i| match i {
            0 => {
                avatar(ui, who);
                ui.add(egui::Label::new(who.as_str()).truncate());
            }
            1 => {
                ui.label(p.commits.to_string());
            }
            2 => bar(ui, p.commits as f32 / max as f32, ui.available_width()),
            3 => lines(ui, p.added, p.removed),
            4 => {
                ui.label(p.files.to_string());
            }
            5 => {
                ui.label(p.active_days.to_string());
            }
            6 => {
                let top: Vec<String> = p.folders.iter().take(2).map(|(d, _)| format!("{d}/")).collect();
                ui.add(egui::Label::new(RichText::new(top.join(", ")).monospace().color(theme::TEXT_DIM())).truncate());
            }
            _ => {
                ui.label(ago(now, p.last));
            }
        });
        if resp.clicked() {
            *pick = Some(Pick::Person(who.clone()));
        }
    }
}

fn hotspots(ui: &mut Ui, st: &Stats, pick: &mut Option<Pick>) {
    ui.label(RichText::new("Files changed most often, and by the most people: where bugs and merge conflicts tend to come from. Click one to see who changed it.").small().color(theme::TEXT_DIM()));
    ui.add_space(4.0);
    let w = ui.available_width().max(720.0);
    let widths = [w * 0.42, w * 0.14, w * 0.08, w * 0.12, w * 0.14, w * 0.1];
    let mut dummy = Sort::Commits;
    header(ui, &widths, &[("File", None), ("", None), ("Commits", None), ("People", None), ("Lines", None), ("", None)], &mut dummy);
    let max = st.files_by_commits.first().map_or(1, |f| f.1.commits).max(1);
    for (path, f) in st.files_by_commits.iter().take(30) {
        let (dir, name) = path.rsplit_once('/').map_or(("", path.as_str()), |(d, n)| (d, n));
        let resp = row(ui, egui::Id::unique(("ins-file", path.as_str())), false, &widths, |ui, i| match i {
            0 => {
                ui.spacing_mut().item_spacing.x = 0.0;
                if !dir.is_empty() {
                    ui.label(RichText::new(format!("{dir}/")).monospace().color(theme::TEXT_DIM()));
                }
                ui.add(egui::Label::new(RichText::new(name).monospace()).truncate());
            }
            1 => bar(ui, f.commits as f32 / max as f32, ui.available_width()),
            2 => {
                ui.label(f.commits.to_string());
            }
            3 => {
                ui.label(f.people.len().to_string());
                if f.people.len() >= 4 {
                    egui::Frame::new().fill(theme::lane(2).gamma_multiply(0.18)).corner_radius(4.0).inner_margin(egui::Margin::symmetric(5, 0)).show(ui, |ui| {
                        ui.label(RichText::new("busy").small().color(theme::lane(2)));
                    })
                    .response
                    .on_hover_text("Many people change this file");
                }
            }
            4 => lines(ui, f.added, f.removed),
            _ => {
                ui.spacing_mut().item_spacing.x = 2.0;
                for who in f.people.iter().take(5) {
                    avatar(ui, who);
                }
            }
        });
        if resp.on_hover_text(f.people.join(", ")).clicked() {
            *pick = Some(Pick::File(path.clone()));
        }
    }
}
