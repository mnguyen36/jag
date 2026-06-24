//! Remote endpoints, stored as JSON `name -> url` in `.jag/remotes`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Result;

use crate::repo::Repo;

fn remotes_path(repo: &Repo) -> PathBuf {
    repo.jagdir().join("remotes")
}

pub fn load_remotes(repo: &Repo) -> Result<BTreeMap<String, String>> {
    let path = remotes_path(repo);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    Ok(serde_json::from_str(&fs::read_to_string(&path)?).unwrap_or_default())
}

pub fn save_remotes(repo: &Repo, remotes: &BTreeMap<String, String>) -> Result<()> {
    fs::write(remotes_path(repo), serde_json::to_string_pretty(remotes)?)?;
    Ok(())
}

pub fn get_remote(repo: &Repo, name: &str) -> Result<Option<String>> {
    Ok(load_remotes(repo)?.get(name).cloned())
}
