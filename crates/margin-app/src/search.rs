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
                Field::Task => margin_core::tasks::extract_keys(&c.subject).iter().any(|k| lc(k).contains(v)),
                Field::Note => notes.iter().any(|t| lc(t).contains(v)),
                Field::Any => lc(&c.subject).contains(v) || lc(&c.author).contains(v) || c.id.starts_with(v) || refs(),
            }
        })
    }
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
}
