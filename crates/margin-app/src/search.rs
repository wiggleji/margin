//! History search: free words, plus filters like Sublime Merge / GitHub.
//!
//!   commit:3c7d469   author:sungmin   title:"merge pull"   branch:develop
//!   tag:26.47        task:SER-6114    note:"rollout plan"
//!   after:2026-11-01 before:2026-11-15  after:7d  before:yesterday
//!
//! Terms are combined with AND; quotes allow spaces. Matching is
//! case-insensitive; `commit:` matches the start of the SHA. Dates are the
//! commit's author date in the machine's time zone (as shown in the app): `after:D` includes day D,
//! `before:D` excludes it. Relative dates: `today`, `yesterday`, `7d`, `2w`, `3m`.

use margin_core::git::Commit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Any,
    Commit,
    Author,
    Title,
    Ref,
    Task,
    /// Title of the commit's note, or of a branch note on a branch tip.
    Note,
}

/// Filters for the search box's hint / tooltip.
pub const HELP: &str = "Filters (combine freely):\n  commit:<sha>    author:<name or email>\n  title:<words>   branch:<name>   tag:<name>\n  task:<KEY-123>   note:<note title>\n  after:<date>    before:<date>   (2026-11-01, 2026-11, 7d, 2w, 3m, today, yesterday)\nUse quotes for spaces: title:\"merge pull\"";

#[derive(Debug, PartialEq)]
pub struct Query {
    terms: Vec<(Field, String)>,
    /// Unix-time bounds: `after <= time < before`.
    after: Option<i64>,
    before: Option<i64>,
}

impl Query {
    pub fn parse(q: &str) -> Query {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        Query::parse_at(q, now, &margin_core::local_offset)
    }

    /// `parse` with a fixed "now" (for relative dates; tests).
    /// `tz`: the time zone's offset from UTC at a given time (seconds).
    pub fn parse_at(q: &str, now: i64, tz: &dyn Fn(i64) -> i64) -> Query {
        let mut terms = vec![];
        let (mut after, mut before): (Option<i64>, Option<i64>) = (None, None);
        for tok in tokens(q) {
            // after: / before: (an unreadable date becomes a plain word, so it matches nothing)
            if let Some((k, v)) = tok.split_once(':') {
                let k = k.to_lowercase();
                if matches!(k.as_str(), "after" | "since" | "before" | "until") {
                    let v = v.trim_matches('"');
                    if v.is_empty() {
                        continue;
                    }
                    match day_start(v, now, tz) {
                        Some(t) if k == "after" || k == "since" => after = Some(after.map_or(t, |a| a.max(t))),
                        Some(t) => before = Some(before.map_or(t, |b| b.min(t))),
                        None => terms.push((Field::Any, tok.to_lowercase())),
                    }
                    continue;
                }
            }
            let (field, value) = match tok.split_once(':') {
                Some((k, v)) => match k.to_lowercase().as_str() {
                    "commit" | "sha" | "hash" | "id" => (Field::Commit, v),
                    "author" | "by" => (Field::Author, v),
                    "title" | "message" | "msg" | "subject" => (Field::Title, v),
                    "branch" | "tag" | "ref" => (Field::Ref, v),
                    "task" | "ticket" | "issue" => (Field::Task, v),
                    "note" | "notes" => (Field::Note, v),
                    _ => (Field::Any, tok.as_str()),
                },
                None => (Field::Any, tok.as_str()),
            };
            let value = value.trim_matches('"').to_lowercase();
            if !value.is_empty() {
                terms.push((field, value));
            }
        }
        Query { terms, after, before }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.after.is_none() && self.before.is_none()
    }

    /// `notes`: titles of the notes on this commit (its own, and branch notes
    /// of branches whose tip it is).
    pub fn matches(&self, c: &Commit, notes: &[&str]) -> bool {
        let lc = |s: &str| s.to_lowercase();
        if self.after.is_some_and(|a| c.time < a) || self.before.is_some_and(|b| c.time >= b) {
            return false;
        }
        self.terms.iter().all(|(f, v)| {
            let v = v.as_str();
            let refs = || c.refs.iter().any(|r| lc(r).contains(v));
            match f {
                Field::Commit => c.id.starts_with(v),
                Field::Author => lc(&c.author).contains(v) || lc(&c.email).contains(v),
                Field::Title => lc(&c.subject).contains(v),
                Field::Ref => refs(),
                // either kind of key: PROJ-12 or #12
                Field::Task => margin_core::tasks::extract_keys(&c.subject).iter().chain(&margin_core::tasks::github_refs(&c.subject)).any(|k| lc(k).contains(v)),
                Field::Note => notes.iter().any(|t| lc(t).contains(v)),
                Field::Any => lc(&c.subject).contains(v) || lc(&c.author).contains(v) || c.id.starts_with(v) || refs(),
            }
        })
    }
}

// ------------------------------------------------------------ autocomplete

/// One dropdown entry: shown as `label` (+ dim `detail`); choosing it
/// replaces the word being typed with `insert`.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    pub insert: String,
}

/// What the dropdown can offer, from the loaded history.
#[derive(Default)]
pub struct Data {
    /// Most frequent first.
    pub authors: Vec<String>,
    pub branches: Vec<String>,
    pub tags: Vec<String>,
    pub tasks: Vec<String>,
    pub notes: Vec<String>,
}

const KEYS: &[(&str, &str)] = &[
    ("author:", "name or email"),
    ("title:", "words in the commit title"),
    ("commit:", "start of the SHA"),
    ("branch:", "branch name"),
    ("tag:", "tag name"),
    ("task:", "task key, e.g. PROJ-123"),
    ("note:", "note title"),
    ("after:", "date: 2026-11-01, 7d, today"),
    ("before:", "date: 2026-11-15, yesterday"),
];

/// Suggestions for the word at the end of `input`: filter names while typing a
/// word, values after `key:`. Returns where that word starts (byte offset).
pub fn suggest(input: &str, d: &Data) -> Option<(usize, Vec<Suggestion>)> {
    // start of the last word (spaces inside quotes don't split)
    let mut start = 0;
    let mut quoted = false;
    for (i, ch) in input.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            start = i + ch.len_utf8();
        }
    }
    let word = &input[start..];
    if word.is_empty() {
        return None;
    }
    let out: Vec<Suggestion> = match word.split_once(':') {
        None => {
            let w = word.to_lowercase();
            KEYS.iter()
                .filter(|(k, _)| k.starts_with(&w))
                .map(|(k, det)| Suggestion { label: k.to_string(), detail: det.to_string(), insert: k.to_string() })
                .collect()
        }
        Some((key, value)) => {
            let key = key.to_lowercase();
            let value = value.trim_matches('"').to_lowercase();
            let dates = ["today", "yesterday", "7d", "2w", "1m", "3m"].map(String::from).to_vec();
            let (pool, what): (Vec<&String>, &str) = match key.as_str() {
                "author" | "by" => (d.authors.iter().collect(), "author"),
                "branch" => (d.branches.iter().collect(), "branch"),
                "tag" => (d.tags.iter().collect(), "tag"),
                "ref" => (d.branches.iter().chain(&d.tags).collect(), "ref"),
                "task" | "ticket" | "issue" => (d.tasks.iter().collect(), "task"),
                "note" | "notes" => (d.notes.iter().collect(), "note"),
                "after" | "before" | "since" | "until" => return Some((start, date_suggestions(&key, &value, &dates))),
                _ => return None,
            };
            // prefix matches first, then other matches; nothing when already complete
            let lc: Vec<(String, &String)> = pool.into_iter().map(|p| (p.to_lowercase(), p)).collect();
            if lc.iter().any(|(l, _)| *l == value) {
                return None;
            }
            let pre = lc.iter().filter(|(l, _)| l.starts_with(&value));
            let mid = lc.iter().filter(|(l, _)| !l.starts_with(&value) && l.contains(&value));
            pre.chain(mid)
                .take(8)
                .map(|(_, p)| Suggestion { label: p.to_string(), detail: what.into(), insert: format!("{key}:{} ", quote(p)) })
                .collect()
        }
    };
    (!out.is_empty()).then_some((start, out))
}

fn date_suggestions(key: &str, value: &str, dates: &[String]) -> Vec<Suggestion> {
    let detail = |v: &str| match v {
        "today" | "yesterday" => v.to_string(),
        _ => {
            let (n, u) = v.split_at(v.len() - 1);
            let unit = match u {
                "d" => "day",
                "w" => "week",
                _ => "month",
            };
            format!("{n} {unit}{} ago", if n == "1" { "" } else { "s" })
        }
    };
    dates
        .iter()
        .filter(|v| v.starts_with(value) && v.as_str() != value)
        .map(|v| Suggestion { label: v.clone(), detail: detail(v), insert: format!("{key}:{v} ") })
        .collect()
}

fn quote(v: &str) -> String {
    if v.contains(char::is_whitespace) { format!("\"{v}\"") } else { v.to_string() }
}

/// The history search box with its autocomplete dropdown.
/// ↑/↓ pick, Tab or ↵ insert, Esc closes the dropdown (a second Esc leaves the box).
pub fn search_box(ui: &mut eframe::egui::Ui, rv: &mut crate::app::RepoView, width: f32, hint: &str) -> eframe::egui::Response {
    use eframe::egui::{self, Key, Modifiers};
    let id = egui::Id::unique("commit_search");
    let focused = ui.memory(|m| m.has_focus(id));
    let sugg = if focused && !rv.search_hidden { suggest(&rv.filter, &data(rv)) } else { None };
    let mut accept = None;
    if let Some((_, list)) = &sugg {
        let n = list.len();
        rv.search_sel = rv.search_sel.min(n - 1);
        ui.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                rv.search_sel = (rv.search_sel + 1) % n;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                rv.search_sel = (rv.search_sel + n - 1) % n;
            }
            if i.consume_key(Modifiers::NONE, Key::Tab) || i.consume_key(Modifiers::NONE, Key::Enter) {
                accept = Some(rv.search_sel);
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                rv.search_hidden = true;
            }
        });
    }
    // While the dropdown is open, Tab / Esc / ↑↓ belong to it (not to focus navigation).
    let open = sugg.is_some();
    let filter = egui::EventFilter { tab: open, escape: open, vertical_arrows: open, horizontal_arrows: true };
    let resp = ui.add(egui::TextEdit::singleline(&mut rv.filter).id(id).hint_text(hint).desired_width(width).event_filter(filter));
    if resp.changed() {
        rv.search_hidden = false;
        rv.search_sel = 0;
    }
    if let Some((start, list)) = &sugg {
        if accept.is_none() && !rv.search_hidden {
            egui::Area::new(id.with("dropdown"))
                .order(egui::Order::Foreground)
                .fixed_pos(resp.rect.left_bottom() + egui::vec2(0.0, 2.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(resp.rect.width() - 12.0);
                        ui.style_mut().interaction.selectable_labels = false;
                        for (k, s) in list.iter().enumerate() {
                            let r = ui.add(
                                egui::Button::selectable(
                                    k == rv.search_sel,
                                    egui::Atoms::new((
                                        s.label.as_str(),
                                        egui::Atom::grow(),
                                        egui::RichText::new(&s.detail).small().color(crate::theme::TEXT_DIM()),
                                    )),
                                )
                                .min_size(egui::vec2(ui.available_width(), 0.0)),
                            );
                            if r.hovered() {
                                rv.search_sel = k;
                            }
                            if r.clicked() {
                                accept = Some(k);
                            }
                        }
                    });
                });
        }
        if let Some(k) = accept {
            rv.filter = format!("{}{}", &rv.filter[..*start], list[k].insert);
            rv.search_sel = 0;
            // caret to the end, keep typing
            let end = rv.filter.chars().count();
            if let Some(mut st) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
                st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
                st.store(ui.ctx(), id);
            }
            ui.memory_mut(|m| m.request_focus(id));
        }
    }
    resp
}

/// Autocomplete values from the loaded history and notes.
fn data(rv: &crate::app::RepoView) -> Data {
    use margin_core::git::RefKind;
    let mut d = Data::default();
    let Some(snap) = rv.snap.as_ref() else { return d };
    let mut count: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for c in &snap.commits {
        *count.entry(c.author.as_str()).or_default() += 1;
    }
    let mut authors: Vec<(&str, usize)> = count.into_iter().collect();
    authors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    d.authors = authors.into_iter().map(|(a, _)| a.to_string()).collect();
    for r in &snap.refs {
        match r.kind {
            RefKind::Local | RefKind::Remote => d.branches.push(r.name.clone()),
            RefKind::Tag => d.tags.push(r.name.clone()),
        }
    }
    // tasks: newest first (row_tasks follows history order)
    for c in &snap.commits {
        if let Some(keys) = rv.row_tasks.get(&c.id) {
            d.tasks.extend(keys.iter().cloned());
        }
    }
    let mut notes: Vec<String> = rv.note_index.values().filter(|t| !t.is_empty()).cloned().collect();
    notes.sort();
    notes.dedup();
    d.notes = notes;
    d
}

/// Start of the day (local midnight, by `tz`) a date names: `2026-11-01`,
/// `2026-11` (first of the month), `today`, `yesterday`, `7d` / `2w` / `3m` ago.
fn day_start(v: &str, now: i64, tz: &dyn Fn(i64) -> i64) -> Option<i64> {
    const DAY: i64 = 86_400;
    // local midnight → unix time (offset taken near that moment, for DST)
    let midnight = |local_day_start: i64| local_day_start - tz(local_day_start - tz(now));
    let today = midnight((now + tz(now)).div_euclid(DAY) * DAY);
    let v = v.to_lowercase();
    match v.as_str() {
        "today" => return Some(today),
        "yesterday" => return Some(today - DAY),
        _ => {}
    }
    if let Some(unit) = v.chars().last().filter(|c| matches!(c, 'd' | 'w' | 'm' | 'y')) {
        if let Ok(n) = v[..v.len() - 1].parse::<i64>() {
            let days = n * match unit {
                'd' => 1,
                'w' => 7,
                'm' => 30,
                _ => 365,
            };
            return Some(today - days * DAY);
        }
    }
    let parts: Vec<&str> = v.split(['-', '.', '/']).collect();
    let num = |i: usize| parts.get(i).and_then(|p| p.parse::<i64>().ok());
    let (y, m, d) = match parts.len() {
        2 => (num(0)?, num(1)?, 1),
        3 => (num(0)?, num(1)?, num(2)?),
        _ => return None,
    };
    if !(1970..=9999).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(midnight(days_from_civil(y, m, d) * DAY))
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Split on spaces, keeping "quoted parts" (also after `key:`) together.
fn tokens(q: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for ch in q.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                cur.push(ch);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Query;
    use margin_core::git::Commit;

    fn commit(id: &str, author: &str, subject: &str, refs: &[&str]) -> Commit {
        Commit {
            id: id.into(),
            parents: vec![],
            author: author.into(),
            email: format!("{}@x.io", author.to_lowercase().replace(' ', ".")),
            time: 0,
            refs: refs.iter().map(|r| r.to_string()).collect(),
            subject: subject.into(),
        }
    }

    #[test]
    fn filters() {
        let c = commit("3c7d4690abc", "Sungmin Park", "[SER-6284] 상호 검색 merge pull fix", &["origin/develop", "tag: 26.47.0"]);
        let m = |q: &str| Query::parse(q).matches(&c, &["Rollout Plan"]);
        assert!(m("commit:3c7d"));
        assert!(!m("commit:7d46"));
        assert!(m("author:sungmin"));
        assert!(m("author:sungmin.park@")); // email
        assert!(m("title:\"merge pull\""));
        assert!(!m("title:\"pull merge\""));
        assert!(m("branch:develop tag:26.47"));
        assert!(m("task:ser-6284"));
        assert!(!m("task:SER-1"));
        assert!(m("검색 author:park")); // free word AND filter
        assert!(!m("검색 author:kim"));
        assert!(m("foo:bar") == false); // unknown key = plain word "foo:bar"
        assert!(Query::parse("author:").is_empty());
        assert!(m("note:rollout"));
        assert!(m("note:\"rollout plan\" author:park"));
        assert!(!Query::parse("note:rollout").matches(&c, &[]));
    }

    #[test]
    fn dates() {
        // 2026-11-16 12:00 UTC
        let now = super::days_from_civil(2026, 11, 16) * 86_400 + 12 * 3600;
        assert_eq!(super::days_from_civil(1970, 1, 1), 0);
        assert_eq!(super::days_from_civil(2000, 3, 1), 11_017);
        let at = |y, m, d, h: i64| super::days_from_civil(y, m, d) * 86_400 + h * 3600;
        let mut c = commit("abc", "Kim", "work", &[]);
        let m = |q: &str, c: &Commit| Query::parse_at(q, now, &|_| 0).matches(c, &[]);
        c.time = at(2026, 11, 10, 9);
        assert!(m("after:2026-11-10", &c)); // the day itself is included
        assert!(!m("after:2026-11-11", &c));
        assert!(m("before:2026-11-11", &c));
        assert!(!m("before:2026-11-10", &c)); // before excludes the day
        assert!(m("after:2026-11 before:2026-12", &c));
        assert!(m("after:7d", &c)); // since 2026-11-09
        assert!(!m("after:3d", &c));
        assert!(!m("after:yesterday", &c));
        assert!(m("after:2w author:kim", &c));
        c.time = at(2026, 11, 16, 1);
        assert!(m("after:today", &c));
        // an unreadable date matches nothing rather than being ignored
        assert!(!m("after:someday", &c));
        assert!(Query::parse_at("after:", now, &|_| 0).is_empty());
        // in UTC+9, "after:2026-11-10" starts at 2026-11-09 15:00 UTC
        let kst = |_| 9 * 3600;
        c.time = at(2026, 11, 9, 16); // 2026-11-10 01:00 KST
        assert!(Query::parse_at("after:2026-11-10", now, &kst).matches(&c, &[]));
        assert!(!Query::parse_at("after:2026-11-10", now, &|_| 0).matches(&c, &[]));
    }

    #[test]
    fn autocomplete() {
        use super::{Data, suggest};
        let d = Data {
            authors: vec!["Sungmin Park".into(), "jihyung".into()],
            branches: vec!["develop".into(), "feature/SER-6114".into(), "origin/develop".into()],
            tags: vec!["26.47.0".into()],
            tasks: vec!["SER-6114".into(), "SER-6284".into()],
            notes: vec!["Rollout plan".into()],
        };
        let labels = |q: &str| suggest(q, &d).map(|(s, v)| (s, v.into_iter().map(|x| x.label).collect::<Vec<_>>()));
        // filter names while typing a word
        assert_eq!(labels("au"), Some((0, vec!["author:".to_string()])));
        assert_eq!(labels("fix t").unwrap().1, vec!["title:", "tag:", "task:"]);
        assert_eq!(labels("fix t").unwrap().0, 4);
        // values after key:
        let (start, v) = suggest("x author:sung", &d).unwrap();
        assert_eq!(start, 2);
        assert_eq!(v[0].insert, "author:\"Sungmin Park\" ");
        assert_eq!(labels("branch:dev").unwrap().1, vec!["develop", "origin/develop"]); // prefix first
        assert_eq!(labels("task:6284").unwrap().1, vec!["SER-6284"]);
        assert_eq!(labels("note:roll").unwrap().1, vec!["Rollout plan"]);
        assert_eq!(labels("after:").unwrap().1, vec!["today", "yesterday", "7d", "2w", "1m", "3m"]);
        // nothing to offer
        assert!(suggest("hello ", &d).is_none());
        assert!(suggest("title:abc", &d).is_none());
        assert!(suggest("branch:develop", &d).is_none()); // already complete
    }
}
