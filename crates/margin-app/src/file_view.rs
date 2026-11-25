//! One file through time, in place of the history (like review mode):
//! **Commits** lists the commits that changed it (following renames), with
//! what each changed in it; **Blame** shows each line with the commit that
//! last changed it. A block in the blame opens its commit in Commits, or
//! blames the file as it was just before that change.

use crate::app::{Action, RepoView};
use crate::diff_view::{DiffMode, DiffPrefs, DiffView};
use crate::theme;
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Ui, pos2, vec2};
use margin_core::file_history::{Blame, FileCommit};

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    History,
    Blame,
}

/// A blame, ready to draw: syntax colours per line, and the age range.
pub struct ShownBlame {
    pub blame: Blame,
    colors: Vec<Vec<(usize, usize, Color32)>>,
    oldest: i64,
    newest: i64,
}

impl ShownBlame {
    pub fn new(blame: Blame, path: &str) -> ShownBlame {
        let mut colors = vec![];
        if let Some(lang) = crate::syntax::for_path(path) {
            let mut st = crate::syntax::State::Normal;
            for l in &blame.lines {
                colors.push(crate::syntax::line(lang, l, &mut st).into_iter().map(|(s, e, k)| (s, e, k.color())).collect());
            }
        }
        let times = blame.commits.iter().filter(|c| !c.uncommitted).map(|c| c.time);
        let (oldest, newest) = (times.clone().min().unwrap_or(0), times.max().unwrap_or(0));
        ShownBlame { blame, colors, oldest, newest }
    }
}

pub struct FileView {
    pub path: String,
    pub tab: Tab,
    pub generation: u64,
    pub commits: Option<Result<Vec<FileCommit>, String>>,
    /// The commit shown in History (index into `commits`).
    pub sel: Option<usize>,
    pub diff: DiffView,
    /// The revision blamed ("HEAD", or just before a change).
    pub blame_rev: String,
    pub blame: Option<Result<ShownBlame, String>>,
}

impl FileView {
    pub fn new(path: String, tab: Tab, generation: u64) -> FileView {
        FileView { path, tab, generation, commits: None, sel: None, diff: DiffView::default(), blame_rev: "HEAD".into(), blame: None }
    }

    fn commit_index(&self, id: &str) -> Option<usize> {
        self.commits.as_ref()?.as_ref().ok()?.iter().position(|c| c.id == id)
    }
}

pub fn show(ui: &mut Ui, rv: &mut RepoView, prefs: &mut DiffPrefs, actions: &mut Vec<Action>) {
    let Some(f) = rv.file_view.as_mut() else { return };

    egui::Panel::top("file_head").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let typing = ui.memory(|m| m.focused().is_some());
            if ui.button("‹ History").on_hover_text("Back to the history (Esc)").clicked() || (!typing && ui.input(|i| i.key_pressed(egui::Key::Escape))) {
                actions.push(Action::CloseFileView);
            }
            ui.add_space(6.0);
            let (dir, name) = f.path.rsplit_once('/').map_or(("", f.path.as_str()), |(d, n)| (d, n));
            ui.spacing_mut().item_spacing.x = 0.0;
            if !dir.is_empty() {
                ui.label(RichText::new(format!("{dir}/")).monospace().color(theme::TEXT_DIM()));
            }
            ui.label(RichText::new(name).monospace().strong());
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(Ok(c)) = &f.commits {
                ui.label(RichText::new(format!("  {} commit{}", c.len(), if c.len() == 1 { "" } else { "s" })).color(theme::TEXT_DIM()));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                use crate::widgets::{Seg, segmented};
                let segs = [Seg::new(None, "Commits").tip("The commits that changed this file"), Seg::new(None, "Blame").tip("Who last changed each line")];
                if let Some(i) = segmented(ui, &segs, Some((f.tab == Tab::Blame) as usize)) {
                    f.tab = if i == 1 { Tab::Blame } else { Tab::History };
                }
            });
        });
        ui.add_space(4.0);
    });

    match f.tab {
        Tab::History => history(ui, f, prefs, actions),
        Tab::Blame => blame(ui, f, actions),
    }
}

fn history(ui: &mut Ui, f: &mut FileView, prefs: &mut DiffPrefs, actions: &mut Vec<Action>) {
    let now = margin_core::time_now();
    egui::Panel::left("file_commits").resizable(true).default_size(320.0).min_size(200.0).max_size(560.0).show(ui, |ui| {
        ui.add_space(6.0);
        let commits = match &f.commits {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Reading the history…");
                });
                return;
            }
            Some(Err(e)) => {
                ui.colored_label(theme::ERROR(), e);
                return;
            }
            Some(Ok(c)) => c,
        };
        if commits.is_empty() {
            ui.weak("No commits changed this file.");
            return;
        }
        if commits.len() >= margin_core::file_history::MAX_FILE_COMMITS {
            ui.label(RichText::new(format!("The newest {} commits.", margin_core::file_history::MAX_FILE_COMMITS)).small().color(theme::TEXT_DIM()));
        }
        let mut pick = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, c) in commits.iter().enumerate() {
                let (block, _) = crate::review::entry_block(ui, egui::Id::unique(("file-commit", c.id.as_str())), f.sel == Some(i), None, |ui| {
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 70.0).max(40.0);
                        ui.allocate_ui_with_layout(vec2(w, ui.spacing().interact_size.y), Layout::left_to_right(Align::Center), |ui| {
                            ui.add(egui::Label::new(&c.subject).truncate());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("−{}", c.removed)).small().color(theme::REMOVED()));
                            ui.label(RichText::new(format!("+{}", c.added)).small().color(theme::ADDED()));
                        });
                    });
                    let meta = format!("{} · {} · {}", &c.id[..8.min(c.id.len())], c.author, crate::commit_list::rel_time(now, c.time));
                    ui.add(egui::Label::new(RichText::new(meta).small().color(theme::TEXT_DIM())).truncate());
                    if let Some(old) = &c.old_path {
                        ui.add(egui::Label::new(RichText::new(format!("renamed from {old}")).small().color(theme::lane(0))).truncate());
                    }
                });
                if block.clicked() {
                    pick = Some(i);
                }
            }
        });
        if let Some(i) = pick {
            actions.push(Action::FileViewSelect(i));
        }
    });
    egui::CentralPanel::default().show(ui, |ui| {
        if f.sel.is_none() {
            ui.weak("Choose a commit to see what it changed in this file.");
            return;
        }
        let events = f.diff.show(ui, DiffMode::Commit, "file_commit_diff", prefs, None);
        for e in events {
            if let crate::diff_view::DiffEvent::OpenUrl(u) = e {
                actions.push(Action::OpenUrl(u));
            }
        }
    });
}

/// Newer changes in a stronger colour (0 oldest, 1 newest).
fn age_color(t: i64, oldest: i64, newest: i64) -> Color32 {
    let k = if newest > oldest { (t - oldest) as f32 / (newest - oldest) as f32 } else { 1.0 };
    theme::ACCENT().gamma_multiply(0.15 + 0.85 * k.clamp(0.0, 1.0))
}

fn blame(ui: &mut Ui, f: &mut FileView, actions: &mut Vec<Action>) {
    egui::CentralPanel::default().show(ui, |ui| {
        let now = margin_core::time_now();
        if f.blame_rev != "HEAD" {
            ui.horizontal(|ui| {
                let short = f.blame_rev.trim_end_matches('^');
                ui.label(RichText::new(format!("As it was just before {}", &short[..8.min(short.len())])).color(theme::TEXT_DIM()));
                if ui.small_button("Back to the latest").clicked() {
                    actions.push(Action::FileViewBlameAt { rev: "HEAD".into(), path: f.path.clone() });
                }
            });
            ui.add_space(4.0);
        }
        let sb = match &f.blame {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Reading who changed each line…");
                });
                return;
            }
            Some(Err(e)) => {
                ui.colored_label(theme::ERROR(), e);
                return;
            }
            Some(Ok(b)) => b,
        };
        let b = &sb.blame;
        if b.lines.is_empty() {
            ui.weak("The file is empty here.");
            return;
        }
        let font = FontId::monospace(ui.style().text_styles[&egui::TextStyle::Monospace].size);
        let small = egui::TextStyle::Small.resolve(ui.style());
        let row_h = (font.size * 1.4 + 2.0).round();
        let char_w = ui.fonts_mut(|fo| fo.glyph_width(&font, 'm'));
        let gutter = 300.0;
        let num_w = char_w * (b.lines.len().to_string().len() as f32 + 1.0);
        // the change under the pointer, last frame: all its lines light up
        let hover_id = egui::Id::unique("blame-hover");
        let hovered: Option<usize> = ui.data(|d| d.get_temp(hover_id)).flatten();
        let mut now_hovered: Option<usize> = None;
        let mut open: Option<String> = None;
        let mut before: Option<(String, String)> = None;
        egui::ScrollArea::both().auto_shrink([false, false]).show_rows(ui, row_h, b.lines.len(), |ui, range| {
            let longest = b.lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
            ui.set_min_width(gutter + num_w + 16.0 + longest * char_w);
            for i in range {
                let ci = b.line_commit[i];
                let c = &b.commits[ci];
                let first = i == 0 || b.line_commit[i - 1] != ci;
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                let p = ui.painter();
                // the change's block: an age bar, and on its first line the commit
                let gr = Rect::from_min_size(rect.min, vec2(gutter, row_h));
                let bar = if c.uncommitted { theme::lane(2) } else { age_color(c.time, sb.oldest, sb.newest) };
                p.rect_filled(Rect::from_min_size(gr.min, vec2(4.0, row_h)), 0.0, bar);
                let resp = ui.interact(gr, egui::Id::unique(("blame-line", i)), Sense::click());
                if resp.hovered() {
                    now_hovered = Some(ci);
                    if !c.uncommitted {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }
                if hovered == Some(ci) {
                    p.rect_filled(gr, 0.0, theme::HOVER());
                }
                if first {
                    if i > 0 {
                        p.hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, theme::BG_DARK()));
                    }
                    let who = if c.uncommitted { "Not committed yet".to_string() } else { c.author.clone() };
                    let id = if c.uncommitted { "        ".to_string() } else { c.id[..8].to_string() };
                    p.text(pos2(gr.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, &id, font.clone(), theme::TEXT_DIM());
                    let x = gr.left() + 10.0 + char_w * 9.0;
                    let age = if c.uncommitted { String::new() } else { crate::commit_list::rel_time(now, c.time) };
                    let age_w = ui.fonts_mut(|fo| fo.layout_no_wrap(age.clone(), small.clone(), Color32::WHITE).size().x);
                    let room = (gr.right() - 8.0 - age_w - 8.0 - x).max(20.0);
                    let g = ui.fonts_mut(|fo| fo.layout_no_wrap(who.clone(), small.clone(), theme::TEXT()));
                    let who = if g.size().x > room { format!("{}…", who.chars().take((room / g.size().x * who.chars().count() as f32) as usize).collect::<String>()) } else { who };
                    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, who, small.clone(), theme::TEXT());
                    p.text(pos2(gr.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, age, small.clone(), theme::TEXT_DIM());
                }
                if !c.uncommitted {
                    let resp = resp.on_hover_text(format!("{}\n{} · {}\n{}\n\nClick: see what this commit changed", &c.id[..12], c.author, margin_core::format_local(c.time), c.summary));
                    if resp.clicked() {
                        open = Some(c.id.clone());
                    }
                    let id = c.id.clone();
                    let cpath = c.path.clone();
                    resp.context_menu(|ui| {
                        if ui.button("See what this commit changed").clicked() {
                            open = Some(id.clone());
                            ui.close();
                        }
                        if ui.button("Blame before this change").on_hover_text("The file as it was just before this commit, and who changed each line then").clicked() {
                            before = Some((format!("{id}^"), cpath.clone()));
                            ui.close();
                        }
                        if ui.button("Copy commit SHA").clicked() {
                            ui.ctx().copy_text(id.clone());
                            ui.close();
                        }
                    });
                }
                // line number and code
                let nx = gr.right() + 8.0;
                p.text(pos2(nx + num_w, rect.center().y), Align2::RIGHT_CENTER, (i + 1).to_string(), font.clone(), theme::TEXT_DIM());
                let cx = nx + num_w + 12.0;
                let text = b.lines[i].replace('\t', "    ");
                match sb.colors.get(i).filter(|c| !c.is_empty()) {
                    Some(spans) if !b.lines[i].contains('\t') => {
                        let mut job = egui::text::LayoutJob::default();
                        let mut at = 0;
                        let line = &b.lines[i];
                        for &(s, e, col) in spans {
                            let (s, e) = (s.min(line.len()), e.min(line.len()));
                            if s > at {
                                job.append(&line[at..s], 0.0, egui::TextFormat::simple(font.clone(), theme::TEXT()));
                            }
                            if e > s {
                                job.append(&line[s..e], 0.0, egui::TextFormat::simple(font.clone(), col));
                            }
                            at = e.max(at);
                        }
                        if at < line.len() {
                            job.append(&line[at..], 0.0, egui::TextFormat::simple(font.clone(), theme::TEXT()));
                        }
                        let g = ui.fonts_mut(|fo| fo.layout_job(job));
                        p.galley(pos2(cx, rect.center().y - g.size().y / 2.0), g, theme::TEXT());
                    }
                    _ => {
                        p.text(pos2(cx, rect.center().y), Align2::LEFT_CENTER, text, font.clone(), theme::TEXT());
                    }
                }
            }
        });
        ui.data_mut(|d| d.insert_temp(hover_id, now_hovered));
        if hovered != now_hovered {
            ui.ctx().request_repaint();
        }
        if let Some(id) = open {
            match f.commit_index(&id) {
                Some(i) => {
                    f.tab = Tab::History;
                    actions.push(Action::FileViewSelect(i));
                }
                None => {
                    actions.push(Action::CloseFileView);
                    actions.push(Action::JumpTo(id));
                }
            }
        }
        if let Some((rev, path)) = before {
            // the file's name just before the change (it may have been renamed in it)
            let at = rev.trim_end_matches('^');
            let path = f.commits.as_ref().and_then(|c| c.as_ref().ok()).and_then(|c| c.iter().find(|c| c.id == at)).and_then(|c| c.old_path.clone()).unwrap_or(path);
            actions.push(Action::FileViewBlameAt { rev, path });
        }
    });
}
