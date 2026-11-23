//! "Viewed" marks in review mode, like GitHub's: per comparison (a pull
//! request, or base...head), which files you've looked at. Each mark keeps a
//! fingerprint of the file's changes, so a file that changes again after you
//! viewed it shows as unviewed. Stored in `<git-common-dir>/margin/review.json`
//! (shared by worktrees, never committed).

use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Comparisons kept (most recently used first); older ones are forgotten.
const KEEP: usize = 50;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Viewed {
    /// comparison key → (path → fingerprint of its changes when viewed)
    #[serde(default)]
    marks: BTreeMap<String, BTreeMap<String, String>>,
    /// Comparison keys, most recently used first.
    #[serde(default)]
    recent: Vec<String>,
}

/// The key for a pull request, or for comparing two branches.
pub fn key(pr: Option<u64>, base: &str, head: &str) -> String {
    match pr {
        Some(n) => format!("pr:{n}"),
        None => format!("cmp:{base}...{head}"),
    }
}

/// A fingerprint of a file's changes (its patch text).
pub fn fingerprint(patch: &str) -> String {
    let h = patch.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    format!("{h:016x}")
}

fn path(common_dir: &Path) -> PathBuf {
    common_dir.join("margin").join("review.json")
}

impl Viewed {
    pub fn load(common_dir: &Path) -> Viewed {
        std::fs::read_to_string(path(common_dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, common_dir: &Path) -> Result<()> {
        let p = path(common_dir);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        std::fs::rename(tmp, p)?;
        Ok(())
    }

    /// Was `file` viewed in this comparison, with these same changes?
    pub fn is_viewed(&self, key: &str, file: &str, fingerprint: &str) -> bool {
        self.marks.get(key).and_then(|m| m.get(file)).is_some_and(|f| f == fingerprint)
    }

    /// Mark `file` viewed (with its current changes) or not.
    pub fn set(&mut self, key: &str, file: &str, fingerprint: &str, viewed: bool) {
        let m = self.marks.entry(key.to_string()).or_default();
        if viewed {
            m.insert(file.to_string(), fingerprint.to_string());
        } else {
            m.remove(file);
        }
        self.recent.retain(|k| k != key);
        self.recent.insert(0, key.to_string());
        for old in self.recent.split_off(self.recent.len().min(KEEP)) {
            self.marks.remove(&old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewed_until_the_file_changes() {
        let d = std::env::temp_dir().join(format!("margin-viewed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let k = key(Some(12), "main", "feat/x");
        let (a, b) = (fingerprint("@@ -1 +1 @@\n-a\n+b\n"), fingerprint("@@ -1 +1 @@\n-a\n+c\n"));
        let mut v = Viewed::load(&d);
        v.set(&k, "src/x.rs", &a, true);
        v.save(&d).unwrap();
        let v = Viewed::load(&d);
        assert!(v.is_viewed(&k, "src/x.rs", &a));
        assert!(!v.is_viewed(&k, "src/x.rs", &b), "changed since: unviewed again");
        assert!(!v.is_viewed(&key(None, "main", "feat/x"), "src/x.rs", &a), "another comparison");
        let mut v = v;
        v.set(&k, "src/x.rs", &a, false);
        assert!(!v.is_viewed(&k, "src/x.rs", &a));
        // only the most recent comparisons are kept
        for i in 0..(KEEP + 5) {
            v.set(&format!("cmp:{i}"), "f", "x", true);
        }
        assert_eq!(v.marks.len(), KEEP);
        let _ = std::fs::remove_dir_all(&d);
    }
}
