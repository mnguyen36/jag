//! Optional local-LLM backend for the `jag <sentence>` front-end.
//!
//! Config is install-wide (`~/.jag/nl.json`), not per-repo, because the model
//! belongs to the installed `jag`, not to any one repository. The default
//! provider is Ollama; the model is asked to emit a single jag command line,
//! which jag then runs. The built-in matcher in `nl.rs` handles the common
//! cases for free — the model is only consulted when that matcher can't map a
//! request, so common commands stay instant and offline.

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct NlConfig {
    pub enabled: bool,
    pub provider: String,
    pub host: String,
    pub model: String,
}

impl Default for NlConfig {
    fn default() -> Self {
        NlConfig {
            enabled: false,
            provider: "ollama".to_string(),
            host: "http://localhost:11434".to_string(),
            model: "qwen2.5:0.5b".to_string(),
        }
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

pub fn config_path() -> Result<PathBuf> {
    Ok(home()
        .context("cannot locate home directory for jag config")?
        .join(".jag")
        .join("nl.json"))
}

pub fn load() -> NlConfig {
    config_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &NlConfig) -> Result<()> {
    let p = config_path()?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&p, serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

#[derive(Serialize)]
struct GenOptions {
    temperature: f32,
    num_predict: i32,
}
#[derive(Serialize)]
struct GenRequest<'a> {
    model: &'a str,
    prompt: String,
    stream: bool,
    options: GenOptions,
}
#[derive(Deserialize)]
struct GenResponse {
    response: String,
}

const SYSTEM: &str = "You translate a user's request into ONE jag version-control command line.
Output ONLY the command: no explanation, no backticks, no leading 'jag'.
Commands:
  push -m \"<message>\"       stage all changes, commit, and push
  push --local -m \"<msg>\"   commit locally only (do not push)
  undo                       undo the last change
  redo                       reapply the last undone change
  status                     show working-tree status
  log                        show commit history
  merge <lane>               merge a lane into main
  checkout <lane>            switch to a lane
  lane new <name>            create a lane
  lane list                  list lanes
  agent list                 list agents
Examples:
  Request: commit my changes and push to main
  Command: push -m \"my changes\"
  Request: just save locally, don't push, message wip
  Command: push --local -m \"wip\"
  Request: undo that last thing
  Command: undo
  Request: bring feature-login into main
  Command: merge feature-login
  Request: what's the status
  Command: status";

/// Ask the configured model to translate `sentence` into a jag command line.
pub fn ask(cfg: &NlConfig, sentence: &str) -> Result<String> {
    if cfg.provider != "ollama" {
        bail!("unsupported NL provider: {}", cfg.provider);
    }
    let body = serde_json::to_string(&GenRequest {
        model: &cfg.model,
        prompt: format!("{SYSTEM}\nRequest: {sentence}\nCommand:"),
        stream: false,
        options: GenOptions {
            temperature: 0.0,
            num_predict: 48,
        },
    })?;
    let url = format!("{}/api/generate", cfg.host.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|e| anyhow::anyhow!("model request to {} failed: {e}", cfg.host))?
        .into_string()?;
    let parsed: GenResponse =
        serde_json::from_str(&resp).map_err(|e| anyhow::anyhow!("unexpected model response: {e}"))?;
    Ok(clean_command(&parsed.response))
}

/// Reachability check for `jag model status`.
pub fn reachable(cfg: &NlConfig) -> bool {
    let url = format!("{}/api/tags", cfg.host.trim_end_matches('/'));
    ureq::get(&url).call().is_ok()
}

/// First meaningful line, with backticks and a leading 'jag '/'Command:' stripped.
fn clean_command(raw: &str) -> String {
    let mut line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_matches('`')
        .trim()
        .to_string();
    for prefix in ["Command:", "command:", "jag "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            line = rest.trim().to_string();
        }
    }
    line
}
