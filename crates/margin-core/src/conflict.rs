//! Merge conflicts: read a conflicted file's markers into parts, choose a
//! side (or both, or your own text) for each conflict, write the result and
//! mark the file resolved. Also the whole-file choices git offers (take one
//! side, keep or delete a file one side deleted).

use crate::git::Repo;
use crate::{Result, bail};
use std::path::PathBuf;

/// One piece of a conflicted file: text both sides agree on, or a conflict.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Conflict(Hunk),
}

/// A conflict: what each side has there (lines with their line endings).
#[derive(Debug, Clone, PartialEq)]
pub struct Hunk {
    /// The current side (`<<<<<<<`): HEAD, the branch you're on (in a rebase,
    /// the branch you're rebasing onto).
    pub ours: String,
    /// The common ancestor, when the file was written with `diff3` markers.
    pub base: Option<String>,
    /// The incoming side (`>>>>>>>`): what's merged in (in a rebase, your commit).
    pub theirs: String,
    /// The names git wrote after the markers (e.g. `HEAD`, `feature/x`).
    pub ours_label: String,
    pub theirs_label: String,
    pub base_label: String,
}

/// What to keep for one conflict.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Ours,
    Theirs,
    /// Both: current first, then incoming.
    OursThenTheirs,
    TheirsThenOurs,
    Base,
    /// Text you wrote.
    Custom(String),
}

impl Hunk {
    /// The text a choice gives.
    pub fn resolve(&self, c: &Choice) -> String {
        let nl = |s: &str| if s.is_empty() || s.ends_with('\n') { s.to_string() } else { format!("{s}\n") };
        match c {
            Choice::Ours => self.ours.clone(),
            Choice::Theirs => self.theirs.clone(),
            Choice::OursThenTheirs => nl(&self.ours) + &self.theirs,
            Choice::TheirsThenOurs => nl(&self.theirs) + &self.ours,
            Choice::Base => self.base.clone().unwrap_or_default(),
            Choice::Custom(s) => s.clone(),
        }
    }

    /// The conflict written back with its markers (left unresolved).
    fn markers(&self, eol: &str) -> String {
        let label = |m: &str, l: &str| if l.is_empty() { format!("{m}{eol}") } else { format!("{m} {l}{eol}") };
        let mut s = label("<<<<<<<", &self.ours_label) + &self.ours;
        if let Some(b) = &self.base {
            s += &label("|||||||", &self.base_label);
            s += b;
        }
        s += &format!("======={eol}");
        s += &self.theirs;
        s + &label(">>>>>>>", &self.theirs_label)
    }
}

fn marker<'a>(line: &'a str, m: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(m)?;
    let rest = rest.trim_end_matches(['\n', '\r']);
    if rest.is_empty() {
        Some("")
    } else {
        rest.strip_prefix(' ')
    }
}

/// Split a conflicted file into parts. `None` when its markers don't pair up
/// (it was edited by hand, or has no conflicts left).
pub fn parse(text: &str) -> Option<Vec<Part>> {
    #[derive(PartialEq)]
    enum At {
        Text,
        Ours,
        Base,
        Theirs,
    }
    let mut parts = Vec::new();
    let mut text_buf = String::new();
    let mut h = Hunk { ours: String::new(), base: None, theirs: String::new(), ours_label: String::new(), theirs_label: String::new(), base_label: String::new() };
    let mut at = At::Text;
    for line in text.split_inclusive('\n') {
        match at {
            At::Text => {
                if let Some(l) = marker(line, "<<<<<<<") {
                    if !text_buf.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text_buf)));
                    }
                    h.ours_label = l.to_string();
                    at = At::Ours;
                } else if marker(line, "=======").is_some() || marker(line, ">>>>>>>").is_some() || marker(line, "|||||||").is_some() {
                    return None;
                } else {
                    text_buf += line;
                }
            }
            At::Ours | At::Base => {
                if let Some(l) = marker(line, "|||||||").filter(|_| at == At::Ours) {
                    h.base = Some(String::new());
                    h.base_label = l.to_string();
                    at = At::Base;
                } else if line.trim_end_matches(['\n', '\r']) == "=======" {
                    at = At::Theirs;
                } else if marker(line, "<<<<<<<").is_some() || marker(line, ">>>>>>>").is_some() {
                    return None;
                } else if at == At::Ours {
                    h.ours += line;
                } else {
                    h.base.get_or_insert_default().push_str(line);
                }
            }
            At::Theirs => {
                if let Some(l) = marker(line, ">>>>>>>") {
                    h.theirs_label = l.to_string();
                    let done = std::mem::replace(&mut h, Hunk { ours: String::new(), base: None, theirs: String::new(), ours_label: String::new(), theirs_label: String::new(), base_label: String::new() });
                    parts.push(Part::Conflict(done));
                    at = At::Text;
                } else if marker(line, "<<<<<<<").is_some() || line.trim_end_matches(['\n', '\r']) == "=======" {
                    return None;
                } else {
                    h.theirs += line;
                }
            }
        }
    }
    if at != At::Text {
        return None;
    }
    if !text_buf.is_empty() {
        parts.push(Part::Text(text_buf));
    }
    parts.iter().any(|p| matches!(p, Part::Conflict(_))).then_some(parts)
}

/// The file's text with each conflict replaced by its choice (one per
/// conflict, in order); conflicts without a choice keep their markers.
pub fn assemble(parts: &[Part], choices: &[Option<Choice>]) -> String {
    let crlf = parts.iter().any(|p| matches!(p, Part::Text(t) if t.contains("\r\n")));
    let eol = if crlf { "\r\n" } else { "\n" };
    let mut out = String::new();
    let mut ci = 0;
    for p in parts {
        match p {
            Part::Text(t) => out += t,
            Part::Conflict(h) => {
                match choices.get(ci).and_then(|c| c.as_ref()) {
                    Some(c) => out += &h.resolve(c),
                    None => out += &h.markers(eol),
                }
                ci += 1;
            }
        }
    }
    out
}

/// What kind of conflict a file has, from its `git status` letters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// Both sides changed it (or both added it): conflict markers in the file.
    Both,
    /// You deleted it (current side); the incoming side changed it.
    DeletedByUs,
    /// The incoming side deleted it; you changed it.
    DeletedByThem,
    BothDeleted,
    /// Only the current side has it (e.g. a rename on one side).
    AddedByUs,
    AddedByThem,
}

pub fn kind(index: char, worktree: char) -> Kind {
    match (index, worktree) {
        ('D', 'U') => Kind::DeletedByUs,
        ('U', 'D') => Kind::DeletedByThem,
        ('D', 'D') => Kind::BothDeleted,
        ('A', 'U') => Kind::AddedByUs,
        ('U', 'A') => Kind::AddedByThem,
        _ => Kind::Both,
    }
}

/// A file looks binary (a NUL byte near the start, like git's own check).
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

/// Which side of a conflict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Ours,
    Theirs,
}

impl Repo {
    fn file(&self, path: &str) -> Result<PathBuf> {
        let p = std::path::Path::new(path);
        if p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            bail!("not a path in this repository: {path}");
        }
        Ok(self.workdir.join(p))
    }

    /// The conflicted file as it is in the working directory (None: not there).
    pub fn conflict_file(&self, path: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.file(path)?) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Write the file (conflicts left in keep their markers), without marking it resolved.
    pub fn conflict_save(&self, path: &str, text: &str) -> Result<()> {
        std::fs::write(self.file(path)?, text)?;
        Ok(())
    }

    /// Write the result and mark the file resolved (stage it).
    pub fn conflict_resolve(&self, path: &str, text: &str) -> Result<()> {
        self.conflict_save(path, text)?;
        self.stage(&[path])
    }

    /// Resolve the whole file with one side's version; a side that deleted
    /// the file deletes it.
    pub fn conflict_take(&self, path: &str, side: Side, kind: Kind) -> Result<()> {
        let deleted = matches!((side, kind), (Side::Ours, Kind::DeletedByUs | Kind::AddedByThem) | (Side::Theirs, Kind::DeletedByThem | Kind::AddedByUs) | (_, Kind::BothDeleted));
        if deleted {
            return self.conflict_delete(path);
        }
        let flag = if side == Side::Ours { "--ours" } else { "--theirs" };
        self.run(&["checkout", flag, "--", path])?;
        self.stage(&[path])
    }

    /// Resolve by deleting the file.
    pub fn conflict_delete(&self, path: &str) -> Result<()> {
        self.run(&["rm", "-q", "-f", "--", path]).map(drop)
    }

    /// Put the conflict markers back as git first wrote them (with the
    /// common ancestor between them), dropping edits to the file.
    pub fn conflict_restart(&self, path: &str) -> Result<()> {
        self.run(&["checkout", "--conflict=diff3", "--", path]).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: &str = "a\n<<<<<<< HEAD\nours 1\nours 2\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> feature/x\nb\n<<<<<<< HEAD\n=======\nadded\n>>>>>>> feature/x\n";

    #[test]
    fn parses_and_assembles() {
        let parts = parse(F).unwrap();
        assert_eq!(parts.len(), 4);
        let Part::Conflict(h) = &parts[1] else { panic!() };
        assert_eq!((h.ours.as_str(), h.base.as_deref(), h.theirs.as_str()), ("ours 1\nours 2\n", Some("base\n"), "theirs\n"));
        assert_eq!((h.ours_label.as_str(), h.theirs_label.as_str()), ("HEAD", "feature/x"));
        // unresolved: written back exactly
        assert_eq!(assemble(&parts, &[None, None]), F);
        assert_eq!(assemble(&parts, &[Some(Choice::Ours), Some(Choice::Theirs)]), "a\nours 1\nours 2\nb\nadded\n");
        assert_eq!(assemble(&parts, &[Some(Choice::TheirsThenOurs), Some(Choice::Custom("x\n".into()))]), "a\ntheirs\nours 1\nours 2\nb\nx\n");
        assert_eq!(assemble(&parts, &[Some(Choice::Base), None]).lines().next(), Some("a"));
    }

    #[test]
    fn both_adds_a_missing_newline() {
        let h = Hunk { ours: "x".into(), base: None, theirs: "y\n".into(), ours_label: String::new(), theirs_label: String::new(), base_label: String::new() };
        assert_eq!(h.resolve(&Choice::OursThenTheirs), "x\ny\n");
    }

    #[test]
    fn broken_markers_are_refused() {
        assert!(parse("a\n<<<<<<< HEAD\nx\n").is_none());
        assert!(parse("a\n=======\n").is_none());
        assert!(parse("no conflicts\n").is_none());
        // a line that merely starts like a marker is text
        assert!(parse("<<<<<<<<x\n").is_none());
    }

    #[test]
    fn crlf_kept() {
        let f = "a\r\n<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> b\r\n";
        let parts = parse(f).unwrap();
        assert_eq!(assemble(&parts, &[None]), f);
        assert_eq!(assemble(&parts, &[Some(Choice::Theirs)]), "a\r\ny\r\n");
    }

    #[test]
    fn resolves_in_a_real_merge() {
        let d = std::env::temp_dir().join(format!("margin-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let git = |args: &[&str]| {
            let o = std::process::Command::new("git").current_dir(&d).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"]).args(args).output().unwrap();
            assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        };
        git(&["init", "-q"]);
        git(&["config", "core.autocrlf", "false"]); // (Windows runners may convert line endings)
        std::fs::write(d.join("f.txt"), "one\ntwo\n").unwrap();
        std::fs::write(d.join("g.txt"), "keep\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        git(&["switch", "-qc", "other"]);
        std::fs::write(d.join("f.txt"), "one\nTWO (other)\n").unwrap();
        std::fs::write(d.join("g.txt"), "changed on other\n").unwrap();
        git(&["commit", "-qam", "other"]);
        git(&["switch", "-q", "main"]);
        std::fs::write(d.join("f.txt"), "one\nTWO (main)\n").unwrap();
        git(&["rm", "-q", "g.txt"]);
        git(&["commit", "-qam", "main"]);
        // stops on the conflicts (exit 1); the same identity as above (CI has none)
        let o = std::process::Command::new("git").current_dir(&d).args(["-c", "user.name=t", "-c", "user.email=t@t", "merge", "other"]).output().unwrap();
        assert!(String::from_utf8_lossy(&o.stdout).contains("CONFLICT"), "{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        let repo = Repo::open(&d).unwrap();
        let st = repo.status().unwrap();
        let k: Vec<_> = st.files.iter().filter(|f| f.conflicted).map(|f| (f.path.as_str(), kind(f.index, f.worktree))).collect();
        assert_eq!(k, [("f.txt", Kind::Both), ("g.txt", Kind::DeletedByUs)]);

        let text = String::from_utf8(repo.conflict_file("f.txt").unwrap().unwrap()).unwrap();
        let parts = parse(&text).unwrap();
        repo.conflict_resolve("f.txt", &assemble(&parts, &[Some(Choice::TheirsThenOurs)])).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("f.txt")).unwrap(), "one\nTWO (other)\nTWO (main)\n");
        // start over brings the markers back, with the base
        repo.conflict_restart("f.txt").unwrap();
        let again = parse(&String::from_utf8(repo.conflict_file("f.txt").unwrap().unwrap()).unwrap()).unwrap();
        let Part::Conflict(h) = &again[1] else { panic!() };
        assert_eq!(h.base.as_deref(), Some("two\n"));
        repo.conflict_take("f.txt", Side::Ours, Kind::Both).unwrap();
        // the incoming side changed g.txt, we deleted it: keep theirs
        repo.conflict_take("g.txt", Side::Theirs, Kind::DeletedByUs).unwrap();
        let st = repo.status().unwrap();
        assert!(st.files.iter().all(|f| !f.conflicted), "{:?}", st.files);
        assert_eq!(std::fs::read_to_string(d.join("g.txt")).unwrap(), "changed on other\n");
        assert!(repo.conflict_file("../x").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
