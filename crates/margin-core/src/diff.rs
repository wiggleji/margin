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

impl FileDiff {
    /// A patch of hunk `idx` with only the changed lines in `lines` (indices
    /// into the hunk's lines): the rest of the hunk stays as it is.
    ///
    /// `reverse`: the patch will be applied with `--reverse` (unstaging, or
    /// discarding from the working tree). Lines left out are dropped from the
    /// side being created and kept as context on the side being matched, so
    /// that side is still exactly what the file has.
    pub fn lines_patch(&self, idx: usize, lines: &std::collections::BTreeSet<usize>, reverse: bool) -> Option<String> {
        let h = self.hunks.get(idx)?;
        let (old_start, new_start) = hunk_starts(&h.header)?;
        // each line with the "\ No newline" marker that may follow it
        let mut items: Vec<(usize, &DiffLine, Option<&str>)> = vec![];
        for (i, l) in h.lines.iter().enumerate() {
            match (l.kind, items.last_mut()) {
                (LineKind::NoNewline, Some(last)) => last.2 = Some(&l.text),
                (LineKind::NoNewline, None) => {}
                _ => items.push((i, l, None)),
            }
        }
        // A block of removed lines followed by added ones is paired line by
        // line (as side by side shows it), so a staged replacement lands
        // where the line it replaces was, not after the lines kept around it.
        let mut order: Vec<usize> = vec![];
        let mut k = 0;
        while k < items.len() {
            if items[k].1.kind != LineKind::Removed {
                order.push(k);
                k += 1;
                continue;
            }
            let r0 = k;
            while k < items.len() && items[k].1.kind == LineKind::Removed {
                k += 1;
            }
            let a0 = k;
            while k < items.len() && items[k].1.kind == LineKind::Added {
                k += 1;
            }
            for j in 0..(a0 - r0).max(k - a0) {
                order.extend((r0 + j < a0).then_some(r0 + j));
                order.extend((a0 + j < k).then_some(a0 + j));
            }
        }
        let mut body: Vec<(char, &str)> = vec![];
        let mut picked = false;
        for (i, l, marker) in order.into_iter().map(|o| items[o]) {
            let chosen = lines.contains(&i);
            let c = match (l.kind, chosen, reverse) {
                (LineKind::Added, true, _) => Some('+'),
                (LineKind::Removed, true, _) => Some('-'),
                // left out: forward drops an addition and keeps a removal as
                // context; reverse the other way round
                (LineKind::Added, false, false) | (LineKind::Removed, false, true) => None,
                _ => Some(' '),
            };
            picked |= chosen && matches!(l.kind, LineKind::Added | LineKind::Removed);
            if let Some(c) = c {
                body.push((c, &l.text));
                if let Some(m) = marker {
                    body.push(('\\', m));
                }
            }
        }
        if !picked {
            return None;
        }
        let old_n = body.iter().filter(|(c, _)| *c == ' ' || *c == '-').count();
        let new_n = body.iter().filter(|(c, _)| *c == ' ' || *c == '+').count();
        let mut p = String::new();
        for l in &self.header {
            p.push_str(l);
            p.push('\n');
        }
        // (the side being matched keeps its start; the other side's start
        // doesn't matter to a single-hunk `git apply`)
        p.push_str(&format!("@@ -{old_start},{old_n} +{new_start},{new_n} @@\n"));
        for (c, t) in body {
            p.push(c);
            p.push_str(t);
            p.push('\n');
        }
        Some(p)
    }
}

/// The old and new start lines of a hunk header (`@@ -12,3 +12,4 @@`).
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut it = header.split_whitespace().skip(1);
    let num = |s: Option<&str>, sign: char| s?.strip_prefix(sign)?.split(',').next()?.parse::<u32>().ok();
    Some((num(it.next(), '-')?, num(it.next(), '+')?))
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
                f.old_path = Some(unquote(p));
            } else if let Some(p) = line.strip_prefix("rename to ") {
                f.new_path = Some(unquote(p));
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
    let p = unquote(p.trim_end_matches('\t'));
    if p == "/dev/null" {
        return None;
    }
    let p = p.strip_prefix("a/").or_else(|| p.strip_prefix("b/")).unwrap_or(&p);
    Some(p.to_string())
}

/// Undo git's C-style path quoting: `"dir/\355\225\234 \"x\".md"` → `dir/한 "x".md`.
/// Git quotes paths with unusual characters (and, unless `core.quotepath`
/// is off, every non-ASCII one), escaping bytes as octal. Unquoted text is
/// returned as is.
pub fn unquote(p: &str) -> String {
    let Some(inner) = p.strip_prefix('"').and_then(|q| q.strip_suffix('"')) else {
        return p.to_string();
    };
    let b = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        i += 2;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'v' => out.push(11),
            b'0'..=b'7' => {
                // up to three octal digits = one byte
                let mut v = (c - b'0') as u32;
                let mut n = 1;
                while n < 3 && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                    v = v * 8 + (b[i] - b'0') as u32;
                    i += 1;
                    n += 1;
                }
                out.push(v as u8);
            }
            other => out.push(other), // \" and \\
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

    /// Stage, unstage and discard single lines in a real repository.
    #[test]
    fn line_patches_apply_with_git() {
        let d = std::env::temp_dir().join(format!("margin-lines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let g = |args: &[&str]| {
            let o = std::process::Command::new("git").arg("-C").arg(&d).args(["-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).to_string()
        };
        let apply = |patch: &str, args: &[&str]| {
            use std::io::Write;
            let mut c = std::process::Command::new("git").arg("-C").arg(&d).args(["apply", "--whitespace=nowarn"]).args(args).arg("-").stdin(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
            c.stdin.take().unwrap().write_all(patch.as_bytes()).unwrap();
            let o = c.wait_with_output().unwrap();
            assert!(o.status.success(), "git apply {args:?}: {}\n{patch}", String::from_utf8_lossy(&o.stderr));
        };
        g(&["init", "-q"]);
        g(&["config", "user.name", "Mina"]);
        g(&["config", "user.email", "mina@example.com"]);
        g(&["config", "core.autocrlf", "false"]);
        std::fs::write(d.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-qm", "first"]);
        // change two, remove four, add five
        std::fs::write(d.join("a.txt"), "one\nTWO\nthree\nfive\n").unwrap();
        let worktree = |g: &dyn Fn(&[&str]) -> String| parse(&g(&["diff", "--no-color"])).0.remove(0);
        let f = worktree(&g);
        let h = &f.hunks[0];
        let idx = |kind: LineKind, text: &str| h.lines.iter().position(|l| l.kind == kind && l.text == text).unwrap();

        // stage only "two" → "TWO"
        let sel = [idx(LineKind::Removed, "two"), idx(LineKind::Added, "TWO")].into_iter().collect();
        apply(&f.lines_patch(0, &sel, false).unwrap(), &["--cached"]);
        assert_eq!(g(&["show", ":a.txt"]), "one\nTWO\nthree\nfour\n");

        // stage the added "five" alone (without removing "four")
        let f = worktree(&g);
        let h = &f.hunks[0];
        let five = h.lines.iter().position(|l| l.kind == LineKind::Added && l.text == "five").unwrap();
        apply(&f.lines_patch(0, &[five].into_iter().collect(), false).unwrap(), &["--cached"]);
        assert_eq!(g(&["show", ":a.txt"]), "one\nTWO\nthree\nfour\nfive\n");

        // unstage "TWO" again (reverse on the staged diff)
        let staged = parse(&g(&["diff", "--cached", "--no-color"])).0.remove(0);
        let h = &staged.hunks[0];
        let sel = [h.lines.iter().position(|l| l.kind == LineKind::Removed && l.text == "two").unwrap(), h.lines.iter().position(|l| l.kind == LineKind::Added && l.text == "TWO").unwrap()].into_iter().collect();
        apply(&staged.lines_patch(0, &sel, true).unwrap(), &["--cached", "--reverse"]);
        assert_eq!(g(&["show", ":a.txt"]), "one\ntwo\nthree\nfour\nfive\n");

        // discard the removal of "four" from the working tree only
        let f = worktree(&g);
        let h = &f.hunks[0];
        let four = h.lines.iter().position(|l| l.kind == LineKind::Removed && l.text == "four").unwrap();
        apply(&f.lines_patch(0, &[four].into_iter().collect(), true).unwrap(), &["--reverse"]);
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "one\nTWO\nthree\nfour\nfive\n");

        // a replacement in the middle of a block stays in place
        std::fs::write(d.join("b.txt"), "port 1\nhost a\nlog x\n").unwrap();
        g(&["add", "b.txt"]);
        g(&["commit", "-qm", "b"]);
        std::fs::write(d.join("b.txt"), "port 2\nhost b\nlog y\n").unwrap();
        let fb = parse(&g(&["diff", "--no-color", "--", "b.txt"])).0.remove(0);
        let hb = &fb.hunks[0];
        let pick = |t: &str| hb.lines.iter().position(|l| l.text == t).unwrap();
        let sel = [pick("host a"), pick("host b")].into_iter().collect();
        apply(&fb.lines_patch(0, &sel, false).unwrap(), &["--cached"]);
        assert_eq!(g(&["show", ":b.txt"]), "port 1\nhost b\nlog x\n");
        // and discarding it from the working tree too
        apply(&fb.lines_patch(0, &sel, true).unwrap(), &["--reverse"]);
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "port 2\nhost a\nlog y\n");

        // nothing chosen (or only context): no patch
        assert!(f.lines_patch(0, &[0].into_iter().collect(), false).is_none());
    }

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

    #[test]
    fn quoted_paths() {
        use super::unquote;
        // "한글.md" as git writes it with core.quotepath on
        assert_eq!(unquote(r#""docs/\355\225\234\352\270\200.md""#), "docs/한글.md");
        assert_eq!(unquote(r#""a/say \"hi\".txt""#), r#"a/say "hi".txt"#);
        assert_eq!(unquote(r#""tab\there""#), "tab\there");
        assert_eq!(unquote("plain/path.rs"), "plain/path.rs");
        let d = concat!(
            r#"diff --git "a/\355\225\234.md" "b/\355\225\234.md""#, "\n",
            "new file mode 100644\n",
            "--- /dev/null\n",
            r#"+++ "b/\355\225\234.md""#, "\n",
            "@@ -0,0 +1 @@\n",
            "+x\n",
        );
        let (files, _) = super::parse(d);
        assert_eq!(files[0].new_path.as_deref(), Some("한.md"));
    }
}
