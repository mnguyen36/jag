//! Per-agent staging area.
//!
//! Each agent has its own `index` (a JSON `path -> {oid, mode}` map) under
//! `.jag/agents/<name>/`. Because indexes are per-agent, two agents can stage
//! different work from the *same* working folder without colliding — the
//! single-index bottleneck that forces Git users to clone is gone.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::repo::Repo;
use crate::tree::flatten_tree;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IndexEntry {
    pub oid: String,
    pub mode: String,
}

pub struct Index {
    pub entries: BTreeMap<String, IndexEntry>,
    path: PathBuf,
}

impl Index {
    pub fn load(repo: &Repo, agent: Option<&str>) -> Result<Index> {
        let path = repo.index_path(agent);
        let entries: BTreeMap<String, IndexEntry> = if path.exists() {
            let data = fs::read_to_string(&path)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        Ok(Index { entries, path })
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, serde_json::to_string_pretty(&self.entries)?)?;
        Ok(())
    }

    pub fn add(&mut self, path: &str, oid: &str, mode: &str) {
        self.entries.insert(
            path.to_string(),
            IndexEntry {
                oid: oid.to_string(),
                mode: mode.to_string(),
            },
        );
    }

    pub fn remove(&mut self, path: &str) {
        self.entries.remove(path);
    }

    /// `path -> (mode, oid)` view, matching the tree/worktree map shape.
    pub fn path_oids(&self) -> BTreeMap<String, (String, String)> {
        self.entries
            .iter()
            .map(|(p, e)| (p.clone(), (e.mode.clone(), e.oid.clone())))
            .collect()
    }
}

/// Replace an agent's index with the full contents of `tree` (used after
/// commit-checkout / reconcile so the index mirrors the lane tip, Git-style).
pub fn seed_index_from_tree(repo: &Repo, agent: &str, tree: Option<&str>) -> Result<()> {
    let mut idx = Index {
        entries: BTreeMap::new(),
        path: repo.index_path(Some(agent)),
    };
    if let Some(t) = tree {
        for (path, (mode, oid)) in flatten_tree(repo, t, "")? {
            idx.add(&path, &oid, &mode);
        }
    }
    idx.save()
}
