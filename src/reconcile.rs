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
use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::agent::list_agents;
use crate::commit::{commit_tree, read_commit, write_commit};
use crate::graph::merge_base;
use crate::index::Index;
use crate::objects::{hash_object, read_object, ObjectKind};
use crate::refs::{list_lanes, read_lane, write_lane};
use crate::repo::Repo;
use crate::tree::{build_tree_from_paths, flatten_tree};

pub struct ReconcilePlan {
    pub into: String,
    pub base_tip: Option<String>,
    pub sources: Vec<String>,
    pub merged: BTreeMap<String, (String, String)>,
    /// Paths combined automatically by a 3-way line merge.
    pub automerged: Vec<String>,
    /// Paths with overlapping edits that need manual resolution.
    pub conflicts: BTreeMap<String, ConflictInfo>,
}

/// A path that could not be merged automatically.
pub struct ConflictInfo {
    /// source lane -> oid changed (or None = deleted), for reporting.
    pub producers: BTreeMap<String, Option<String>>,
    /// Conflict-marked file content to drop in the working tree when the file is
    /// text with overlapping hunks. `None` for binary or add/delete conflicts.
    pub marked: Option<String>,
}

/// A reconcile that paused on conflicts, recorded at `.jag/MERGE` so the next
/// commit on the target lane becomes a proper merge commit.
#[derive(Serialize, Deserialize)]
pub struct MergeState {
    pub into: String,
    pub parents: Vec<String>,
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
    let mut conflicts: BTreeMap<String, ConflictInfo> = BTreeMap::new();
    let mut automerged: Vec<String> = Vec::new();
    for (path, srcvals) in &changes {
        let distinct: BTreeSet<Option<String>> = srcvals.values().cloned().collect();
        if distinct.len() == 1 {
            // Only one source changed this path — take it as-is.
            match distinct.into_iter().next().unwrap() {
                None => {
                    merged.remove(path);
                }
                Some(oid) => {
                    merged.insert(path.clone(), ("100644".to_string(), oid));
                }
            }
            continue;
        }

        // Two or more sources changed this path. Try a 3-way line merge when
        // exactly two text versions disagree (the common case: two agents
        // editing the same file). Anything else stays a conflict.
        let has_delete = srcvals.values().any(|v| v.is_none());
        let changed: Vec<String> = srcvals
            .values()
            .filter_map(|v| v.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if has_delete || changed.len() != 2 {
            conflicts.insert(
                path.clone(),
                ConflictInfo {
                    producers: srcvals.clone(),
                    marked: None,
                },
            );
            continue;
        }

        let lane_a = source_for(srcvals, &changed[0]);
        let lane_b = source_for(srcvals, &changed[1]);
        let tip_a = read_lane(repo, &lane_a)?;
        let tip_b = read_lane(repo, &lane_b)?;
        let base_oid = base_path_oid(repo, tip_a.as_deref(), tip_b.as_deref(), path)?;

        let a_txt = blob_text(repo, &changed[0])?;
        let b_txt = blob_text(repo, &changed[1])?;
        let o_txt = match &base_oid {
            Some(o) => blob_text(repo, o)?,
            None => Some(String::new()),
        };
        let (Some(a_txt), Some(b_txt), Some(o_txt)) = (a_txt, b_txt, o_txt) else {
            // binary on some side — cannot line-merge
            conflicts.insert(
                path.clone(),
                ConflictInfo {
                    producers: srcvals.clone(),
                    marked: None,
                },
            );
            continue;
        };

        let (lines, n_conflicts) = diff3(
            &split_lines(&o_txt),
            &split_lines(&a_txt),
            &split_lines(&b_txt),
            &lane_a,
            &lane_b,
        );
        let content = lines.join("\n");
        if n_conflicts == 0 {
            let oid = hash_object(repo, content.as_bytes(), ObjectKind::Blob, true)?;
            merged.insert(path.clone(), ("100644".to_string(), oid));
            automerged.push(path.clone());
        } else {
            conflicts.insert(
                path.clone(),
                ConflictInfo {
                    producers: srcvals.clone(),
                    marked: Some(content),
                },
            );
        }
    }

    Ok(ReconcilePlan {
        into: into.to_string(),
        base_tip,
        sources,
        merged,
        automerged,
        conflicts,
    })
}

fn source_for(srcvals: &BTreeMap<String, Option<String>>, oid: &str) -> String {
    srcvals
        .iter()
        .find(|(_, v)| v.as_deref() == Some(oid))
        .map(|(s, _)| s.clone())
        .unwrap_or_default()
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
    let _ = clear_merge_state(repo);
    Ok(oid)
}

// --- merge-in-progress state (.jag/MERGE) --------------------------------
fn merge_state_path(repo: &Repo) -> PathBuf {
    repo.jagdir().join("MERGE")
}

pub fn write_merge_state(repo: &Repo, into: &str, parents: &[String]) -> Result<()> {
    let state = MergeState {
        into: into.to_string(),
        parents: parents.to_vec(),
    };
    fs::write(merge_state_path(repo), serde_json::to_string_pretty(&state)?)?;
    Ok(())
}

pub fn read_merge_state(repo: &Repo) -> Option<MergeState> {
    let data = fs::read_to_string(merge_state_path(repo)).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn clear_merge_state(repo: &Repo) -> Result<()> {
    let p = merge_state_path(repo);
    if p.exists() {
        fs::remove_file(p)?;
    }
    Ok(())
}

// --- 3-way line merge ----------------------------------------------------
/// Blob contents as text, or `None` if it looks binary.
fn blob_text(repo: &Repo, oid: &str) -> Result<Option<String>> {
    let (_, data) = read_object(repo, oid)?;
    if data.iter().take(8000).any(|&b| b == 0) {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&data).into_owned()))
}

/// Split keeping the structure exactly (so `join("\n")` round-trips, including a
/// trailing newline, which shows up as a final empty element).
fn split_lines(s: &str) -> Vec<String> {
    s.split('\n').map(|l| l.to_string()).collect()
}

/// The version of `path` at the merge base (common ancestor) of two source
/// tips — the base for the 3-way merge.
fn base_path_oid(
    repo: &Repo,
    tip_a: Option<&str>,
    tip_b: Option<&str>,
    path: &str,
) -> Result<Option<String>> {
    let base = match (tip_a, tip_b) {
        (Some(a), Some(b)) => merge_base(repo, a, b)?,
        _ => None,
    };
    let Some(base) = base else {
        return Ok(None);
    };
    let tree = read_commit(repo, &base)?.tree;
    Ok(flatten_tree(repo, &tree, "")?
        .get(path)
        .map(|(_, o)| o.clone()))
}

/// Longest-common-subsequence index pairs `(i, j)` where `a[i] == b[j]`.
fn lcs_pairs(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// diff3 three-way merge of base `o` with sides `a`/`b`. Returns the merged
/// lines and the number of conflict regions; conflicts are emitted with
/// git-style markers labelled by lane.
fn diff3(o: &[String], a: &[String], b: &[String], la: &str, lb: &str) -> (Vec<String>, usize) {
    let map_a: BTreeMap<usize, usize> = lcs_pairs(o, a).into_iter().collect();
    let map_b: BTreeMap<usize, usize> = lcs_pairs(o, b).into_iter().collect();
    let anchors: Vec<usize> = (0..o.len())
        .filter(|i| map_a.contains_key(i) && map_b.contains_key(i))
        .collect();

    let mut out: Vec<String> = Vec::new();
    let mut conflicts = 0usize;
    let (mut o0, mut a0, mut b0) = (0usize, 0usize, 0usize);
    let mut idx = 0usize;
    loop {
        let (o1, a1, b1) = if idx < anchors.len() {
            let oi = anchors[idx];
            (oi, map_a[&oi], map_b[&oi])
        } else {
            (o.len(), a.len(), b.len())
        };
        let (oo, aa, bb) = (&o[o0..o1], &a[a0..a1], &b[b0..b1]);
        if aa == oo {
            out.extend_from_slice(bb); // only B changed this region
        } else if bb == oo {
            out.extend_from_slice(aa); // only A changed this region
        } else if aa == bb {
            out.extend_from_slice(aa); // both made the same change
        } else {
            conflicts += 1;
            out.push(format!("<<<<<<< {la}"));
            out.extend_from_slice(aa);
            out.push("=======".to_string());
            out.extend_from_slice(bb);
            out.push(format!(">>>>>>> {lb}"));
        }
        if idx >= anchors.len() {
            break;
        }
        out.push(o[o1].clone()); // the stable anchor line
        o0 = o1 + 1;
        a0 = a1 + 1;
        b0 = b1 + 1;
        idx += 1;
    }
    (out, conflicts)
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
