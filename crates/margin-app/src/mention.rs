//! @mentions in review comments: typing `@` offers people to mention, from
//! the pull request (its author, reviewers and commenters) at once, and from
//! GitHub (anyone who can be mentioned in the repository) as you type more.
//! Choosing one (↑↓ then ↵ / Tab, or a click) writes `@login `.

use eframe::egui::{self, Id, Ui};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Who can be suggested: shared by every comment box (egui temp data), filled
/// by the app (the pull request's people, GitHub's answers).
#[derive(Default)]
pub struct Directory {
    /// The repository (or pull request) these are for; the app resets on a change.
    pub scope: String,
    /// (login, name): people in the pull request, first suggestions.
    pub local: Vec<(String, String)>,
    /// GitHub's answers by query.
    pub remote: HashMap<String, Vec<(String, String)>>,
    /// Queries asked already (answered or not).
    asked: HashSet<String>,
    /// GitHub is reachable (otherwise only `local`).
    pub online: bool,
}

pub type Shared = Arc<Mutex<Directory>>;

fn key() -> Id {
    Id::unique("mention-directory")
}

/// Make `dir` the one comment boxes use (each frame).
pub fn install(ctx: &egui::Context, dir: &Shared) {
    ctx.data_mut(|d| d.insert_temp(key(), dir.clone()));
}

/// People matching `q` (login or name, case-insensitive), the pull
/// request's first, then by how well they match; at most 8.
fn matches(dir: &Directory, q: &str) -> Vec<(String, String)> {
    let ql = q.to_lowercase();
    let mut seen = HashSet::new();
    let mut out: Vec<(u8, usize, (String, String))> = vec![];
    let remote = (1..=q.len()).rev().find_map(|n| q.get(..n).and_then(|p| dir.remote.get(p))).into_iter().flatten();
    for (i, (from_pr, u)) in dir.local.iter().map(|u| (true, u)).chain(remote.map(|u| (false, u))).enumerate() {
        let (l, n) = (u.0.to_lowercase(), u.1.to_lowercase());
        let rank = if l.starts_with(&ql) {
            0
        } else if n.split_whitespace().any(|w| w.starts_with(&ql)) {
            1
        } else if l.contains(&ql) || n.contains(&ql) {
            2
        } else {
            continue;
        };
        if seen.insert(l) {
            out.push((rank + if from_pr { 0 } else { 3 }, i, u.clone()));
        }
    }
    out.sort_by_key(|(r, i, _)| (*r, *i));
    out.into_iter().take(8).map(|(_, _, u)| u).collect()
}

/// `te` with @mention completion. Returns its output, and a query to look
/// up on GitHub (new text after `@`) for the caller to send off.
pub fn show(ui: &mut Ui, id: Id, te: egui::TextEdit<'_>) -> (egui::text_edit::TextEditOutput, Option<String>) {
    let dir: Option<Shared> = ui.ctx().data(|d| d.get_temp(key()));
    let mut lookup = None;
    let out = egui::CompletionPopup::new(id)
        .word_boundary(|c| c.is_whitespace() || "([{,;\"'".contains(c))
        .show(ui, te, |query| {
            let Some(q) = query.word.strip_prefix('@') else { return vec![] };
            // `@` in an email address isn't a mention
            if q.contains('@') {
                return vec![];
            }
            let Some(dir) = &dir else { return vec![] };
            let Ok(mut d) = dir.lock() else { return vec![] };
            if d.online && !q.is_empty() && d.asked.insert(q.to_string()) {
                lookup = Some(q.to_string());
            }
            matches(&d, q)
                .into_iter()
                .map(|(login, name)| {
                    let s = egui::Suggestion::new(format!("@{login} ")).content(format!("@{login}"));
                    if name.is_empty() { s } else { s.description(name) }
                })
                .collect()
        });
    (out.text_edit, lookup)
}

/// The people in a pull request, for the first suggestions: its author,
/// then everyone who reviewed or commented (most recent first).
pub fn people(d: &margin_core::github::PrDetail) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    let names = std::iter::once(d.author.clone())
        .chain(d.comments.iter().map(|c| c.author.clone()))
        .chain(d.threads.iter().flat_map(|t| t.comments.iter().map(|c| c.author.clone())));
    for login in names {
        if !login.is_empty() && !login.ends_with("[bot]") && seen.insert(login.to_lowercase()) {
            out.push((login, String::new()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_request_people_first_then_github() {
        let mut d = Directory { local: vec![("mina".into(), String::new()), ("jo".into(), String::new())], ..Default::default() };
        d.remote.insert("do".into(), vec![("doni-spendit".into(), "Doni Kim".into()), ("dora".into(), String::new()), ("mina".into(), "Mina".into())]);
        let logins = |q: &str| matches(&d, q).into_iter().map(|u| u.0).collect::<Vec<_>>();
        assert_eq!(logins(""), ["mina", "jo"]);
        // typed further than asked: the shorter query's answers still help
        assert_eq!(logins("don"), ["doni-spendit"]);
        assert_eq!(logins("do"), ["doni-spendit", "dora"]);
        // by name too (GitHub answers a name query), and no duplicates
        d.remote.insert("kim".into(), vec![("doni-spendit".into(), "Doni Kim".into()), ("jhk".into(), "Jaehoon Kim".into())]);
        let logins = |q: &str| matches(&d, q).into_iter().map(|u| u.0).collect::<Vec<_>>();
        assert_eq!(logins("kim"), ["doni-spendit", "jhk"]);
        assert_eq!(logins("mi"), ["mina"]);
    }
}
