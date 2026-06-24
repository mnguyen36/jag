//! Repository location, paths, config, and per-agent resolution.
//!
//! A single repository (single physical folder) hosts many agents. Each agent
//! has its own index, HEAD, and lane, all stored under `.jag/agents/<name>/`,
//! while sharing one object store and one set of lane refs. That sharing is
//! what lets agents work concurrently in one folder instead of cloning N times.

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const JAG_DIR: &str = ".jag";

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub default_agent: String,
    pub default_lane: String,
    pub version: u32,
}

#[derive(Clone)]
pub struct Repo {
    pub root: PathBuf,
    agent_override: Option<String>,
}

impl Repo {
    pub fn new(root: PathBuf, agent_override: Option<String>) -> Self {
        Repo { root, agent_override }
    }

    // --- core paths -------------------------------------------------------
    pub fn jagdir(&self) -> PathBuf {
        self.root.join(JAG_DIR)
    }
    pub fn objects_dir(&self) -> PathBuf {
        self.jagdir().join("objects")
    }
    pub fn lanes_dir(&self) -> PathBuf {
        self.jagdir().join("refs").join("lanes")
    }
    pub fn agents_dir(&self) -> PathBuf {
        self.jagdir().join("agents")
    }
    pub fn config_path(&self) -> PathBuf {
        self.jagdir().join("config")
    }

    // --- config -----------------------------------------------------------
    pub fn config(&self) -> Result<Config> {
        let data = fs::read_to_string(self.config_path())?;
        Ok(serde_json::from_str(&data)?)
    }
    pub fn write_config(&self, cfg: &Config) -> Result<()> {
        fs::write(self.config_path(), serde_json::to_string_pretty(cfg)?)?;
        Ok(())
    }

    // --- agent resolution -------------------------------------------------
    /// The agent this invocation acts as: explicit override, then `JAG_AGENT`,
    /// then the configured default, then "main".
    pub fn agent(&self) -> String {
        if let Some(a) = &self.agent_override {
            return a.clone();
        }
        if let Ok(a) = std::env::var("JAG_AGENT") {
            if !a.is_empty() {
                return a;
            }
        }
        self.config()
            .map(|c| c.default_agent)
            .unwrap_or_else(|_| "main".to_string())
    }

    fn resolve_agent(&self, agent: Option<&str>) -> String {
        match agent {
            Some(a) => a.to_string(),
            None => self.agent(),
        }
    }

    pub fn agent_dir(&self, agent: Option<&str>) -> PathBuf {
        self.agents_dir().join(self.resolve_agent(agent))
    }
    pub fn index_path(&self, agent: Option<&str>) -> PathBuf {
        self.agent_dir(agent).join("index")
    }
    pub fn head_path(&self, agent: Option<&str>) -> PathBuf {
        self.agent_dir(agent).join("HEAD")
    }

    // --- discovery --------------------------------------------------------
    pub fn find(start: Option<PathBuf>, agent_override: Option<String>) -> Result<Repo> {
        let mut path = match start {
            Some(p) => p,
            None => std::env::current_dir()?,
        };
        path = path.canonicalize().unwrap_or(path);
        loop {
            if path.join(JAG_DIR).is_dir() {
                return Ok(Repo::new(path, agent_override));
            }
            match path.parent() {
                Some(p) => path = p.to_path_buf(),
                None => bail!("not a jag repository (no .jag directory found)"),
            }
        }
    }
}
