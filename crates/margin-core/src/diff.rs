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

/// Byte ranges within a line.
pub type Spans = Vec<(usize, usize)>;

/// Word-level changes between a removed line and the added line that
/// replaced it: the byte ranges that differ on each side. `None` when the
/// lines are too different (highlighting would just be noise) or too long.
pub fn word_changes(old: &str, new: &str) -> Option<(Spans, Spans)> {
    let (a, b) = (tokens(old), tokens(new));
    if a.is_empty() || b.is_empty() || a.len() * b.len() > 250_000 {
        return None;
    }
    let (ta, tb): (Vec<&str>, Vec<&str>) = (a.iter().map(|&(s, e)| &old[s..e]).collect(), b.iter().map(|&(s, e)| &new[s..e]).collect());
    // Longest common subsequence of tokens.
    let (n, m) = (ta.len(), tb.len());
    let mut dp = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * (m + 1) + j] = if ta[i] == tb[j] {
                dp[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                dp[(i + 1) * (m + 1) + j].max(dp[i * (m + 1) + j + 1])
            };
        }
    }
    let (mut keep_a, mut keep_b) = (vec![false; n], vec![false; m]);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if ta[i] == tb[j] {
            keep_a[i] = true;
            keep_b[j] = true;
            i += 1;
            j += 1;
        } else if dp[(i + 1) * (m + 1) + j] >= dp[i * (m + 1) + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    // Mostly rewritten lines: no word highlight (the whole line is the change).
    let same: usize = ta.iter().zip(&keep_a).filter(|(t, k)| **k && !t.trim().is_empty()).map(|(t, _)| t.len()).sum();
    let longest = old.trim().len().max(new.trim().len()).max(1);
    if same * 10 < longest * 3 {
        return None;
    }
    let spans = |toks: &[(usize, usize)], keep: &[bool]| -> Spans {
        let mut out: Spans = vec![];
        for (k, &(s, e)) in toks.iter().enumerate() {
            if keep[k] {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.1 == s => last.1 = e,
                _ => out.push((s, e)),
            }
        }
        out
    };
    Some((spans(&a, &keep_a), spans(&b, &keep_b)))
}

/// Words (letters, digits, `_`), whitespace runs, and single other characters.
fn tokens(s: &str) -> Vec<(usize, usize)> {
    let class = |c: char| if c.is_alphanumeric() || c == '_' { 1 } else if c.is_whitespace() { 2 } else { 0 };
    let mut out = vec![];
    let mut start: Option<(usize, u8)> = None;
    for (i, c) in s.char_indices() {
        let k = class(c);
        match start {
            Some((_, sk)) if sk == k && k != 0 => continue,
            Some((st, _)) => {
                out.push((st, i));
                start = Some((i, k));
            }
            None => start = Some((i, k)),
        }
    }
    if let Some((st, _)) = start {
        out.push((st, s.len()));
    }
    out
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

    #[test]
    fn word_changes_mark_only_what_changed() {
        let (a, b) = super::word_changes("    let timeout = 10;", "    let timeout = 30;").unwrap();
        assert_eq!(a, vec![(18, 20)]);
        assert_eq!(b, vec![(18, 20)]);
        let (a, b) = super::word_changes("connect(user, timeout);", "connect_with_retry(user, timeout, retries);").unwrap();
        assert_eq!(&"connect(user, timeout);"[a[0].0..a[0].1], "connect");
        assert!(b.iter().any(|&(s, e)| &"connect_with_retry(user, timeout, retries);"[s..e] == "connect_with_retry"));
        // completely different lines: no word highlight
        assert!(super::word_changes("log(\"done\");", "let retries = 1;").is_none());
        // non-ASCII stays on char boundaries
        let (a, _) = super::word_changes("이름 = 홍길동", "이름 = 김철수").unwrap();
        assert_eq!(&"이름 = 홍길동"[a[0].0..a[0].1], "홍길동");
    }
}
