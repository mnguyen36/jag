//! Per-lane undo/redo journal.
//!
//! Every tip-moving operation (commit, reconcile) appends to a lane's journal.
//! `undo`/`redo` then walk a single pointer along that timeline — editor-style,
//! not git's reset/revert/reflog tangle. Making a new change after an undo
//! truncates the redo tail, exactly like undo in a text editor.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::repo::Repo;

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub oid: String,
    pub label: String,
    pub time: i64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Journal {
    pub history: Vec<Entry>,
    pub position: usize,
}

/// The result of an undo/redo move.
pub struct Step {
    pub new_tip: String,
    pub label: String,
}

fn path(repo: &Repo, lane: &str) -> PathBuf {
    repo.jagdir().join("journal").join(lane.replace('/', "~"))
}

fn load(repo: &Repo, lane: &str) -> Result<Journal> {
    let p = path(repo, lane);
    if !p.exists() {
        return Ok(Journal::default());
    }
    Ok(serde_json::from_str(&fs::read_to_string(&p)?).unwrap_or_default())
}

fn save(repo: &Repo, lane: &str, j: &Journal) -> Result<()> {
    let p = path(repo, lane);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&p, serde_json::to_string_pretty(j)?)?;
    Ok(())
}

/// Append a new tip, dropping any redo tail (a new branch of history).
pub fn record(repo: &Repo, lane: &str, oid: &str, label: &str, time: i64) -> Result<()> {
    let mut j = load(repo, lane)?;
    if !j.history.is_empty() {
        j.history.truncate(j.position + 1);
    }
    j.history.push(Entry {
        oid: oid.to_string(),
        label: label.to_string(),
        time,
    });
    j.position = j.history.len() - 1;
    save(repo, lane, &j)
}

/// Step back one entry. `None` if already at the oldest recorded tip.
pub fn undo(repo: &Repo, lane: &str) -> Result<Option<Step>> {
    let mut j = load(repo, lane)?;
    if j.history.is_empty() || j.position == 0 {
        return Ok(None);
    }
    let undone = j.history[j.position].label.clone();
    j.position -= 1;
    let new_tip = j.history[j.position].oid.clone();
    save(repo, lane, &j)?;
    Ok(Some(Step {
        new_tip,
        label: undone,
    }))
}

/// Step forward one entry. `None` if there is nothing to redo.
pub fn redo(repo: &Repo, lane: &str) -> Result<Option<Step>> {
    let mut j = load(repo, lane)?;
    if j.history.is_empty() || j.position + 1 >= j.history.len() {
        return Ok(None);
    }
    j.position += 1;
    let e = j.history[j.position].clone();
    save(repo, lane, &j)?;
    Ok(Some(Step {
        new_tip: e.oid,
        label: e.label,
    }))
}
