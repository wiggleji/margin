//! History search: free words, plus filters like Sublime Merge / GitHub.
//!
//!   commit:3c7d469   author:sungmin   title:"merge pull"   branch:develop
//!   tag:26.47        task:SER-6114
//!
//! Terms are combined with AND; quotes allow spaces. Matching is
//! case-insensitive; `commit:` matches the start of the SHA.

use margin_core::git::Commit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Any,
    Commit,
    Author,
    Title,
    Ref,
    Task,
}

/// Filters for the search box's hint / tooltip.
pub const HELP: &str = "Filters (combine freely):\n  commit:<sha>    author:<name or email>\n  title:<words>   branch:<name>   tag:<name>\n  task:<KEY-123>\nUse quotes for spaces: title:\"merge pull\"";

#[derive(Debug, PartialEq)]
pub struct Query(Vec<(Field, String)>);

impl Query {
    pub fn parse(q: &str) -> Query {
        let mut terms = vec![];
        for tok in tokens(q) {
            let (field, value) = match tok.split_once(':') {
                Some((k, v)) => match k.to_lowercase().as_str() {
                    "commit" | "sha" | "hash" | "id" => (Field::Commit, v),
                    "author" | "by" => (Field::Author, v),
                    "title" | "message" | "msg" | "subject" => (Field::Title, v),
                    "branch" | "tag" | "ref" => (Field::Ref, v),
                    "task" | "ticket" | "issue" => (Field::Task, v),
                    _ => (Field::Any, tok.as_str()),
                },
                None => (Field::Any, tok.as_str()),
            };
            let value = value.trim_matches('"').to_lowercase();
            if !value.is_empty() {
                terms.push((field, value));
            }
        }
        Query(terms)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn matches(&self, c: &Commit) -> bool {
        let lc = |s: &str| s.to_lowercase();
        self.0.iter().all(|(f, v)| {
            let v = v.as_str();
            let refs = || c.refs.iter().any(|r| lc(r).contains(v));
            match f {
                Field::Commit => c.id.starts_with(v),
                Field::Author => lc(&c.author).contains(v) || lc(&c.email).contains(v),
                Field::Title => lc(&c.subject).contains(v),
                Field::Ref => refs(),
                Field::Task => margin_core::tasks::extract_keys(&c.subject).iter().any(|k| lc(k).contains(v)),
                Field::Any => lc(&c.subject).contains(v) || lc(&c.author).contains(v) || c.id.starts_with(v) || refs(),
            }
        })
    }
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
        let m = |q: &str| Query::parse(q).matches(&c);
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
    }
}
