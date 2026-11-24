//! One file through time: the commits that changed it (following renames),
//! what each changed in it, and who last changed each line (blame).

use crate::Result;
use crate::git::{Repo, check_rev};
use std::collections::HashMap;

/// A commit that changed the file.
#[derive(Debug, Clone, PartialEq)]
pub struct FileCommit {
    pub id: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub subject: String,
    /// The file's path in this commit (it may have been renamed since).
    pub path: String,
    /// Its path before this commit, when this commit renamed it.
    pub old_path: Option<String>,
    pub added: u32,
    pub removed: u32,
}

/// Commits read at most for one file.
pub const MAX_FILE_COMMITS: usize = 2000;

impl Repo {
    /// The commits that changed `path` (as it is named at `rev`), newest
    /// first, following renames.
    pub fn file_history(&self, rev: &str, path: &str) -> Result<Vec<FileCommit>> {
        check_rev(rev)?;
        let max = format!("--max-count={MAX_FILE_COMMITS}");
        let out = self.run(&["log", "--follow", "-M", "--no-color", "--numstat", "--format=%x1e%H%x1f%an%x1f%ae%x1f%at%x1f%s", &max, rev, "--", path])?;
        Ok(parse_history(&out, path))
    }

    /// What commit `id` changed in the file (`path` there; `old_path` when it
    /// renamed it), as a patch.
    pub fn file_change_in(&self, id: &str, path: &str, old_path: Option<&str>, ignore_ws: bool) -> Result<String> {
        check_rev(id)?;
        let mut args = vec!["show", "--format=", "--no-color", "--no-ext-diff", "-M", "--diff-merges=first-parent", "--patch"];
        if ignore_ws {
            args.push("-w");
        }
        args.extend([id, "--", path]);
        if let Some(o) = old_path {
            args.push(o);
        }
        self.run(&args)
    }

    /// Every tracked file, by path.
    pub fn tracked_files(&self) -> Result<Vec<String>> {
        Ok(self.run(&["-c", "core.quotePath=false", "ls-files", "-z"])?.split('\0').filter(|f| !f.is_empty()).map(String::from).collect())
    }

    /// Who last changed each line of `path` as of `rev`.
    pub fn blame(&self, rev: &str, path: &str) -> Result<Blame> {
        check_rev(rev)?;
        let out = self.run(&["blame", "--porcelain", rev, "--", path])?;
        Ok(parse_blame(&out))
    }
}

fn parse_history(out: &str, path: &str) -> Vec<FileCommit> {
    // newest first: until a rename is seen, the path is the one asked for
    let mut current = path.to_string();
    out.split('\x1e')
        .filter_map(|rec| {
            let mut lines = rec.lines();
            let f: Vec<&str> = lines.next()?.split('\x1f').collect();
            if f.len() < 5 {
                return None;
            }
            let (mut added, mut removed, mut here, mut old) = (0, 0, current.clone(), None);
            for l in lines {
                let mut it = l.splitn(3, '\t');
                let (Some(a), Some(r), Some(p)) = (it.next(), it.next(), it.next()) else { continue };
                added += a.parse::<u32>().unwrap_or(0);
                removed += r.parse::<u32>().unwrap_or(0);
                let new = crate::insights::renamed(p);
                if p.contains(" => ") {
                    old = Some(old_name(p));
                }
                here = new;
            }
            // older commits name the file by its name before this rename
            if let Some(o) = &old {
                current = o.clone();
            }
            Some(FileCommit { id: f[0].into(), author: f[1].into(), email: f[2].into(), time: f[3].trim().parse().unwrap_or(0), subject: f[4].into(), path: here, old_path: old, added, removed })
        })
        .collect()
}

/// `src/{old => new}/x.rs` → `src/old/x.rs`; `a.rs => b.rs` → `a.rs`.
fn old_name(p: &str) -> String {
    if let (Some(a), Some(b)) = (p.find('{'), p.find('}'))
        && a < b
        && let Some((old, _)) = p[a + 1..b].split_once(" => ")
    {
        return format!("{}{}{}", &p[..a], old, &p[b + 1..]).replace("//", "/");
    }
    p.split_once(" => ").map_or(p, |(o, _)| o).to_string()
}

/// A commit in a blame.
#[derive(Debug, Clone, PartialEq)]
pub struct BlameCommit {
    pub id: String,
    pub author: String,
    pub time: i64,
    pub summary: String,
    /// The file's name in that commit.
    pub path: String,
    /// Not committed yet (changes in the working directory).
    pub uncommitted: bool,
}

/// Each line of the file, and the commit that last changed it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Blame {
    pub lines: Vec<String>,
    /// Index into `commits`, one per line.
    pub line_commit: Vec<usize>,
    pub commits: Vec<BlameCommit>,
}

fn parse_blame(out: &str) -> Blame {
    let mut b = Blame::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut cur: Option<usize> = None;
    for line in out.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            b.lines.push(text.to_string());
            b.line_commit.push(cur.unwrap_or(0));
            continue;
        }
        let mut words = line.split(' ');
        let first = words.next().unwrap_or("");
        if first.len() == 40 && first.bytes().all(|c| c.is_ascii_hexdigit()) {
            let i = *index.entry(first.to_string()).or_insert_with(|| {
                b.commits.push(BlameCommit { id: first.to_string(), author: String::new(), time: 0, summary: String::new(), path: String::new(), uncommitted: first.bytes().all(|c| c == b'0') });
                b.commits.len() - 1
            });
            cur = Some(i);
            continue;
        }
        let Some(i) = cur else { continue };
        let c = &mut b.commits[i];
        let rest = line.split_once(' ').map_or("", |(_, r)| r);
        match first {
            "author" => c.author = rest.to_string(),
            "author-time" => c.time = rest.parse().unwrap_or(0),
            "summary" => c.summary = rest.to_string(),
            "filename" => c.path = rest.to_string(),
            _ => {}
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_blame() {
        let a = "a".repeat(40);
        let z = "0".repeat(40);
        let out = format!("{a} 1 1 2\nauthor Mina\nauthor-time 100\nsummary First\nfilename x.rs\n\tfn main() {{\n{a} 2 2\n\t}}\n{z} 3 3 1\nauthor Not Committed Yet\nauthor-time 200\nsummary Version of x.rs from x.rs\nfilename x.rs\n\t// new\n");
        let b = parse_blame(&out);
        assert_eq!(b.lines, ["fn main() {", "}", "// new"]);
        assert_eq!(b.line_commit, [0, 0, 1]);
        assert_eq!((b.commits[0].author.as_str(), b.commits[0].time, b.commits[0].summary.as_str()), ("Mina", 100, "First"));
        assert!(b.commits[1].uncommitted && !b.commits[0].uncommitted);
    }

    #[test]
    fn renamed_names() {
        assert_eq!(old_name("src/{a => b}/x.rs"), "src/a/x.rs");
        assert_eq!(old_name("old.rs => new.rs"), "old.rs");
    }

    #[test]
    fn history_diff_and_blame_in_a_real_repository() {
        let d = std::env::temp_dir().join(format!("margin-filehist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let g = |args: &[&str]| {
            let o = std::process::Command::new("git").arg("-C").arg(&d).args(["-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        g(&["init", "-q"]);
        g(&["config", "user.name", "Mina"]);
        g(&["config", "user.email", "m@x"]);
        g(&["config", "core.autocrlf", "false"]);
        std::fs::write(d.join("old.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-qm", "add"]);
        g(&["mv", "old.txt", "new.txt"]);
        g(&["commit", "-qm", "rename"]);
        std::fs::write(d.join("new.txt"), "one\nTWO\nthree\nfour\nfive\n").unwrap();
        g(&["-c", "user.name=Jo", "commit", "-qam", "change two"]);
        std::fs::write(d.join("other.txt"), "x\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-qm", "unrelated"]);
        let repo = Repo::open(&d).unwrap();

        let h = repo.file_history("HEAD", "new.txt").unwrap();
        assert_eq!(h.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["change two", "rename", "add"], "renames followed, other commits left out");
        assert_eq!((h[0].path.as_str(), h[0].added, h[0].removed), ("new.txt", 1, 1));
        assert_eq!((h[1].path.as_str(), h[1].old_path.as_deref()), ("new.txt", Some("old.txt")));
        assert_eq!((h[2].path.as_str(), h[2].added), ("old.txt", 5), "before the rename: its old name");

        let diff = repo.file_change_in(&h[0].id, &h[0].path, None, false).unwrap();
        assert!(diff.contains("-two") && diff.contains("+TWO") && !diff.contains("other.txt"), "{diff}");
        let first = repo.file_change_in(&h[2].id, &h[2].path, None, false).unwrap();
        assert!(first.contains("+five"), "{first}");

        let b = repo.blame("HEAD", "new.txt").unwrap();
        assert_eq!(b.lines.len(), 5);
        let who: Vec<&str> = b.line_commit.iter().map(|&i| b.commits[i].author.as_str()).collect();
        assert_eq!(who, ["Mina", "Jo", "Mina", "Mina", "Mina"]);
        assert_eq!(b.commits[b.line_commit[0]].path, "old.txt", "line 1 comes from before the rename");
        // blame before Jo's change: every line from Mina
        let before = repo.blame(&format!("{}^", h[0].id), "new.txt").unwrap();
        assert!(before.line_commit.iter().all(|&i| before.commits[i].author == "Mina"));
        assert!(repo.blame("--nope", "new.txt").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
