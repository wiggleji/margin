//! Journal of automatic re-attachments, so each one can be undone.
//!
//! Stored as `<notes>/reattach.json`. Every automatic move records what was
//! moved and what the destination held before, so undo can restore both
//! notes exactly. An undone `from → to` pair is remembered and never
//! re-attached automatically again.

use crate::notes::{NoteStore, NoteTarget};
use crate::{Result, bail};
use serde::{Deserialize, Serialize};

const FILE: &str = "reattach.json";
const KEEP: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    /// Applied; can be undone.
    #[default]
    Active,
    /// Reverted by the user.
    Undone,
    /// The user confirmed it ("Keep"); no longer offered for undo.
    Kept,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReattachEntry {
    pub id: String,
    pub from: String,
    pub to: String,
    pub at: i64,
    /// Content that was moved off `from`.
    pub moved_content: String,
    /// What `to` contained before the move (`None`: no note there).
    pub dest_before: Option<String>,
    /// Version of `to` right after the move — detects later edits.
    pub dest_after_version: String,
    #[serde(default)]
    pub state: EntryState,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Journal {
    #[serde(default)]
    entries: Vec<ReattachEntry>,
    /// `(from, to)` pairs the user undid: never auto-apply them again.
    #[serde(default)]
    rejected: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UndoOutcome {
    pub restored: String,
    /// The destination was edited after the move, so it was left as is
    /// (its text may now duplicate the restored note).
    pub kept_edited_destination: bool,
}

fn load(store: &NoteStore) -> Journal {
    std::fs::read_to_string(store.root().join(FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(store: &NoteStore, j: &mut Journal) -> Result<()> {
    if j.entries.len() > KEEP {
        let cut = j.entries.len() - KEEP;
        j.entries.drain(..cut);
    }
    crate::notes::atomic_write(&store.root().join(FILE), serde_json::to_string_pretty(j)?.as_bytes())
}

pub(crate) fn is_rejected(store: &NoteStore, from: &str, to: &str) -> bool {
    load(store).rejected.iter().any(|(f, t)| f == from && t == to)
}

/// Move `from` → `to` and journal it. Returns the entry id.
pub(crate) fn move_recorded(store: &NoteStore, from: &NoteTarget, to: &NoteTarget) -> Result<String> {
    let Some(src) = store.read(from)? else { bail!("note '{from}' does not exist") };
    let dest_before = store.read(to)?.map(|n| n.content);
    let after = store.move_note(from, to)?;
    let now = crate::time::now_unix();
    let fstr = from.to_string();
    let short = fstr.rsplit(':').next().unwrap_or("").chars().take(8).collect::<String>();
    let mut j = load(store);
    // Unique even for several moves of the same note within one second.
    let mut n = j.entries.len();
    let mut id = format!("{now}-{short}-{n}");
    while j.entries.iter().any(|e| e.id == id) {
        n += 1;
        id = format!("{now}-{short}-{n}");
    }
    j.entries.push(ReattachEntry {
        id: id.clone(),
        from: fstr,
        to: to.to_string(),
        at: now,
        moved_content: src.content,
        dest_before,
        dest_after_version: after.version,
        state: EntryState::Active,
    });
    save(store, &mut j)?;
    Ok(id)
}

/// Re-attachments that can still be undone, newest first.
pub fn active(store: &NoteStore) -> Vec<ReattachEntry> {
    let mut v: Vec<_> = load(store).entries.into_iter().filter(|e| e.state == EntryState::Active).collect();
    v.reverse();
    v
}

pub fn undo(store: &NoteStore, id: &str) -> Result<UndoOutcome> {
    let mut j = load(store);
    let Some(e) = j.entries.iter_mut().find(|e| e.id == id) else { bail!("no re-attachment with id {id}") };
    if e.state != EntryState::Active {
        bail!("re-attachment {id} was already {}", if e.state == EntryState::Undone { "undone" } else { "kept" });
    }
    let from = NoteTarget::parse(&e.from)?;
    let to = NoteTarget::parse(&e.to)?;

    // 1) Destination: restore its previous state only if untouched since the move.
    let cur = store.read(&to)?;
    let untouched = cur.as_ref().map(|n| n.version.as_str()).unwrap_or("") == e.dest_after_version;
    if untouched {
        match &e.dest_before {
            Some(prev) => {
                store.write(&to, prev, Some(&e.dest_after_version))?;
            }
            None => store.delete(&to)?,
        }
    }
    // 2) Source: put the moved note back on the old commit.
    match store.read(&from)? {
        None => {
            store.write(&from, &e.moved_content, Some(""))?;
        }
        // Someone wrote a new note there meanwhile: keep both.
        Some(n) => {
            let merged = format!("{}\n\n---\n\n{}\n", n.content.trim_end(), e.moved_content.trim());
            store.write(&from, &merged, Some(&n.version))?;
        }
    }
    e.state = EntryState::Undone;
    let pair = (e.from.clone(), e.to.clone());
    if !j.rejected.contains(&pair) {
        j.rejected.push(pair);
    }
    save(store, &mut j)?;
    Ok(UndoOutcome { restored: from.to_string(), kept_edited_destination: !untouched })
}

/// Confirm a re-attachment: it stops being offered for undo.
pub fn keep(store: &NoteStore, id: &str) -> Result<()> {
    let mut j = load(store);
    let Some(e) = j.entries.iter_mut().find(|e| e.id == id) else { bail!("no re-attachment with id {id}") };
    e.state = EntryState::Kept;
    save(store, &mut j)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_restores_merged_destination_exactly() {
        let d = std::env::temp_dir().join(format!("margin-journal-{}-{}", std::process::id(), crate::time::now_unix()));
        let _ = std::fs::remove_dir_all(&d);
        let s = NoteStore::at(&d).unwrap();
        let (a, b) = (NoteTarget::Commit("aaaa1111".into()), NoteTarget::Commit("bbbb2222".into()));
        s.write(&a, "moved text", None).unwrap();
        s.write(&b, "already here", None).unwrap();

        let id = move_recorded(&s, &a, &b).unwrap();
        assert!(s.read(&a).unwrap().is_none());
        assert!(s.read(&b).unwrap().unwrap().content.contains("moved text"));
        assert_eq!(active(&s).len(), 1);

        let out = undo(&s, &id).unwrap();
        assert!(!out.kept_edited_destination);
        assert_eq!(s.read(&a).unwrap().unwrap().content, "moved text");
        assert_eq!(s.read(&b).unwrap().unwrap().content, "already here");
        assert!(active(&s).is_empty());
        assert!(is_rejected(&s, "commit:aaaa1111", "commit:bbbb2222"));

        // Keep: no longer offered for undo.
        let id2 = move_recorded(&s, &a, &NoteTarget::Commit("cccc3333".into())).unwrap();
        keep(&s, &id2).unwrap();
        assert!(active(&s).is_empty());
        assert!(undo(&s, &id2).is_err());
    }
}
