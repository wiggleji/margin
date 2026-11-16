//! Formatting toolbar for the note editor: wrap the selection (bold, italic,
//! code, link) or toggle a prefix on the selected lines (heading, list, task,
//! quote). Pure text functions; selections are char indices like egui's cursors.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fmt {
    Bold,
    Italic,
    Code,
    Link,
    Heading,
    Bullet,
    Task,
    Quote,
}

impl Fmt {
    pub const ALL: [Fmt; 8] = [Fmt::Heading, Fmt::Bold, Fmt::Italic, Fmt::Code, Fmt::Link, Fmt::Bullet, Fmt::Task, Fmt::Quote];

    /// Toolbar label and tooltip.
    pub fn label(self) -> (&'static str, &'static str) {
        let mac = cfg!(target_os = "macos");
        match self {
            Fmt::Heading => ("H", "Heading"),
            Fmt::Bold => ("B", if mac { "Bold (⌘B)" } else { "Bold (Ctrl+B)" }),
            Fmt::Italic => ("I", if mac { "Italic (⌘I)" } else { "Italic (Ctrl+I)" }),
            Fmt::Code => ("<>", "Code"),
            Fmt::Link => ("Link", if mac { "Link (⌘K)" } else { "Link (Ctrl+K)" }),
            Fmt::Bullet => ("•", "Bulleted list"),
            Fmt::Task => ("☐", "Checklist item"),
            Fmt::Quote => ("❝", "Quote"),
        }
    }
}

fn byte_at(s: &str, ch: usize) -> usize {
    s.char_indices().nth(ch).map_or(s.len(), |(b, _)| b)
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// Apply `f` to `text` with the selection `[a, b)` (char indices, any order).
/// Returns the new text and the new selection.
pub fn apply(text: &str, a: usize, b: usize, f: Fmt) -> (String, usize, usize) {
    let (a, b) = (a.min(b).min(chars(text)), a.max(b).min(chars(text)));
    match f {
        Fmt::Bold => wrap(text, a, b, "**"),
        Fmt::Italic => wrap(text, a, b, "_"),
        Fmt::Code => wrap(text, a, b, "`"),
        Fmt::Link => link(text, a, b),
        Fmt::Heading => prefix_lines(text, a, b, "## "),
        Fmt::Bullet => prefix_lines(text, a, b, "- "),
        Fmt::Task => prefix_lines(text, a, b, "- [ ] "),
        Fmt::Quote => prefix_lines(text, a, b, "> "),
    }
}

/// `sel` → `**sel**`, or remove the markers if they're already there.
fn wrap(text: &str, a: usize, b: usize, m: &str) -> (String, usize, usize) {
    let (ba, bb) = (byte_at(text, a), byte_at(text, b));
    let n = chars(m);
    // Already wrapped (markers just outside the selection): unwrap.
    if ba >= m.len() && text[..ba].ends_with(m) && text[bb..].starts_with(m) {
        let out = format!("{}{}{}", &text[..ba - m.len()], &text[ba..bb], &text[bb + m.len()..]);
        return (out, a - n, b - n);
    }
    let out = format!("{}{m}{}{m}{}", &text[..ba], &text[ba..bb], &text[bb..]);
    (out, a + n, b + n)
}

/// `sel` → `[sel](https://)` with the URL selected; no selection → `[](https://)`
/// with the cursor in the brackets.
fn link(text: &str, a: usize, b: usize) -> (String, usize, usize) {
    let (ba, bb) = (byte_at(text, a), byte_at(text, b));
    let sel = &text[ba..bb];
    let out = format!("{}[{sel}](https://){}", &text[..ba], &text[bb..]);
    if a == b {
        (out, a + 1, a + 1)
    } else {
        let url = b + 3; // after "[sel]("
        (out, url, url + chars("https://"))
    }
}

/// Toggle `p` at the start of every line the selection touches. If all of
/// them already have it, remove it; otherwise add it where missing.
/// (List-like prefixes replace each other: "- " ↔ "- [ ] ".)
fn prefix_lines(text: &str, a: usize, b: usize, p: &str) -> (String, usize, usize) {
    let (ba, bb) = (byte_at(text, a), byte_at(text, b));
    let start = text[..ba].rfind('\n').map_or(0, |i| i + 1);
    // A selection ending right after a newline doesn't include the next line.
    let end_probe = if bb > ba && text[..bb].ends_with('\n') { bb - 1 } else { bb };
    let end = text[end_probe..].find('\n').map_or(text.len(), |i| end_probe + i);
    let block = &text[start..end];
    // "- [ ] x" doesn't count as a plain "- " bullet.
    let has = |l: &str| l.starts_with(p) && !(p == "- " && (l.starts_with("- [ ] ") || l.starts_with("- [x] ")));
    let all = block.split('\n').all(has);
    let new_block: Vec<String> = block
        .split('\n')
        .map(|l| {
            if all {
                l[p.len()..].to_string()
            } else if has(l) {
                l.to_string()
            } else {
                // Swap one list marker for another instead of stacking them.
                let rest = l.strip_prefix("- [ ] ").or_else(|| l.strip_prefix("- [x] ")).or_else(|| l.strip_prefix("- ")).filter(|_| p.starts_with("- "));
                format!("{p}{}", rest.unwrap_or(l))
            }
        })
        .collect();
    let new_block = new_block.join("\n");
    let out = format!("{}{new_block}{}", &text[..start], &text[end..]);
    let s = chars(&text[..start]);
    if a == b {
        // Keep the caret at the end of the (single) line.
        let e = s + chars(&new_block);
        (out, e, e)
    } else {
        (out, s, s + chars(&new_block))
    }
}

#[cfg(test)]
mod tests {
    use super::{Fmt, apply};

    #[test]
    fn wraps_and_unwraps() {
        let (t, a, b) = apply("say hello now", 4, 9, Fmt::Bold);
        assert_eq!((t.as_str(), a, b), ("say **hello** now", 6, 11));
        let (t, a, b) = apply(&t, a, b, Fmt::Bold);
        assert_eq!((t.as_str(), a, b), ("say hello now", 4, 9));
        // empty selection: markers with the caret between them
        assert_eq!(apply("ab", 1, 1, Fmt::Code), ("a``b".to_string(), 2, 2));
        // non-ASCII text
        assert_eq!(apply("한글 텍스트", 3, 6, Fmt::Italic).0, "한글 _텍스트_");
    }

    #[test]
    fn links() {
        assert_eq!(apply("see docs", 4, 8, Fmt::Link), ("see [docs](https://)".to_string(), 11, 19));
        assert_eq!(apply("x", 1, 1, Fmt::Link), ("x[](https://)".to_string(), 2, 2));
    }

    #[test]
    fn line_prefixes() {
        let text = "one\ntwo\nthree";
        // selection spanning "one" and "two"
        let (t, a, b) = apply(text, 1, 5, Fmt::Bullet);
        assert_eq!(t, "- one\n- two\nthree");
        assert_eq!((a, b), (0, 11));
        let (t, _, _) = apply(&t, a, b, Fmt::Bullet);
        assert_eq!(t, text);
        // a bullet becomes a checklist item instead of "- [ ] - one"
        assert_eq!(apply("- one", 2, 2, Fmt::Task).0, "- [ ] one");
        assert_eq!(apply("- [ ] one", 2, 2, Fmt::Bullet).0, "- one");
        // caret only: just that line
        assert_eq!(apply(text, 9, 9, Fmt::Heading).0, "one\ntwo\n## three");
        assert_eq!(apply("", 0, 0, Fmt::Quote), ("> ".to_string(), 2, 2));
    }
}
