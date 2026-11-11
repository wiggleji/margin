//! Command palette (⌘P / Ctrl+P), Sublime-style.
//!
//! Fuzzy search over every command; results are grouped (Branch, Tag,
//! Git Flow, …) with a header + icon per group. Commands that need input run
//! as steps inside the palette: pick from a list, type a value, or confirm.
//! ↑/↓ move, Enter runs, Esc goes back / closes.

use crate::app::Action;
use crate::icons::{self, Icon};
use crate::theme;
use eframe::egui::{self, Align2, Key, Modifiers, RichText, Sense, vec2};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Group {
    Operation,
    Repository,
    Remote,
    Commit,
    Branch,
    Tag,
    Stash,
    Worktree,
    GitFlow,
    Submodule,
    GitHub,
    Notes,
    Navigate,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Operation => "In progress",
            Group::Repository => "Repository",
            Group::Remote => "Remote",
            Group::Commit => "Commit",
            Group::Branch => "Branch",
            Group::Tag => "Tag",
            Group::Stash => "Stash",
            Group::Worktree => "Worktree",
            Group::GitFlow => "Git Flow",
            Group::Submodule => "Submodule",
            Group::GitHub => "GitHub",
            Group::Notes => "Notes",
            Group::Navigate => "Go to",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Group::Operation => Icon::Alert,
            Group::Repository => Icon::Repo,
            Group::Remote => Icon::RepoPush,
            Group::Commit => Icon::GitCommit,
            Group::Branch => Icon::GitBranch,
            Group::Tag => Icon::Tag,
            Group::Stash => Icon::Stack,
            Group::Worktree => Icon::FileDirectory,
            Group::GitFlow => Icon::Workflow,
            Group::Submodule => Icon::FileSubmodule,
            Group::GitHub => Icon::Github,
            Group::Notes => Icon::Note,
            Group::Navigate => Icon::Search,
        }
    }
}

#[derive(Clone)]
pub struct Item {
    pub label: String,
    pub detail: String,
    pub value: String,
}

impl Item {
    pub fn new(label: impl Into<String>, detail: impl Into<String>, value: impl Into<String>) -> Self {
        Item { label: label.into(), detail: detail.into(), value: value.into() }
    }
    pub fn simple(v: impl Into<String>) -> Self {
        let v = v.into();
        Item { label: v.clone(), detail: String::new(), value: v }
    }
}

pub enum Step {
    Pick { prompt: String, items: Vec<Item> },
    Input { prompt: String, initial: String },
    Confirm(String),
}

pub type Run = Box<dyn Fn(&[String]) -> Vec<Action>>;

pub struct Command {
    pub group: Group,
    pub title: String,
    /// Right-aligned hint (shortcut or short description).
    pub hint: String,
    /// Extra search words.
    pub keywords: String,
    pub steps: Vec<Step>,
    pub run: Run,
}

pub struct Palette {
    cmds: Vec<Command>,
    query: String,
    sel: usize,
    active: Option<usize>,
    step: usize,
    values: Vec<String>,
    focus: bool,
}

/// What the palette wants after a frame.
pub enum Outcome {
    Open,
    Close,
    Run(Vec<Action>),
}

impl Palette {
    pub fn new(cmds: Vec<Command>) -> Self {
        Palette { cmds, query: String::new(), sel: 0, active: None, step: 0, values: vec![], focus: true }
    }

    pub fn with_query(mut self, q: String) -> Self {
        self.query = q;
        self
    }

    fn enter_step(&mut self) {
        self.sel = 0;
        self.focus = true;
        self.query = match self.active.and_then(|a| self.cmds[a].steps.get(self.step)) {
            Some(Step::Input { initial, .. }) => initial.clone(),
            _ => String::new(),
        };
    }

    /// Advance with a value from the current step; runs the command when done.
    fn submit(&mut self, value: String) -> Option<Vec<Action>> {
        let a = self.active?;
        self.values.push(value);
        self.step += 1;
        if self.step >= self.cmds[a].steps.len() {
            return Some((self.cmds[a].run)(&self.values));
        }
        self.enter_step();
        None
    }

    fn start(&mut self, idx: usize) -> Option<Vec<Action>> {
        self.active = Some(idx);
        self.step = 0;
        self.values.clear();
        if self.cmds[idx].steps.is_empty() {
            return Some((self.cmds[idx].run)(&[]));
        }
        self.enter_step();
        None
    }

    fn back(&mut self) -> bool {
        if self.active.is_none() {
            return false;
        }
        if self.step > 0 {
            self.step -= 1;
            self.values.pop();
        } else {
            self.active = None;
        }
        self.enter_step();
        true
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if esc && !self.back() {
            return Outcome::Close;
        }

        // Owned snapshot of the current step (keeps borrows simple below).
        let view = match self.active.and_then(|a| self.cmds[a].steps.get(self.step)) {
            None => View::List,
            Some(Step::Pick { prompt, .. }) => View::Pick(prompt.clone()),
            Some(Step::Input { prompt, .. }) => View::Input(prompt.clone()),
            Some(Step::Confirm(m)) => View::Confirm(m.clone()),
        };
        let entries = self.entries(&view);
        if !entries.is_empty() {
            if up {
                self.sel = self.sel.saturating_sub(1);
            }
            if down {
                self.sel = (self.sel + 1).min(entries.len() - 1);
            }
            self.sel = self.sel.min(entries.len() - 1);
        }

        let mut clicked: Option<usize> = None;
        let mut go_back = false;
        let width = 600.0_f32.min(ctx.content_rect().width() - 32.0);
        let area = egui::Area::new(egui::Id::unique("command_palette"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_TOP, vec2(0.0, 46.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(8.0).show(ui, |ui| {
                    ui.set_width(width);
                    // Breadcrumb for multi-step commands: "Branch › Rename branch… — New name"
                    if let Some(a) = self.active {
                        let c = &self.cmds[a];
                        let p = match &view {
                            View::Pick(p) | View::Input(p) => p.as_str(),
                            View::Confirm(_) => "Confirm",
                            View::List => "",
                        };
                        ui.horizontal(|ui| {
                            ui.add(icons::image(ui.ctx(), c.group.icon(), 13.0, theme::ACCENT));
                            ui.label(RichText::new(format!("{} › {}", c.group.label(), c.title)).small().color(theme::ACCENT));
                            ui.label(RichText::new(format!("— {p}")).small().color(theme::TEXT_DIM));
                        });
                    }
                    if let View::Confirm(msg) = &view {
                        ui.add_space(4.0);
                        ui.label(msg.as_str());
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("Confirm  ↵").clicked() || enter {
                                clicked = Some(0);
                            }
                            if ui.button("Back  Esc").clicked() {
                                go_back = true;
                            }
                        });
                        return;
                    }
                    let hint = match &view {
                        View::List => "Search commands — branch, tag, git flow, worktree, stash…",
                        View::Pick(_) => "Type to filter, ↵ to choose",
                        _ => "↵ to continue, Esc to go back",
                    };
                    let te = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(hint).desired_width(f32::INFINITY).id_salt("palette_query"));
                    if self.focus {
                        te.request_focus();
                        self.focus = false;
                    }
                    if te.changed() {
                        self.sel = 0;
                    }
                    if let View::Input(_) = view {
                        if enter && !self.query.trim().is_empty() {
                            clicked = Some(0);
                        }
                        return;
                    }

                    ui.add_space(4.0);
                    if entries.is_empty() {
                        ui.weak(if matches!(view, View::List) { "No matching commands" } else { "Nothing to choose" });
                    }
                    egui::ScrollArea::vertical().max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
                        let mut last_group = None;
                        for (row, e) in entries.iter().enumerate() {
                            let (label, detail, group) = match *e {
                                Entry::Cmd(i) => {
                                    let c = &self.cmds[i];
                                    (c.title.clone(), c.hint.clone(), Some(c.group))
                                }
                                Entry::Item(i) => match self.active.and_then(|a| self.cmds[a].steps.get(self.step)) {
                                    Some(Step::Pick { items, .. }) => (items[i].label.clone(), items[i].detail.clone(), None),
                                    _ => continue,
                                },
                            };
                            // Group header: icon + name, once per group.
                            if let Some(g) = group {
                                if last_group != Some(g) {
                                    if last_group.is_some() {
                                        ui.add_space(5.0);
                                    }
                                    ui.horizontal(|ui| {
                                        ui.add(icons::image(ui.ctx(), g.icon(), 13.0, theme::TEXT_DIM));
                                        ui.label(RichText::new(g.label().to_uppercase()).small().strong().color(theme::TEXT_DIM));
                                    });
                                    last_group = Some(g);
                                }
                            }
                            let h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
                            let selected = row == self.sel;
                            if selected || resp.hovered() {
                                ui.painter().rect_filled(rect, 3.0, if selected { theme::SELECTED } else { theme::HOVER });
                            }
                            if selected && (up || down) {
                                resp.scroll_to_me(None);
                            }
                            let font = egui::TextStyle::Body.resolve(ui.style());
                            let indent = if group.is_some() { 22.0 } else { 8.0 };
                            let p = ui.painter_at(rect);
                            p.text(rect.left_center() + vec2(indent, 0.0), Align2::LEFT_CENTER, &label, font, theme::TEXT);
                            if !detail.is_empty() {
                                let small = egui::TextStyle::Small.resolve(ui.style());
                                p.text(rect.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, &detail, small, theme::TEXT_DIM);
                            }
                            if resp.clicked() {
                                self.sel = row;
                                clicked = Some(row);
                            }
                        }
                    });
                    if enter && !entries.is_empty() {
                        clicked = Some(self.sel);
                    }
                });
            });

        if go_back {
            self.back();
            return Outcome::Open;
        }
        // Click outside closes.
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
        if outside {
            return Outcome::Close;
        }
        let Some(row) = clicked else { return Outcome::Open };
        let res = match (&view, entries.get(row)) {
            (View::List, Some(Entry::Cmd(i))) => self.start(*i),
            (View::Pick(_), Some(Entry::Item(i))) => {
                let v = match self.active.and_then(|a| self.cmds[a].steps.get(self.step)) {
                    Some(Step::Pick { items, .. }) => items[*i].value.clone(),
                    _ => return Outcome::Open,
                };
                self.submit(v)
            }
            (View::Input(_), _) => {
                let v = self.query.trim().to_string();
                self.submit(v)
            }
            (View::Confirm(_), _) => self.submit(String::new()),
            _ => None,
        };
        match res {
            Some(actions) => Outcome::Run(actions),
            None => Outcome::Open,
        }
    }

    /// Selectable rows for the current view, best matches first.
    fn entries(&self, view: &View) -> Vec<Entry> {
        let q = self.query.trim();
        match view {
            View::List => {
                let mut scored: Vec<(i32, usize)> = self
                    .cmds
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| {
                        if q.is_empty() {
                            return Some((0, i));
                        }
                        let full = format!("{} {} {}", c.group.label(), c.title, c.keywords);
                        fuzzy(q, &c.title).map(|s| s + 40).max(fuzzy(q, &full)).map(|s| (s, i))
                    })
                    .collect();
                if q.is_empty() {
                    scored.sort_by_key(|&(_, i)| (self.cmds[i].group, i));
                } else {
                    // Groups ordered by their best match; commands by score inside a group.
                    let mut best = std::collections::HashMap::new();
                    for &(s, i) in &scored {
                        let e = best.entry(self.cmds[i].group).or_insert(i32::MIN);
                        *e = (*e).max(s);
                    }
                    scored.sort_by_key(|&(s, i)| (-best[&self.cmds[i].group], self.cmds[i].group, -s, i));
                }
                scored.into_iter().map(|(_, i)| Entry::Cmd(i)).collect()
            }
            View::Pick(_) => {
                let Some(Step::Pick { items, .. }) = self.active.and_then(|a| self.cmds[a].steps.get(self.step)) else { return vec![] };
                let mut scored: Vec<(i32, usize)> = items
                    .iter()
                    .enumerate()
                    .filter_map(|(i, it)| if q.is_empty() { Some((0, i)) } else { fuzzy(q, &format!("{} {}", it.label, it.detail)).map(|s| (s, i)) })
                    .collect();
                scored.sort_by_key(|&(s, i)| (-s, i));
                scored.into_iter().map(|(_, i)| Entry::Item(i)).collect()
            }
            _ => vec![],
        }
    }
}

enum View {
    List,
    Pick(String),
    Input(String),
    Confirm(String),
}

#[derive(Clone, Copy)]
enum Entry {
    Cmd(usize),
    Item(usize),
}

/// Subsequence fuzzy match (case-insensitive, spaces in the query ignored).
/// Rewards consecutive runs and word starts; `None` if not all chars match.
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut last: Option<usize> = None;
    for (i, &c) in t.iter().enumerate() {
        if qi < q.len() && c.to_lowercase().next() == Some(q[qi]) {
            let word_start = i == 0 || matches!(t[i - 1], ' ' | ':' | '/' | '-' | '_' | '.' | '›' | '(');
            score += 10;
            if word_start {
                score += 20;
            }
            match last {
                Some(l) if l + 1 == i => score += 15,
                Some(l) => score -= ((i - l - 1) as i32).min(10),
                None => score -= (i as i32).min(10),
            }
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

#[cfg(test)]
mod tests {
    use super::fuzzy;

    #[test]
    fn fuzzy_ranks_word_starts_and_runs() {
        assert!(fuzzy("nb", "New branch…").is_some());
        assert!(fuzzy("xyz", "New branch…").is_none());
        // "ff" should prefer "Feature: Finish" over "Fetch from remote"
        assert!(fuzzy("ff", "Feature: Finish…") > fuzzy("ff", "Fetch (all remotes, prune)"));
        assert!(fuzzy("tag", "Create tag…") > fuzzy("tag", "Stash with message…"));
        assert_eq!(fuzzy("", "anything"), Some(0));
    }
}
