//! Note content policy, enforced on every write (GUI *and* MCP):
//!
//! * CommonMark + tables, strikethrough, task lists
//! * links allowed: http(s), mailto, relative/anchor, and `margin:` internal links
//! * NOT allowed: images, raw HTML (which could smuggle `<img>`/`<video>`),
//!   and dangerous link schemes (`javascript:`, `data:`, `file:` …)
//! * size cap so notes stay light

use pulldown_cmark::{Event, Options, Parser, Tag};

pub const MAX_NOTE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

pub fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

pub fn validate(md: &str) -> Result<(), Vec<Violation>> {
    let mut v = Vec::new();
    if md.len() > MAX_NOTE_BYTES {
        v.push(Violation {
            line: 1,
            message: format!("note is {} bytes; limit is {MAX_NOTE_BYTES}", md.len()),
        });
        return Err(v);
    }
    let line_of = |off: usize| md[..off.min(md.len())].bytes().filter(|&b| b == b'\n').count() + 1;

    for (ev, range) in Parser::new_ext(md, options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::Image { .. }) => v.push(Violation {
                line: line_of(range.start),
                message: "images are not allowed in notes (use a link instead)".into(),
            }),
            Event::Html(_) | Event::InlineHtml(_) => v.push(Violation {
                line: line_of(range.start),
                message: "raw HTML is not allowed in notes".into(),
            }),
            Event::Start(Tag::Link { dest_url, .. }) => {
                if !link_allowed(&dest_url) {
                    v.push(Violation {
                        line: line_of(range.start),
                        message: format!("link scheme not allowed: {dest_url}"),
                    });
                }
            }
            _ => {}
        }
    }
    v.dedup();
    if v.is_empty() { Ok(()) } else { Err(v) }
}

pub fn link_allowed(url: &str) -> bool {
    match scheme(url) {
        None => true, // relative path or #anchor
        Some(s) => matches!(s.as_str(), "http" | "https" | "mailto" | "margin"),
    }
}

fn scheme(url: &str) -> Option<String> {
    let url = url.trim();
    let end = url.find(|c: char| c == ':' || c == '/' || c == '?' || c == '#')?;
    if url.as_bytes()[end] != b':' || end == 0 {
        return None;
    }
    let s = &url[..end];
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        .then(|| s.to_ascii_lowercase())
}

pub fn format_violations(v: &[Violation]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_markdown_with_links() {
        let md = "# Plan\n\n- [x] repro\n- [ ] fix — see [PROJ-12](https://acme.atlassian.net/browse/PROJ-12)\n\n<https://example.com>\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        assert!(validate(md).is_ok());
    }

    #[test]
    fn rejects_images_html_and_bad_schemes() {
        let md = "ok\n\n![x](https://e.com/a.png)\n\n<video src=x></video>\n\n[click](javascript:alert(1))\n[d](DATA:text/html,hi)";
        let v = validate(md).unwrap_err();
        assert_eq!(v.len(), 4, "{v:?}");
        assert_eq!(v[0].line, 3);
    }
}
