//! Lanes (JAG's branches) and per-agent HEAD.
//!
//! A lane is a named ref under `.jag/refs/lanes/<name>` holding a commit oid —
//! shared by all agents. An agent's HEAD lives under its own dir and points at
//! a lane (`ref: refs/lanes/<name>`) or, when detached, holds a raw oid.

use std::collections::BTreeMap;
use std::fs;

use anyhow::Result;
use walkdir::WalkDir;

use crate::repo::Repo;

const LANE_PREFIX: &str = "ref: refs/lanes/";

pub fn read_lane(repo: &Repo, lane: &str) -> Result<Option<String>> {
    let path = repo.lanes_dir().join(lane);
    if !path.exists() {
        return Ok(None);
    }
    let s = fs::read_to_string(&path)?.trim().to_string();
    Ok(if s.is_empty() { None } else { Some(s) })
}

pub fn write_lane(repo: &Repo, lane: &str, oid: &str) -> Result<()> {
    let path = repo.lanes_dir().join(lane);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format!("{oid}\n"))?;
    Ok(())
}

/// All lanes, including nested remote-tracking lanes like `origin/main`. Names
/// are relative to the lanes dir using `/` separators.
pub fn list_lanes(repo: &Repo) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let base = repo.lanes_dir();
    if !base.is_dir() {
        return Ok(out);
    }
    for entry in WalkDir::new(&base).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let name = entry
                .path()
                .strip_prefix(&base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if let Some(oid) = read_lane(repo, &name)? {
                out.insert(name, oid);
            }
        }
    }
    Ok(out)
}

pub fn read_head(repo: &Repo, agent: Option<&str>) -> Result<Option<String>> {
    let path = repo.head_path(agent);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(fs::read_to_string(&path)?.trim().to_string()))
}

pub fn write_head(repo: &Repo, reference: &str, agent: Option<&str>) -> Result<()> {
    let path = repo.head_path(agent);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format!("{reference}\n"))?;
    Ok(())
}

/// The lane an agent's HEAD points at, or `None` if detached.
pub fn current_lane(repo: &Repo, agent: Option<&str>) -> Result<Option<String>> {
    match read_head(repo, agent)? {
        Some(h) if h.starts_with(LANE_PREFIX) => Ok(Some(h[LANE_PREFIX.len()..].to_string())),
        _ => Ok(None),
    }
}

/// The commit an agent is currently on (resolving its lane, or detached oid).
pub fn head_commit(repo: &Repo, agent: Option<&str>) -> Result<Option<String>> {
    if let Some(lane) = current_lane(repo, agent)? {
        return read_lane(repo, &lane);
    }
    read_head(repo, agent)
}
