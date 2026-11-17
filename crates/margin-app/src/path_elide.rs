//! Shorten a path to fit a width, from the middle: folders between the first
//! and the last few become `…`, and a file name that alone is too long keeps
//! its start and extension: `src/…/auth/login.rs`, `too…long.png`.
//!
//! Widths are in monospace cells; wide characters (Hangul, CJK, emoji) take two.

/// Cells a character takes in a monospace font.
fn cells(c: char) -> usize {
    let u = c as u32;
    let wide = matches!(u,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F | 0x1F900..=0x1FAFF | 0x20000..=0x3FFFD);
    if wide { 2 } else { 1 }
}

pub fn width(s: &str) -> usize {
    s.chars().map(cells).sum()
}

/// The first characters of `s` that fit in `max` cells.
fn head(s: &str, max: usize) -> &str {
    let mut w = 0;
    for (i, c) in s.char_indices() {
        w += cells(c);
        if w > max {
            return &s[..i];
        }
    }
    s
}

/// The last characters of `s` that fit in `max` cells.
fn tail(s: &str, max: usize) -> &str {
    let mut w = 0;
    for (i, c) in s.char_indices().rev() {
        w += cells(c);
        if w > max {
            return &s[i + c.len_utf8()..];
        }
    }
    s
}

/// `path` shortened to at most `max` cells (unchanged if it fits).
pub fn elide_path(path: &str, max: usize) -> String {
    if width(path) <= max {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split('/').collect();
    let (name, dirs) = parts.split_last().expect("split gives one part");
    // first folder, …, as many of the last folders as fit, name
    if dirs.len() >= 2 {
        for keep in (0..dirs.len() - 1).rev() {
            let tail_dirs: String = dirs[dirs.len() - keep..].iter().map(|d| format!("{d}/")).collect();
            let cand = format!("{}/…/{tail_dirs}{name}", dirs[0]);
            if width(&cand) <= max {
                return cand;
            }
        }
    }
    let prefix = if dirs.is_empty() { "" } else { "…/" };
    let cand = format!("{prefix}{name}");
    if width(&cand) <= max {
        return cand;
    }
    format!("{prefix}{}", elide_name(name, max.saturating_sub(width(prefix))))
}

/// A file name shortened in the middle, keeping its extension: `too…long.png`.
pub fn elide_name(name: &str, max: usize) -> String {
    if width(name) <= max {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && width(e) <= 8 => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    // stem start … stem end + extension; at least one cell for the start
    let budget = max.saturating_sub(width(&ext) + 1).max(1);
    let front = budget.div_ceil(2).max(1);
    let back = budget - front.min(budget);
    format!("{}…{}{ext}", head(stem, front), tail(stem, back))
}

#[cfg(test)]
mod tests {
    use super::{elide_name, elide_path, width};

    #[test]
    fn fits_unchanged() {
        assert_eq!(elide_path("src/main.rs", 20), "src/main.rs");
    }

    #[test]
    fn middle_folders_first() {
        let p = "gateway/src/main/java/kr/spendit/oa/gateway/DocsRouter.java";
        assert_eq!(elide_path(p, 40), "gateway/…/oa/gateway/DocsRouter.java");
        assert_eq!(elide_path(p, 30), "gateway/…/DocsRouter.java");
        assert_eq!(elide_path(p, 20), "…/DocsRouter.java");
        for m in [12, 16, 20, 30, 40, 50] {
            assert!(width(&elide_path(p, m)) <= m, "{m}");
        }
    }

    #[test]
    fn long_names_keep_extension() {
        let n = "2025-12-19-summary-msa-consolidation-progress.txt";
        let e = elide_name(n, 24);
        assert!(e.starts_with("2025-12-") && e.ends_with("progress.txt") && e.contains('…'), "{e}");
        assert!(width(&e) <= 24);
        assert_eq!(elide_path(&format!("docs/{n}"), 20).chars().take(2).collect::<String>(), "…/");
    }

    #[test]
    fn wide_characters() {
        // Hangul takes two cells each
        assert_eq!(width("한글.md"), 7);
        let p = "문서/회의록/2026년/11월/주간-회의-요약-정리본.md";
        for m in [10, 16, 24, 30] {
            let e = elide_path(p, m);
            assert!(width(&e) <= m, "{m}: {e}");
            assert!(e.ends_with(".md"), "{e}");
        }
    }
}
