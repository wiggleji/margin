//! Unified-diff parsing and per-hunk patch construction.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// "\ No newline at end of file"
    NoNewline,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    /// Raw header lines (`diff --git`, `index`, `---`, `+++`, mode lines…).
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
    pub added: usize,
    pub removed: usize,
}

impl FileDiff {
    pub fn display_path(&self) -> &str {
        self.new_path.as_deref().or(self.old_path.as_deref()).unwrap_or("?")
    }

    /// A standalone patch containing only hunk `idx` — feed to `git apply`.
    pub fn hunk_patch(&self, idx: usize) -> Option<String> {
        let h = self.hunks.get(idx)?;
        let mut p = String::new();
        for l in &self.header {
            p.push_str(l);
            p.push('\n');
        }
        p.push_str(&h.header);
        p.push('\n');
        for l in &h.lines {
            let prefix = match l.kind {
                LineKind::Context => ' ',
                LineKind::Added => '+',
                LineKind::Removed => '-',
                LineKind::NoNewline => '\\',
            };
            p.push(prefix);
            p.push_str(&l.text);
            p.push('\n');
        }
        Some(p)
    }
}

/// Max bytes of diff text we parse; protects the UI from giant generated files.
pub const MAX_DIFF_BYTES: usize = 8 * 1024 * 1024;

pub fn parse(text: &str) -> (Vec<FileDiff>, bool) {
    let truncated = text.len() > MAX_DIFF_BYTES;
    let text = if truncated {
        let mut end = MAX_DIFF_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    } else {
        text
    };

    let mut files: Vec<FileDiff> = Vec::new();
    let mut cur: Option<FileDiff> = None;
    let mut in_hunk = false;
    let (mut old_no, mut new_no) = (0u32, 0u32);

    for line in text.lines() {
        if line.starts_with("diff --git ") || line.starts_with("diff --cc ") {
            files.extend(cur.take());
            cur = Some(FileDiff {
                old_path: None,
                new_path: None,
                header: vec![line.to_string()],
                hunks: vec![],
                binary: false,
                added: 0,
                removed: 0,
            });
            in_hunk = false;
            continue;
        }
        let Some(f) = cur.as_mut() else { continue };

        if line.starts_with("@@") {
            let (o, n) = parse_hunk_header(line);
            old_no = o;
            new_no = n;
            f.hunks.push(Hunk { header: line.to_string(), lines: vec![] });
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            if let Some(p) = line.strip_prefix("--- ") {
                f.old_path = strip_ab(p);
            } else if let Some(p) = line.strip_prefix("+++ ") {
                f.new_path = strip_ab(p);
            } else if let Some(p) = line.strip_prefix("rename from ") {
                f.old_path = Some(p.to_string());
            } else if let Some(p) = line.strip_prefix("rename to ") {
                f.new_path = Some(p.to_string());
            } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
                f.binary = true;
            }
            f.header.push(line.to_string());
            continue;
        }
        let h = f.hunks.last_mut().expect("in hunk");
        let (kind, rest) = match line.as_bytes().first() {
            Some(b'+') => (LineKind::Added, &line[1..]),
            Some(b'-') => (LineKind::Removed, &line[1..]),
            Some(b'\\') => (LineKind::NoNewline, &line[1..]),
            Some(b' ') => (LineKind::Context, &line[1..]),
            None => (LineKind::Context, ""),
            _ => (LineKind::Context, line),
        };
        let (o, n) = match kind {
            LineKind::Context => {
                old_no += 1;
                new_no += 1;
                (Some(old_no - 1), Some(new_no - 1))
            }
            LineKind::Added => {
                f.added += 1;
                new_no += 1;
                (None, Some(new_no - 1))
            }
            LineKind::Removed => {
                f.removed += 1;
                old_no += 1;
                (Some(old_no - 1), None)
            }
            LineKind::NoNewline => (None, None),
        };
        h.lines.push(DiffLine { kind, text: rest.to_string(), old_no: o, new_no: n });
    }
    files.extend(cur);

    // Fill paths from `diff --git a/x b/y` when ---/+++ are absent (binary, mode-only).
    for f in &mut files {
        if f.old_path.is_none() && f.new_path.is_none() {
            if let Some(rest) = f.header[0].strip_prefix("diff --git ") {
                if let Some((a, b)) = rest.split_once(" b/") {
                    f.old_path = Some(a.trim_start_matches("a/").to_string());
                    f.new_path = Some(b.to_string());
                }
            }
        }
    }
    (files, truncated)
}

fn strip_ab(p: &str) -> Option<String> {
    if p == "/dev/null" {
        return None;
    }
    let p = p.strip_prefix("a/").or_else(|| p.strip_prefix("b/")).unwrap_or(p);
    Some(p.trim_end_matches('\t').to_string())
}

fn parse_hunk_header(h: &str) -> (u32, u32) {
    // @@ -a,b +c,d @@
    let mut old = 0;
    let mut new = 0;
    for tok in h.split_whitespace().skip(1) {
        if let Some(v) = tok.strip_prefix('-') {
            old = v.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(0);
        } else if let Some(v) = tok.strip_prefix('+') {
            new = v.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(0);
            break;
        }
    }
    (old, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: &str = "diff --git a/src/x.rs b/src/x.rs
index 111..222 100644
--- a/src/x.rs
+++ b/src/x.rs
@@ -1,3 +1,3 @@ fn main
 a
-b
+B
 c
@@ -10,2 +10,3 @@
 x
+y
 z
";

    #[test]
    fn parses_and_builds_patch() {
        let (files, trunc) = parse(D);
        assert!(!trunc);
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.display_path(), "src/x.rs");
        assert_eq!(f.hunks.len(), 2);
        assert_eq!((f.added, f.removed), (2, 1));
        assert_eq!(f.hunks[1].lines[1].new_no, Some(11));
        let p = f.hunk_patch(1).unwrap();
        assert!(p.starts_with("diff --git a/src/x.rs b/src/x.rs\nindex"));
        assert!(p.contains("+++ b/src/x.rs\n@@ -10,2 +10,3 @@\n x\n+y\n z\n"));
        assert!(!p.contains("-b"));
    }
}
