//! The server's web dashboard: a single embedded page plus a JSON overview the
//! page polls. Read-only — it visualizes lanes, agents, commits, and contention.

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::list_agents;
use crate::commit::read_commit;
use crate::reconcile::find_contention;
use crate::refs::{current_lane, head_commit, list_lanes};
use crate::repo::Repo;

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
