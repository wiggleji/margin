//! Group branch names by prefix for the sidebar:
//! `feature/a`, `feature/b`, `hotfix/x`, `projectA/y` →
//!
//! ```text
//! ▾ feature/   2
//!     a
//!     b
//!   hotfix/x          (only one "hotfix/" branch: stays flat)
//!   projectA/y
//! ```
//!
//! A prefix becomes a group only when 2+ branches share it, recursively at
//! every level. A group whose only child is another group is merged into it
//! ("projectA/feature/"), like compact folders in an editor.
//!
//! Order: the top level is alphabetical; inside a group, entries keep the
//! input order — callers pass branches most-recently-committed first, so
//! groups list recent work first (a sub-group sits where its newest branch is).

pub enum Node<T> {
    Leaf {
        /// Name shown at this level (the part after the enclosing groups).
        label: String,
        item: T,
    },
    Group {
        /// e.g. "feature/" or merged "projectA/feature/"
        label: String,
        /// Full prefix from the root, e.g. "projectA/feature/" (stable id).
        path: String,
        children: Vec<Node<T>>,
        /// Number of branches below.
        count: usize,
    },
}

/// `items` in recency order (newest first).
pub fn build<T>(items: Vec<(String, T)>) -> Vec<Node<T>> {
    let ranked = items.into_iter().enumerate().map(|(i, (n, t))| (n, t, i)).collect();
    build_level(ranked, "").into_iter().map(|(n, _)| n).collect()
}

/// Returns nodes with their rank (input position of their newest member).
fn build_level<T>(items: Vec<(String, T, usize)>, parent: &str) -> Vec<(Node<T>, usize)> {
    // Bucket by first path segment (only names that have a '/').
    let mut buckets: Vec<(String, Vec<(String, T, usize)>)> = Vec::new();
    let mut leaves: Vec<(String, T, usize)> = Vec::new();
    for (name, item, rank) in items {
        match name.split_once('/') {
            Some((seg, _)) if !seg.is_empty() => match buckets.iter_mut().find(|(s, _)| s == seg) {
                Some((_, v)) => v.push((name, item, rank)),
                None => buckets.push((seg.to_string(), vec![(name, item, rank)])),
            },
            _ => leaves.push((name, item, rank)),
        }
    }
    let mut out: Vec<(Node<T>, usize)> = leaves.into_iter().map(|(label, item, r)| (Node::Leaf { label, item }, r)).collect();
    for (seg, group) in buckets {
        if group.len() < 2 {
            // Single branch with this prefix: keep its full name, flat.
            out.extend(group.into_iter().map(|(label, item, r)| (Node::Leaf { label, item }, r)));
            continue;
        }
        let path = format!("{parent}{seg}/");
        let count = group.len();
        let rank = group.iter().map(|g| g.2).min().unwrap_or(0);
        let rest: Vec<(String, T, usize)> = group.into_iter().map(|(n, i, r)| (n[seg.len() + 1..].to_string(), i, r)).collect();
        let mut children: Vec<Node<T>> = build_level(rest, &path).into_iter().map(|(n, _)| n).collect();
        let mut label = format!("{seg}/");
        let mut path = path;
        // Compact a chain of single sub-groups: "projectA/" › "feature/" → "projectA/feature/".
        while children.len() == 1 && matches!(children[0], Node::Group { .. }) {
            if let Some(Node::Group { label: l, path: p, children: c, .. }) = children.pop() {
                label.push_str(&l);
                path = p;
                children = c;
            }
        }
        out.push((Node::Group { label, path, children, count }, rank));
    }
    if parent.is_empty() {
        // Top level: alphabetical, case-insensitive (groups and branches interleaved).
        out.sort_by_key(|(n, _)| match n {
            Node::Leaf { label, .. } | Node::Group { label, .. } => label.to_lowercase(),
        });
    } else {
        // Inside a group: most recent first.
        out.sort_by_key(|(_, r)| *r);
    }
    out
}

/// Does this group (recursively) contain an item matching `pred`?
pub fn any<T>(nodes: &[Node<T>], pred: &dyn Fn(&T) -> bool) -> bool {
    nodes.iter().any(|n| match n {
        Node::Leaf { item, .. } => pred(item),
        Node::Group { children, .. } => any(children, pred),
    })
}

/// Every item below `nodes`, in tree order.
pub fn items<T>(nodes: &[Node<T>]) -> Vec<&T> {
    let mut out = vec![];
    for n in nodes {
        match n {
            Node::Leaf { item, .. } => out.push(item),
            Node::Group { children, .. } => out.extend(items(children)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(nodes: &[Node<&str>], depth: usize, out: &mut Vec<String>) {
        for n in nodes {
            match n {
                Node::Leaf { label, item } => out.push(format!("{}{label} [{item}]", "  ".repeat(depth))),
                Node::Group { label, children, count, .. } => {
                    out.push(format!("{}{label} ({count})", "  ".repeat(depth)));
                    render(children, depth + 1, out);
                }
            }
        }
    }

    fn tree(names: &[&'static str]) -> Vec<String> {
        let nodes = build(names.iter().map(|n| (n.to_string(), *n)).collect());
        let mut out = vec![];
        render(&nodes, 0, &mut out);
        out
    }

    #[test]
    fn groups_only_shared_prefixes() {
        assert_eq!(
            tree(&["main", "develop", "feature/login", "feature/Billing", "hotfix/1.0.1", "projectA/x", "release/2.0", "release/2.1"]),
            vec![
                "develop [develop]",
                "feature/ (2)",
                // inside a group: input (recency) order, not alphabetical
                "  login [feature/login]",
                "  Billing [feature/Billing]",
                "hotfix/1.0.1 [hotfix/1.0.1]",
                "main [main]",
                "projectA/x [projectA/x]",
                "release/ (2)",
                "  2.0 [release/2.0]",
                "  2.1 [release/2.1]",
            ]
        );
    }

    #[test]
    fn subgroup_sits_at_its_newest_member() {
        // newest first: a leaf, then the sub-group's newest member, then older leaf
        assert_eq!(
            tree(&["proj/z-new", "proj/sub/b", "proj/old", "proj/sub/a"]),
            vec!["proj/ (4)", "  z-new [proj/z-new]", "  sub/ (2)", "    b [proj/sub/b]", "    a [proj/sub/a]", "  old [proj/old]"]
        );
    }

    #[test]
    fn nests_and_compacts() {
        assert_eq!(
            tree(&["projectA/feature/a", "projectA/feature/b", "projectA/hotfix/c", "projectB/feature/x", "projectB/feature/y"]),
            vec![
                "projectA/ (3)",
                "  feature/ (2)",
                "    a [projectA/feature/a]",
                "    b [projectA/feature/b]",
                "  hotfix/c [projectA/hotfix/c]",
                // only one sub-group below projectB → merged label
                "projectB/feature/ (2)",
                "  x [projectB/feature/x]",
                "  y [projectB/feature/y]",
            ]
        );
    }
}
