//! Reconciliation: bring concurrent agent lanes back together.
//!
//! This is JAG's answer to "clone N times, then merge". An N-way, path-level
//! merge over lane tip trees:
//!
//! * a path changed by exactly one source (relative to the target) is taken
//!   automatically — non-overlapping work just flows in;
//! * a path changed differently by two or more sources is *contention* and is
//!   surfaced rather than silently resolved.
//!
//! `find_contention` runs the same comparison live (including uncommitted,
//! staged work) so agents can see collisions before they commit.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::agent::list_agents;
use crate::commit::{commit_tree, write_commit};
use crate::index::Index;
use crate::refs::{list_lanes, read_lane, write_lane};
use crate::repo::Repo;
use crate::tree::{build_tree_from_paths, flatten_tree};

pub struct ReconcilePlan {
    pub into: String,
    pub base_tip: Option<String>,
    pub sources: Vec<String>,
    pub merged: BTreeMap<String, (String, String)>,
    /// path -> (source -> Some(oid) changed / None deleted)
    pub conflicts: BTreeMap<String, BTreeMap<String, Option<String>>>,
}

fn lane_paths(
    repo: &Repo,
    lane: &str,
) -> Result<(BTreeMap<String, (String, String)>, Option<String>)> {
    let tip = read_lane(repo, lane)?;
    let tree = commit_tree(repo, tip.as_deref())?;
    let paths = match tree {
        Some(t) => flatten_tree(repo, &t, "")?,
        None => BTreeMap::new(),
    };
    Ok((paths, tip))
}

pub fn plan_reconcile(repo: &Repo, into: &str, sources: Vec<String>) -> Result<ReconcilePlan> {
    let (base_paths, base_tip) = lane_paths(repo, into)?;
    let sources: Vec<String> = if sources.is_empty() {
        // Default to local agent lanes only; remote-tracking lanes (which carry
        // a `/` in their name) must be named explicitly.
        list_lanes(repo)?
            .into_keys()
            .filter(|l| l != into && !l.contains('/'))
            .collect()
    } else {
        sources
    };

    // path -> (source -> Some(oid) | None=deleted), only where it differs from base
    let mut changes: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for lane in &sources {
        let (paths, _) = lane_paths(repo, lane)?;
        for (path, (_, oid)) in &paths {
            let base = base_paths.get(path).map(|(_, o)| o.as_str());
            if base != Some(oid.as_str()) {
                changes
                    .entry(path.clone())
                    .or_default()
                    .insert(lane.clone(), Some(oid.clone()));
            }
        }
        for path in base_paths.keys() {
            if !paths.contains_key(path) {
                changes
                    .entry(path.clone())
                    .or_default()
                    .insert(lane.clone(), None);
            }
        }
    }

    let mut merged = base_paths;
    let mut conflicts = BTreeMap::new();
    for (path, srcvals) in &changes {
        let distinct: BTreeSet<Option<String>> = srcvals.values().cloned().collect();
        if distinct.len() == 1 {
            match distinct.into_iter().next().unwrap() {
                None => {
                    merged.remove(path);
                }
                Some(oid) => {
                    merged.insert(path.clone(), ("100644".to_string(), oid));
                }
            }
        } else {
            conflicts.insert(path.clone(), srcvals.clone());
        }
    }

    Ok(ReconcilePlan {
        into: into.to_string(),
        base_tip,
        sources,
        merged,
        conflicts,
    })
}

pub fn apply_reconcile(
    repo: &Repo,
    plan: &ReconcilePlan,
    message: Option<&str>,
    agent: &str,
    time: i64,
) -> Result<String> {
    let tree = build_tree_from_paths(repo, &plan.merged)?;
    let mut parents = Vec::new();
    if let Some(bt) = &plan.base_tip {
        parents.push(bt.clone());
    }
    for lane in &plan.sources {
        if let Some(tip) = read_lane(repo, lane)? {
            parents.push(tip);
        }
    }
    let default_msg = format!("reconcile {} into {}", plan.sources.join(", "), plan.into);
    let msg = message.unwrap_or(&default_msg);
    let oid = write_commit(repo, &tree, &parents, agent, Some(&plan.into), msg, time)?;
    write_lane(repo, &plan.into, &oid)?;
    Ok(oid)
}

/// Paths that two or more producers (committed lanes and/or live staged agent
/// indexes) have changed to differing content, relative to `main`.
pub fn find_contention(repo: &Repo) -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let lanes = list_lanes(repo)?;
    let base = match lanes.get("main") {
        Some(tip) => {
            let t = commit_tree(repo, Some(tip.as_str()))?.unwrap();
            flatten_tree(repo, &t, "")?
        }
        None => BTreeMap::new(),
    };

    let mut by_path: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (lane, tip) in &lanes {
        if lane == "main" {
            continue;
        }
        let t = commit_tree(repo, Some(tip.as_str()))?.unwrap();
        for (path, (_, oid)) in flatten_tree(repo, &t, "")? {
            if base.get(&path).map(|(_, o)| o.as_str()) != Some(oid.as_str()) {
                by_path
                    .entry(path)
                    .or_default()
                    .insert(format!("lane:{lane}"), oid);
            }
        }
    }
    for agent in list_agents(repo)? {
        let idx = Index::load(repo, Some(&agent))?;
        for (path, e) in &idx.entries {
            if base.get(path).map(|(_, o)| o.as_str()) != Some(e.oid.as_str()) {
                by_path
                    .entry(path.clone())
                    .or_default()
                    .insert(format!("agent:{agent}"), e.oid.clone());
            }
        }
    }

    let mut contended = BTreeMap::new();
    for (path, producers) in by_path {
        let distinct: BTreeSet<&String> = producers.values().collect();
        if distinct.len() > 1 {
            contended.insert(path, producers);
        }
    }
    Ok(contended)
}
