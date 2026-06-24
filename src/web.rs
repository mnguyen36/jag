//! The server's web dashboard: a single embedded page plus a JSON overview the
//! page polls. Read-only — it visualizes lanes, agents, commits, and contention.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::list_agents;
use crate::commit::read_commit;
use crate::objects::read_object;
use crate::reconcile::find_contention;
use crate::refs::{current_lane, head_commit, list_lanes};
use crate::repo::Repo;
use crate::tree::flatten_tree;

/// The dashboard page, compiled into the binary so the server is self-contained.
pub const INDEX: &str = include_str!("web/index.html");

fn short(oid: &str) -> String {
    oid[..oid.len().min(8)].to_string()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Walk a lane's first-parent chain into a list of commit summaries (newest first).
fn chain(repo: &Repo, tip: &str, limit: usize) -> Vec<Value> {
    let mut out = Vec::new();
    let mut cur = Some(tip.to_string());
    while let Some(oid) = cur {
        if out.len() >= limit {
            break;
        }
        match read_commit(repo, &oid) {
            Ok(c) => {
                out.push(json!({
                    "oid": oid,
                    "short": short(&oid),
                    "agent": c.agent,
                    "lane": c.lane,
                    "message": c.message.lines().next().unwrap_or("").to_string(),
                    "time": c.time,
                    "merge": c.parents.len() > 1,
                }));
                cur = c.parents.into_iter().next();
            }
            Err(_) => break,
        }
    }
    out
}

fn count_objects(repo: &Repo) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(repo.objects_dir()) {
        for shard in rd.flatten() {
            if shard.path().is_dir() {
                if let Ok(inner) = std::fs::read_dir(shard.path()) {
                    n += inner.flatten().count();
                }
            }
        }
    }
    n
}

/// The JSON the dashboard renders.
pub fn overview(repo: &Repo) -> Result<Value> {
    let lanes_map = list_lanes(repo)?;

    let mut lanes = Vec::new();
    for (name, tip) in &lanes_map {
        let nodes = chain(repo, tip, 8);
        let head = read_commit(repo, tip).ok();
        lanes.push(json!({
            "name": name,
            "tip": tip,
            "short": short(tip),
            "agent": head.as_ref().map(|c| c.agent.clone()),
            "message": head.as_ref().map(|c| c.message.lines().next().unwrap_or("").to_string()),
            "time": head.as_ref().map(|c| c.time),
            "count": nodes.len(),
            "nodes": nodes,
        }));
    }

    let mut agents = Vec::new();
    for a in list_agents(repo)? {
        let lane = current_lane(repo, Some(&a))?;
        let tip = head_commit(repo, Some(&a))?;
        agents.push(json!({
            "name": a,
            "lane": lane,
            "short": tip.as_deref().map(short),
        }));
    }

    let mut contention = Vec::new();
    for (path, producers) in find_contention(repo)? {
        let ps: Vec<Value> = producers
            .iter()
            .map(|(who, oid)| json!({ "who": who, "short": short(oid) }))
            .collect();
        contention.push(json!({ "path": path, "producers": ps }));
    }

    let commits = match lanes_map.get("main") {
        Some(tip) => chain(repo, tip, 25),
        None => Vec::new(),
    };

    let repo_name = repo
        .root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "jag".to_string());

    Ok(json!({
        "repo": repo_name,
        "root": repo.root.to_string_lossy(),
        "now": now(),
        "objects": count_objects(repo),
        "lanes": lanes,
        "agents": agents,
        "contention": contention,
        "commits": commits,
    }))
}

/// Blob contents as lines, or `None` if it looks binary.
fn blob_lines(repo: &Repo, oid: &str) -> Option<Vec<String>> {
    let (_, data) = read_object(repo, oid).ok()?;
    if data.iter().take(8000).any(|&b| b == 0) {
        return None; // binary
    }
    Some(
        String::from_utf8_lossy(&data)
            .lines()
            .map(|s| s.to_string())
            .collect(),
    )
}

/// Line-level diff: ` ` context, `-` removed, `+` added.
fn lcs_diff(a: &[String], b: &[String]) -> Vec<String> {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
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
            out.push(format!(" {}", a[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push(format!("-{}", a[i]));
            i += 1;
        } else {
            out.push(format!("+{}", b[j]));
            j += 1;
        }
    }
    while i < n {
        out.push(format!("-{}", a[i]));
        i += 1;
    }
    while j < m {
        out.push(format!("+{}", b[j]));
        j += 1;
    }
    out
}

/// A commit's metadata plus its diff against its first parent.
pub fn commit_diff(repo: &Repo, oid: &str) -> Result<Value> {
    let c = read_commit(repo, oid)?;
    let new_tree = flatten_tree(repo, &c.tree, "")?;
    let old_tree = match c.parents.first() {
        Some(p) => flatten_tree(repo, &read_commit(repo, p)?.tree, "")?,
        None => BTreeMap::new(),
    };

    let mut paths: BTreeSet<&String> = old_tree.keys().collect();
    paths.extend(new_tree.keys());

    let mut files = Vec::new();
    for path in paths {
        let old_oid = old_tree.get(path).map(|x| x.1.as_str());
        let new_oid = new_tree.get(path).map(|x| x.1.as_str());
        if old_oid == new_oid {
            continue;
        }
        let status = match (old_oid, new_oid) {
            (None, _) => "added",
            (_, None) => "deleted",
            _ => "modified",
        };
        let old_lines = old_oid.and_then(|o| blob_lines(repo, o));
        let new_lines = new_oid.and_then(|o| blob_lines(repo, o));

        // Binary on either side, or a very large change: summarize instead of diffing.
        let (lines, adds, dels, big): (Vec<String>, usize, usize, Option<String>) =
            match (old_lines, new_lines) {
                (ol, nl) if old_oid.map_or(false, |_| ol.is_none())
                    || new_oid.map_or(false, |_| nl.is_none()) =>
                {
                    (vec![], 0, 0, Some("binary file".to_string()))
                }
                (ol, nl) => {
                    let a = ol.unwrap_or_default();
                    let b = nl.unwrap_or_default();
                    if a.len() + b.len() > 3000 {
                        (vec![], b.len(), a.len(), Some(format!(
                            "large change — {} → {} lines (open in your editor)",
                            a.len(),
                            b.len()
                        )))
                    } else {
                        let hunks = lcs_diff(&a, &b);
                        let adds = hunks.iter().filter(|l| l.starts_with('+')).count();
                        let dels = hunks.iter().filter(|l| l.starts_with('-')).count();
                        let shown: Vec<String> = if hunks.len() > 800 {
                            let mut t: Vec<String> = hunks.into_iter().take(800).collect();
                            t.push(" … diff truncated …".to_string());
                            t
                        } else {
                            hunks
                        };
                        (shown, adds, dels, None)
                    }
                }
            };

        files.push(json!({
            "path": path,
            "status": status,
            "adds": adds,
            "dels": dels,
            "lines": lines,
            "big": big,
        }));
    }

    Ok(json!({
        "oid": oid,
        "short": short(oid),
        "agent": c.agent,
        "lane": c.lane,
        "message": c.message,
        "time": c.time,
        "parents": c.parents,
        "files": files,
    }))
}
