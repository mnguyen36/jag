//! Agent sessions — the unit of concurrency.
//!
//! Starting an agent creates `.jag/agents/<name>/` with its own HEAD and index,
//! and (by default) forks a fresh lane named after the agent off a base lane.
//! From then on, many agents coexist in one folder, each committing to its own
//! lane, until `reconcile` brings the lanes back together.

use std::fs;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::commit::commit_tree;
use crate::index::seed_index_from_tree;
use crate::refs::{read_lane, write_head, write_lane};
use crate::repo::Repo;

#[derive(Serialize, Deserialize)]
pub struct AgentMeta {
    pub created: i64,
    pub lane: String,
}

pub fn list_agents(repo: &Repo) -> Result<Vec<String>> {
    let base = repo.agents_dir();
    let mut out = Vec::new();
    if !base.is_dir() {
        return Ok(out);
    }
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            out.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    out.sort();
    Ok(out)
}

/// Create an agent. Returns the lane it was placed on. If that lane does not
/// exist yet, it is forked from `base_lane`'s current tip.
pub fn create_agent(
    repo: &Repo,
    name: &str,
    lane: Option<&str>,
    base_lane: &str,
    time: i64,
) -> Result<String> {
    let adir = repo.agent_dir(Some(name));
    if adir.exists() {
        bail!("agent already exists: {name}");
    }
    fs::create_dir_all(&adir)?;

    let lane = lane.unwrap_or(name).to_string();
    let tip = match read_lane(repo, &lane)? {
        Some(existing) => Some(existing),
        None => {
            let base_tip = read_lane(repo, base_lane)?;
            if let Some(t) = &base_tip {
                write_lane(repo, &lane, t)?;
            }
            base_tip
        }
    };

    write_head(repo, &format!("ref: refs/lanes/{lane}"), Some(name))?;
    let tree = commit_tree(repo, tip.as_deref())?;
    seed_index_from_tree(repo, name, tree.as_deref())?;

    let meta = AgentMeta {
        created: time,
        lane: lane.clone(),
    };
    fs::write(adir.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
    Ok(lane)
}
