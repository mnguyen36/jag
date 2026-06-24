//! Working-tree status for a single agent: HEAD vs index vs working tree.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::commit::commit_tree;
use crate::index::Index;
use crate::refs::head_commit;
use crate::repo::Repo;
use crate::tree::flatten_tree;
use crate::worktree::{hash_file, walk_worktree};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    New,
    Modified,
    Deleted,
}

impl Change {
    pub fn label(&self) -> &'static str {
        match self {
            Change::New => "new",
            Change::Modified => "modified",
            Change::Deleted => "deleted",
        }
    }
}

pub struct Status {
    pub staged: BTreeMap<String, Change>,
    pub unstaged: BTreeMap<String, Change>,
    pub untracked: Vec<String>,
}

pub fn compute_status(repo: &Repo, agent: &str) -> Result<Status> {
    let idx = Index::load(repo, Some(agent))?;
    let index_entries = idx.path_oids();

    let hc = head_commit(repo, Some(agent))?;
    let tree = commit_tree(repo, hc.as_deref())?;
    let head_entries = match tree {
        Some(t) => flatten_tree(repo, &t, "")?,
        None => BTreeMap::new(),
    };

    let wt = walk_worktree(repo)?;
    let mut wt_oids: BTreeMap<String, (String, String)> = BTreeMap::new();
    for (path, full) in &wt {
        wt_oids.insert(path.clone(), ("100644".to_string(), hash_file(repo, full, false)?));
    }

    // staged: HEAD tree vs index
    let mut staged = BTreeMap::new();
    let mut keys: BTreeSet<&String> = head_entries.keys().collect();
    keys.extend(index_entries.keys());
    for k in keys {
        match (head_entries.get(k), index_entries.get(k)) {
            (None, Some(_)) => {
                staged.insert(k.clone(), Change::New);
            }
            (Some(_), None) => {
                staged.insert(k.clone(), Change::Deleted);
            }
            (Some(h), Some(i)) if h.1 != i.1 => {
                staged.insert(k.clone(), Change::Modified);
            }
            _ => {}
        }
    }

    // unstaged: index vs working tree
    let mut unstaged = BTreeMap::new();
    let mut keys2: BTreeSet<&String> = index_entries.keys().collect();
    keys2.extend(wt_oids.keys());
    for k in keys2 {
        match (index_entries.get(k), wt_oids.get(k)) {
            (Some(_), None) => {
                unstaged.insert(k.clone(), Change::Deleted);
            }
            (Some(i), Some(w)) if i.1 != w.1 => {
                unstaged.insert(k.clone(), Change::Modified);
            }
            _ => {}
        }
    }

    let untracked = wt_oids
        .keys()
        .filter(|k| !index_entries.contains_key(*k))
        .cloned()
        .collect();

    Ok(Status {
        staged,
        unstaged,
        untracked,
    })
}
