//! Commit/object graph walks shared by the sync layer and the server.

use std::collections::BTreeSet;

use anyhow::Result;

use crate::commit::read_commit;
use crate::objects::{read_object, ObjectKind};
use crate::repo::Repo;
use crate::tree::read_tree;

/// Every object reachable from `roots` (commits → parents + tree, trees →
/// entries, blobs are leaves), including the roots themselves. This is the set
/// a push must ensure the remote has, and the set a fetch downloads.
pub fn reachable(repo: &Repo, roots: &[String]) -> Result<BTreeSet<String>> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = roots.to_vec();
    while let Some(oid) = stack.pop() {
        if oid.is_empty() || !seen.insert(oid.clone()) {
            continue;
        }
        let (kind, _) = read_object(repo, &oid)?;
        match kind {
            ObjectKind::Commit => {
                let c = read_commit(repo, &oid)?;
                stack.push(c.tree);
                stack.extend(c.parents);
            }
            ObjectKind::Tree => {
                for (_, e) in read_tree(repo, &oid)? {
                    stack.push(e.oid);
                }
            }
            ObjectKind::Blob => {}
        }
    }
    Ok(seen)
}

/// Whether `anc` is an ancestor of (or equal to) `desc` in the commit graph.
/// Drives fast-forward checks for push/fetch. Requires the relevant commits to
/// be present locally (push uploads objects before the ref update).
pub fn is_ancestor(repo: &Repo, anc: &str, desc: &str) -> Result<bool> {
    if anc == desc {
        return Ok(true);
    }
    let mut seen = BTreeSet::new();
    let mut stack = vec![desc.to_string()];
    while let Some(oid) = stack.pop() {
        if !seen.insert(oid.clone()) {
            continue;
        }
        let c = read_commit(repo, &oid)?;
        for p in c.parents {
            if p == anc {
                return Ok(true);
            }
            stack.push(p);
        }
    }
    Ok(false)
}
