//! Commit-graph lane layout.
//!
//! Input: commits in topo order (children before parents).
//! Output per row: the commit's column + the edges entering the row from
//! the row above. The renderer draws each row independently (so it works
//! with virtualised scrolling): top half = this row's `edges_up`,
//! bottom half = next row's `edges_up`.

use crate::git::Commit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    /// Column in the row above.
    pub from: u16,
    /// Column in this row.
    pub to: u16,
    pub color: u16,
}

#[derive(Debug, Clone, Default)]
pub struct GraphRow {
    pub col: u16,
    pub color: u16,
    pub edges_up: Vec<Edge>,
}

#[derive(Clone)]
struct Lane {
    id: String,
    color: u16,
    from: u16,
}

pub fn layout(commits: &[Commit]) -> (Vec<GraphRow>, usize) {
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let mut next_color: u16 = 0;
    let mut max_width = 1;

    for c in commits {
        let matches: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_ref().is_some_and(|l| l.id == c.id))
            .map(|(i, _)| i)
            .collect();

        let col = match matches.first() {
            Some(&i) => i,
            None => free_slot(&mut lanes),
        };
        let color = match matches.first() {
            Some(&i) => lanes[i].as_ref().unwrap().color,
            None => {
                next_color = next_color.wrapping_add(1);
                next_color
            }
        };

        let mut edges_up = Vec::new();
        for (j, l) in lanes.iter().enumerate() {
            if let Some(l) = l {
                let to = if l.id == c.id { col } else { j };
                edges_up.push(Edge { from: l.from, to: to as u16, color: l.color });
            }
        }

        for &j in &matches {
            lanes[j] = None;
        }
        for (j, l) in lanes.iter_mut().enumerate() {
            if let Some(l) = l {
                l.from = j as u16;
            }
        }

        // Ensure slot exists (free_slot may have pushed).
        if lanes.len() <= col {
            lanes.resize(col + 1, None);
        }
        for (k, p) in c.parents.iter().enumerate() {
            if k == 0 {
                lanes[col] = Some(Lane { id: p.clone(), color, from: col as u16 });
            } else {
                // Extra parents get their own lane; lanes that reach the same
                // commit converge when that commit is laid out.
                let slot = free_slot(&mut lanes);
                next_color = next_color.wrapping_add(1);
                lanes[slot] = Some(Lane { id: p.clone(), color: next_color, from: col as u16 });
            }
        }
        while matches!(lanes.last(), Some(None)) {
            lanes.pop();
        }
        max_width = max_width.max(lanes.len()).max(col + 1);
        rows.push(GraphRow { col: col as u16, color, edges_up });
    }
    (rows, max_width)
}

fn free_slot(lanes: &mut Vec<Option<Lane>>) -> usize {
    if let Some(i) = lanes.iter().position(Option::is_none) {
        i
    } else {
        lanes.push(None);
        lanes.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, parents: &[&str]) -> Commit {
        Commit {
            id: id.into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            author: String::new(),
            email: String::new(),
            time: 0,
            refs: vec![],
            subject: String::new(),
        }
    }

    #[test]
    fn merge_diamond() {
        // d merges b and c; both come from a.
        let commits = vec![c("d", &["b", "c"]), c("c", &["a"]), c("b", &["a"]), c("a", &[])];
        let (rows, w) = layout(&commits);
        assert_eq!(rows[0].col, 0);
        assert_eq!(rows[1].col, 1); // c on second lane
        assert_eq!(rows[2].col, 0); // b on first lane
        assert_eq!(rows[3].col, 0); // a: both lanes converge
        assert!(rows[3].edges_up.iter().any(|e| e.from == 1 && e.to == 0));
        assert_eq!(w, 2);
    }
}
