//! Commit objects.
//!
//! A commit records which agent authored it and which lane it belongs to, in
//! addition to the usual tree + parents. That authorship metadata is what lets
//! `reconcile` and `contention` reason about *who* changed *what* concurrently.
//!
//! Text format:
//! ```text
//! tree <oid>
//! parent <oid>      (zero or more; >1 means a reconcile/merge)
//! agent <name>
//! lane <name>
//! time <unix-seconds>
//!
//! <message>
//! ```

use anyhow::{bail, Result};

use crate::objects::{hash_object, read_object, ObjectKind};
use crate::repo::Repo;

#[derive(Debug, Clone)]
pub struct Commit {
    /// The commit's own oid (kept for callers that traverse the graph).
    #[allow(dead_code)]
    pub oid: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub agent: String,
    pub lane: Option<String>,
    pub time: i64,
    pub message: String,
}

pub fn write_commit(
    repo: &Repo,
    tree: &str,
    parents: &[String],
    agent: &str,
    lane: Option<&str>,
    message: &str,
    time: i64,
) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!("tree {tree}\n"));
    for p in parents {
        if !p.is_empty() {
            out.push_str(&format!("parent {p}\n"));
        }
    }
    out.push_str(&format!("agent {agent}\n"));
    if let Some(l) = lane {
        out.push_str(&format!("lane {l}\n"));
    }
    out.push_str(&format!("time {time}\n"));
    out.push('\n');
    out.push_str(message);
    if !message.ends_with('\n') {
        out.push('\n');
    }
    hash_object(repo, out.as_bytes(), ObjectKind::Commit, true)
}

pub fn read_commit(repo: &Repo, oid: &str) -> Result<Commit> {
    let (kind, data) = read_object(repo, oid)?;
    if kind != ObjectKind::Commit {
        bail!("object {oid} is not a commit");
    }
    let text = String::from_utf8(data)?;
    let (header, message) = text.split_once("\n\n").unwrap_or((text.as_str(), ""));

    let mut tree = String::new();
    let mut parents = Vec::new();
    let mut agent = String::new();
    let mut lane = None;
    let mut time = 0i64;
    for line in header.lines() {
        let (k, v) = line.split_once(' ').unwrap_or((line, ""));
        match k {
            "tree" => tree = v.to_string(),
            "parent" => parents.push(v.to_string()),
            "agent" => agent = v.to_string(),
            "lane" => lane = Some(v.to_string()),
            "time" => time = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    Ok(Commit {
        oid: oid.to_string(),
        tree,
        parents,
        agent,
        lane,
        time,
        message: message.to_string(),
    })
}

/// The tree oid of a commit, if any.
pub fn commit_tree(repo: &Repo, oid: Option<&str>) -> Result<Option<String>> {
    match oid {
        None => Ok(None),
        Some(o) => Ok(Some(read_commit(repo, o)?.tree)),
    }
}
