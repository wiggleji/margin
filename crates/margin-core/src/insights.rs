//! Repository insights from the history: who changed what, where, and when.
//! One `git log --numstat` pass (people as `.mailmap` names them, merges left
//! out so merged work isn't counted twice), then aggregations by folder,
//! person, file, week and weekday × hour, narrowed by a folder or a person.

use crate::Result;
use crate::git::Repo;
use std::collections::{BTreeMap, HashMap, HashSet};

/// One file's lines in a commit.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// A commit, with what it changed.
#[derive(Debug, Clone, PartialEq)]
pub struct CommitStat {
    pub id: String,
    /// The author as `.mailmap` names them.
    pub author: String,
    pub email: String,
    pub time: i64,
    pub changes: Vec<Change>,
}

/// Commits read at most (a very long "all time" stays bounded).
pub const MAX_COMMITS: usize = 50_000;

/// The commits since `since` (unix time; None: all), on HEAD or on every
/// branch, newest first.
pub fn load(repo: &Repo, since: Option<i64>, all_branches: bool) -> Result<Vec<CommitStat>> {
    if !repo.has_head() {
        return Ok(vec![]);
    }
    let max = format!("--max-count={MAX_COMMITS}");
    let mut args: Vec<String> = ["log", "--no-merges", "--use-mailmap", "--numstat", "-M", "--no-color", "--format=%x1e%H%x1f%aN%x1f%aE%x1f%at", &max].map(String::from).to_vec();
    if let Some(t) = since {
        args.push(format!("--since=@{t}"));
    }
    if all_branches {
        args.extend(["--branches", "--remotes", "--tags", "HEAD"].map(String::from));
    } else {
        args.push("HEAD".into());
    }
    args.push("--".into());
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(parse(&repo.run(&a)?))
}

/// `src/{old => new}/x.rs` or `old.rs => new.rs` → the new path.
pub(crate) fn renamed(p: &str) -> String {
    if let (Some(a), Some(b)) = (p.find('{'), p.find('}'))
        && a < b
        && let Some((_, new)) = p[a + 1..b].split_once(" => ")
    {
        let joined = format!("{}{}{}", &p[..a], new, &p[b + 1..]);
        // `{old => }` leaves a double slash
        return joined.replace("//", "/");
    }
    p.split_once(" => ").map_or(p, |(_, n)| n).to_string()
}

fn parse(out: &str) -> Vec<CommitStat> {
    out.split('\x1e')
        .filter_map(|rec| {
            let mut lines = rec.lines();
            let head = lines.next()?;
            let f: Vec<&str> = head.split('\x1f').collect();
            if f.len() < 4 {
                return None;
            }
            let changes = lines
                .filter_map(|l| {
                    let mut it = l.splitn(3, '\t');
                    let (a, r, p) = (it.next()?, it.next()?, it.next()?);
                    // binary files: "-  -  path"
                    Some(Change { path: renamed(p), added: a.parse().unwrap_or(0), removed: r.parse().unwrap_or(0) })
                })
                .collect();
            Some(CommitStat { id: f[0].into(), author: f[1].into(), email: f[2].into(), time: f[3].trim().parse().unwrap_or(0), changes })
        })
        .collect()
}

/// Lock files, vendored and generated code: their line counts drown the rest.
pub fn is_generated(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    const NAMES: &[&str] = &[
        "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "npm-shrinkwrap.json", "bun.lockb", "Cargo.lock", "Gemfile.lock", "poetry.lock", "Pipfile.lock",
        "composer.lock", "go.sum", "mix.lock", "pubspec.lock", "Podfile.lock", "packages.lock.json", "flake.lock", "uv.lock", "schema.rb", "structure.sql",
    ];
    const DIRS: &[&str] = &["vendor/", "node_modules/", "third_party/", "Pods/", "__generated__/"];
    NAMES.contains(&name)
        || DIRS.iter().any(|d| path.starts_with(d) || path.contains(&format!("/{d}")))
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".map")
        || name.contains(".generated.")
        || name.ends_with(".pb.go")
        || name.ends_with("_pb2.py")
        || name.ends_with(".snap")
}

/// What to count: inside a folder (or one file), by one person, and whether
/// generated files are left out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub path: Option<String>,
    pub author: Option<String>,
    pub skip_generated: bool,
}

impl Filter {
    fn keeps(&self, c: &Change) -> bool {
        if self.skip_generated && is_generated(&c.path) {
            return false;
        }
        match &self.path {
            Some(p) => c.path == *p || c.path.strip_prefix(p.as_str()).is_some_and(|r| r.starts_with('/')),
            None => true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FolderStat {
    pub commits: usize,
    pub added: u64,
    pub removed: u64,
    /// person → commits touching this folder
    pub people: HashMap<String, usize>,
}

impl FolderStat {
    /// The person with the most commits here, and their share (0–1).
    pub fn owner(&self) -> Option<(&str, f32)> {
        let (who, n) = self.people.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))?;
        Some((who.as_str(), *n as f32 / self.commits.max(1) as f32))
    }
}

#[derive(Debug, Clone, Default)]
pub struct PersonStat {
    pub commits: usize,
    pub added: u64,
    pub removed: u64,
    pub files: usize,
    pub active_days: usize,
    pub first: i64,
    pub last: i64,
    /// Top-level-ish folders (two levels) by commits, most first.
    pub folders: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Default)]
pub struct FileStat {
    pub commits: usize,
    pub added: u64,
    pub removed: u64,
    pub people: Vec<String>,
}

/// Everything the views show, for one filter.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub commits: usize,
    pub added: u64,
    pub removed: u64,
    pub files: usize,
    /// Commits per week, oldest first, ending with the week of `now`.
    pub weeks: Vec<usize>,
    /// Commits by weekday (Monday first) × hour, in local time.
    pub hours: [[usize; 24]; 7],
    /// Every folder (`src`, `src/api`, …) with its numbers.
    pub folders: BTreeMap<String, FolderStat>,
    pub people: Vec<(String, PersonStat)>,
    /// Files by commits, most first.
    pub files_by_commits: Vec<(String, FileStat)>,
}

/// Aggregate `commits` by `filter`. `weeks`: how many weeks the chart shows;
/// `offset`: the local time zone's offset at a given time (seconds).
pub fn aggregate(commits: &[CommitStat], filter: &Filter, now: i64, weeks: usize, offset: &dyn Fn(i64) -> i64) -> Stats {
    let mut s = Stats { weeks: vec![0; weeks.max(1)], ..Default::default() };
    let mut files: HashMap<&str, FileStat> = HashMap::new();
    // per person: the stats, their files, their days, their folders
    type Acc<'a> = (PersonStat, HashSet<&'a str>, HashSet<i64>, HashMap<String, usize>);
    let mut people: HashMap<&str, Acc> = HashMap::new();
    let mut file_people: HashMap<&str, HashSet<&str>> = HashMap::new();
    for c in commits {
        if filter.author.as_deref().is_some_and(|a| a != c.author) {
            continue;
        }
        let kept: Vec<&Change> = c.changes.iter().filter(|ch| filter.keeps(ch)).collect();
        if kept.is_empty() {
            continue;
        }
        s.commits += 1;
        let local = c.time + offset(c.time);
        // weekday: 1970-01-01 was a Thursday (3 with Monday = 0)
        let day = local.div_euclid(86_400);
        let (wd, hour) = ((day + 3).rem_euclid(7) as usize, (local.rem_euclid(86_400) / 3600) as usize);
        s.hours[wd][hour] += 1;
        let age = (now - c.time).max(0) / (7 * 86_400);
        if (age as usize) < s.weeks.len() {
            let i = s.weeks.len() - 1 - age as usize;
            s.weeks[i] += 1;
        }
        let p = people.entry(c.author.as_str()).or_insert_with(|| (PersonStat { first: i64::MAX, ..Default::default() }, HashSet::new(), HashSet::new(), HashMap::new()));
        p.0.commits += 1;
        p.0.first = p.0.first.min(c.time);
        p.0.last = p.0.last.max(c.time);
        p.2.insert(day);
        let mut touched: HashSet<String> = HashSet::new();
        let mut top2: HashSet<String> = HashSet::new();
        for ch in kept {
            let (a, r) = (ch.added as u64, ch.removed as u64);
            s.added += a;
            s.removed += r;
            p.0.added += a;
            p.0.removed += r;
            p.1.insert(ch.path.as_str());
            let f = files.entry(ch.path.as_str()).or_default();
            f.commits += 1;
            f.added += a;
            f.removed += r;
            file_people.entry(ch.path.as_str()).or_default().insert(c.author.as_str());
            // every folder above the file
            let parts: Vec<&str> = ch.path.split('/').collect();
            for i in 1..parts.len() {
                let d = parts[..i].join("/");
                let fs = s.folders.entry(d.clone()).or_default();
                fs.added += a;
                fs.removed += r;
                if touched.insert(d) {
                    fs.commits += 1;
                    *fs.people.entry(c.author.clone()).or_default() += 1;
                }
            }
            if parts.len() > 1 {
                top2.insert(parts[..(parts.len() - 1).min(2)].join("/"));
            }
        }
        for d in top2 {
            *p.3.entry(d).or_default() += 1;
        }
    }
    s.files = files.len();
    s.people = people
        .into_iter()
        .map(|(who, (mut st, fs, days, folders))| {
            st.files = fs.len();
            st.active_days = days.len();
            let mut f: Vec<(String, usize)> = folders.into_iter().collect();
            f.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            st.folders = f;
            (who.to_string(), st)
        })
        .collect();
    s.people.sort_by(|a, b| b.1.commits.cmp(&a.1.commits).then(a.0.cmp(&b.0)));
    let mut fl: Vec<(String, FileStat)> = files
        .into_iter()
        .map(|(p, mut f)| {
            let mut who: Vec<String> = file_people.remove(p).unwrap_or_default().into_iter().map(String::from).collect();
            who.sort();
            f.people = who;
            (p.to_string(), f)
        })
        .collect();
    fl.sort_by(|a, b| b.1.commits.cmp(&a.1.commits).then(b.1.people.len().cmp(&a.1.people.len())).then(a.0.cmp(&b.0)));
    s.files_by_commits = fl;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_and_binaries() {
        assert_eq!(renamed("src/{a => b}/x.rs"), "src/b/x.rs");
        assert_eq!(renamed("src/{old => }/x.rs"), "src/x.rs");
        assert_eq!(renamed("a.rs => b.rs"), "b.rs");
        let out = "\x1eabc\x1fMina\x1fm@x\x1f100\n\n3\t1\tsrc/a.rs\n-\t-\timg.png\n\x1edef\x1fJo\x1fj@x\x1f200\n\n1\t0\tREADME.md\n";
        let c = parse(out);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].changes, [Change { path: "src/a.rs".into(), added: 3, removed: 1 }, Change { path: "img.png".into(), added: 0, removed: 0 }]);
        assert_eq!((c[1].author.as_str(), c[1].time), ("Jo", 200));
    }

    #[test]
    fn generated_files() {
        for p in ["package-lock.json", "web/yarn.lock", "vendor/x/y.go", "app/node_modules/a.js", "dist/app.min.js", "api/schema.generated.ts", "db/schema.rb"] {
            assert!(is_generated(p), "{p}");
        }
        for p in ["src/lock.rs", "src/vendors.rs", "README.md"] {
            assert!(!is_generated(p), "{p}");
        }
    }

    fn ch(p: &str, a: u32, r: u32) -> Change {
        Change { path: p.into(), added: a, removed: r }
    }

    #[test]
    fn folders_people_files_and_time() {
        let day = 86_400;
        // 2026-11-23 is a Monday; 10:00 UTC
        let mon = 1_795_428_000;
        let commits = vec![
            CommitStat { id: "1".into(), author: "mina".into(), email: String::new(), time: mon, changes: vec![ch("src/chart/a.ts", 10, 2), ch("src/chart/b.ts", 5, 0)] },
            CommitStat { id: "2".into(), author: "mina".into(), email: String::new(), time: mon - day, changes: vec![ch("src/chart/a.ts", 1, 1)] },
            CommitStat { id: "3".into(), author: "jo".into(), email: String::new(), time: mon - 8 * day, changes: vec![ch("src/notify/n.ts", 4, 0), ch("package-lock.json", 900, 800)] },
        ];
        let utc = |_: i64| 0;
        let s = aggregate(&commits, &Filter { skip_generated: true, ..Default::default() }, mon, 4, &utc);
        assert_eq!((s.commits, s.added, s.removed, s.files), (3, 20, 3, 3));
        assert_eq!(s.weeks, [0, 0, 1, 2]);
        assert_eq!(s.hours[0][10], 1, "Monday 10:00");
        assert_eq!(s.hours[6][10], 2, "Sundays: a day and 8 days before");
        // a folder counts a commit once however many files it touched
        let chart = &s.folders["src/chart"];
        assert_eq!((chart.commits, chart.added), (2, 16));
        assert_eq!(chart.owner(), Some(("mina", 1.0)));
        assert_eq!(s.folders["src"].commits, 3);
        assert_eq!(s.folders["src"].people.len(), 2);
        assert_eq!(s.people[0].0, "mina");
        assert_eq!((s.people[0].1.files, s.people[0].1.active_days), (2, 2));
        assert_eq!(s.people[0].1.folders[0], ("src/chart".to_string(), 2));
        assert_eq!(s.files_by_commits[0].0, "src/chart/a.ts");
        // with generated files: the lock file's lines come back
        let all = aggregate(&commits, &Filter::default(), mon, 4, &utc);
        assert_eq!(all.added, 920);
        // narrowed to a folder, then to a person
        let notify = aggregate(&commits, &Filter { path: Some("src/notify".into()), skip_generated: true, ..Default::default() }, mon, 4, &utc);
        assert_eq!((notify.commits, notify.people.len()), (1, 1));
        let file = aggregate(&commits, &Filter { path: Some("src/chart/a.ts".into()), ..Default::default() }, mon, 4, &utc);
        assert_eq!(file.commits, 2);
        assert_eq!(aggregate(&commits, &Filter { path: Some("src/char".into()), ..Default::default() }, mon, 4, &utc).commits, 0, "a prefix of a name isn't its folder");
        let jo = aggregate(&commits, &Filter { author: Some("jo".into()), skip_generated: true, ..Default::default() }, mon, 4, &utc);
        assert_eq!((jo.commits, jo.folders.contains_key("src/chart")), (1, false));
    }

    #[test]
    fn reads_a_real_repository() {
        let d = std::env::temp_dir().join(format!("margin-insights-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        let g = |args: &[&str]| {
            let o = std::process::Command::new("git").arg("-C").arg(&d).args(["-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        };
        g(&["init", "-q"]);
        g(&["config", "core.autocrlf", "false"]);
        std::fs::write(d.join("src/a.txt"), "1\n2\n").unwrap();
        g(&["add", "."]);
        g(&["-c", "user.name=Mina", "-c", "user.email=mina@old.example", "commit", "-qm", "a"]);
        std::fs::write(d.join("src/a.txt"), "1\n2\n3\n").unwrap();
        g(&["-c", "user.name=Mina Park", "-c", "user.email=mina@new.example", "commit", "-qam", "b"]);
        g(&["mv", "src/a.txt", "src/b.txt"]);
        g(&["-c", "user.name=Mina Park", "-c", "user.email=mina@new.example", "commit", "-qm", "rename"]);
        // .mailmap: one person under two emails
        std::fs::write(d.join(".mailmap"), "Mina Park <mina@new.example> Mina <mina@old.example>\n").unwrap();
        let repo = Repo::open(&d).unwrap();
        let c = load(&repo, None, false).unwrap();
        assert_eq!(c.len(), 3);
        assert!(c.iter().all(|c| c.author == "Mina Park"), "{c:?}");
        assert_eq!(c[0].changes[0].path, "src/b.txt", "the rename's new name");
        assert!(load(&repo, Some(crate::time::now_unix() + 3600), false).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
