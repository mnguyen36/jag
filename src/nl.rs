//! `jag <sentence>` — a tiny, dependency-free natural-language front-end.
//!
//! This is NOT a model: it's deterministic keyword/intent matching that covers
//! the common phrasings ("commit my changes and push", "merge X into main",
//! "undo that", "what changed?"). It always prints the command it resolved to,
//! and confirms before running a mutating one.
//!
//! For free-form queries the rules miss, an optional local-LLM backend (e.g.
//! Ollama, or an embedded llama.cpp) can be layered in here as a fallback that
//! returns one of the same `Action`s — the execution path below stays the same.

use std::io::{IsTerminal, Write};

use anyhow::{bail, Result};

use crate::commands;
use crate::repo::Repo;

enum Action {
    Push { message: Option<String>, local: bool },
    Merge { lanes: Vec<String> },
    Checkout { lane: String },
    LaneNew { name: String },
    LaneList,
    AgentList,
    Status,
    Log,
    Undo,
    Redo,
}

struct Resolved {
    action: Action,
    display: String,
    /// Whether to ask for confirmation at the NL layer. `merge` sets this false
    /// because `reconcile` runs its own preview/confirm.
    confirm: bool,
}

pub fn run(sentence: &str, agent: Option<String>) -> Result<()> {
    // 1. Try the instant, offline matcher.
    if let Some(resolved) = resolve(sentence) {
        return execute_resolved(resolved, agent);
    }

    // 2. Fall back to the configured local model, if any.
    let cfg = crate::model::load();
    if cfg.enabled {
        let cmd = crate::model::ask(&cfg, sentence)?;
        if cmd.trim().is_empty() || cmd.split_whitespace().next() == Some("do") {
            bail!("the model didn't return a usable command for: \"{}\"", sentence.trim());
        }
        println!("Interpreted as (model {}):  jag {cmd}", cfg.model);
        if !confirm()? {
            return Ok(());
        }
        let mut args = vec!["jag".to_string()];
        args.extend(tokenize(&cmd));
        return crate::dispatch(&args);
    }

    bail!(
        "couldn't map that to a command: \"{}\"\n  \
         enable a local model for free-form requests:  jag model setup\n  \
         or try:  jag save my work and push  |  jag merge <lane>  |  jag undo",
        sentence.trim()
    )
}

fn execute_resolved(resolved: Resolved, agent: Option<String>) -> Result<()> {
    println!("Interpreted as:  jag {}", resolved.display);
    if resolved.confirm && !confirm()? {
        return Ok(());
    }
    let repo = Repo::find(None, agent)?;
    match resolved.action {
        Action::Push { message, local } => commands::push_all(&repo, message, "origin", local),
        Action::Merge { lanes } => commands::reconcile(&repo, "main", None, lanes, false),
        Action::Checkout { lane } => commands::checkout(&repo, &lane),
        Action::LaneNew { name } => commands::lane_new(&repo, &name, "main"),
        Action::LaneList => commands::lane_list(&repo),
        Action::AgentList => commands::agent_list(&repo),
        Action::Status => commands::status(&repo),
        Action::Log => commands::log(&repo, 20),
        Action::Undo => commands::undo(&repo),
        Action::Redo => commands::redo(&repo),
    }
}

/// Prompt to proceed in an interactive terminal; auto-yes when non-interactive.
fn confirm() -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Ok(true);
    }
    print!("Run it? [Y/n] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let a = line.trim().to_lowercase();
    if a == "n" || a == "no" {
        println!("cancelled");
        Ok(false)
    } else {
        Ok(true)
    }
}

/// Minimal quote-aware tokenizer for a model-produced command line.
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                } else if c.is_whitespace() {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                } else {
                    cur.push(c);
                }
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn resolve(sentence: &str) -> Option<Resolved> {
    let s = sentence.to_lowercase();
    let has = |w: &str| s.contains(w);

    if has("undo") || has("revert the last") || has("take back") {
        return Some(simple(Action::Undo, "undo", true));
    }
    if has("redo") || has("reapply") {
        return Some(simple(Action::Redo, "redo", true));
    }
    if has("status") || has("what changed") || has("what's changed") || has("what has changed") {
        return Some(simple(Action::Status, "status", false));
    }
    if has("history") || has("recent commits") || s.split_whitespace().next() == Some("log") {
        return Some(simple(Action::Log, "log", false));
    }
    if (has("list") || has("show") || has("which") || has("what"))
        && (has("lane") || has("branch"))
        && !has("new")
        && !has("create")
    {
        return Some(simple(Action::LaneList, "lane list", false));
    }
    if has("agent") && (has("list") || has("show") || has("who") || has("which")) {
        return Some(simple(Action::AgentList, "agent list", false));
    }
    if (has("new") || has("create") || has("make") || has("start"))
        && (has("lane") || has("branch"))
    {
        if let Some(name) = named_lane(&s) {
            let display = format!("lane new {name}");
            return Some(Resolved {
                action: Action::LaneNew { name },
                display,
                confirm: true,
            });
        }
    }
    if has("switch") || has("checkout") || has("check out") || has("go to") || has("move to") {
        if let Some(name) = named_lane(&s) {
            let display = format!("checkout {name}");
            return Some(Resolved {
                action: Action::Checkout { lane: name },
                display,
                confirm: true,
            });
        }
    }
    if has("merge") || has("reconcile") || has("combine") || has("integrate") {
        let lanes = merge_sources(&s);
        let display = if lanes.is_empty() {
            "merge".to_string()
        } else {
            format!("merge {}", lanes.join(" "))
        };
        // reconcile runs its own preview/confirm, so don't double-prompt here.
        return Some(Resolved {
            action: Action::Merge { lanes },
            display,
            confirm: false,
        });
    }
    if has("commit")
        || has("save")
        || has("push")
        || has("ship")
        || has("upload")
        || has("share")
        || has("sync")
    {
        let message = message(sentence);
        let wants_remote = has("push")
            || has("share")
            || has("upload")
            || has("sync")
            || has("remote")
            || has("origin");
        let local = !wants_remote;
        let display = match (&message, local) {
            (Some(m), true) => format!("push --local -m \"{m}\""),
            (Some(m), false) => format!("push -m \"{m}\""),
            (None, true) => "push --local".to_string(),
            (None, false) => "push".to_string(),
        };
        return Some(Resolved {
            action: Action::Push { message, local },
            display,
            confirm: true,
        });
    }
    None
}

fn simple(action: Action, display: &str, confirm: bool) -> Resolved {
    Resolved {
        action,
        display: display.to_string(),
        confirm,
    }
}

/// Strip surrounding punctuation but keep lane-name characters (incl. '/').
fn clean(tok: &str) -> String {
    tok.trim_matches(|c: char| !(c.is_alphanumeric() || "/-_.".contains(c)))
        .to_string()
}

fn named_lane(s_lower: &str) -> Option<String> {
    let toks: Vec<&str> = s_lower.split_whitespace().collect();
    let stop = ["lane", "branch", "list", "new", "the", "a", "called", "named", "to", "into"];
    for (i, t) in toks.iter().enumerate() {
        if ["lane", "branch", "called", "named"].contains(t) {
            if let Some(n) = toks.get(i + 1) {
                let c = clean(n);
                if !c.is_empty() && !stop.contains(&c.as_str()) {
                    return Some(c);
                }
            }
        }
    }
    // "to <name> lane"
    for w in toks.windows(3) {
        if w[0] == "to" && (w[2] == "lane" || w[2] == "branch") {
            let c = clean(w[1]);
            if !c.is_empty() {
                return Some(c);
            }
        }
    }
    None
}

fn merge_sources(s_lower: &str) -> Vec<String> {
    let toks: Vec<&str> = s_lower.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let skip = ["into", "lane", "the", "branch", "changes", "work", "everything", "all"];
    for (i, t) in toks.iter().enumerate() {
        if ["merge", "reconcile", "combine", "integrate"].contains(t) {
            if let Some(n) = toks.get(i + 1) {
                let c = clean(n);
                if !c.is_empty() && !skip.contains(&c.as_str()) && !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        if *t == "into" && i > 0 {
            let c = clean(toks[i - 1]);
            if !c.is_empty() && !skip.contains(&c.as_str()) && c != "merge" && !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

fn message(orig: &str) -> Option<String> {
    let lower = orig.to_lowercase();
    let anchors = [
        "message ", "saying ", "named ", "called ", "commit ", "save ", "ship ", "push ", "-m ",
    ];
    let start = anchors
        .iter()
        .find_map(|a| lower.find(a).map(|i| i + a.len()))?;
    let mut msg = orig[start.min(orig.len())..].to_string();
    let ml = msg.to_lowercase();
    let cuts = [
        " and push", " then push", " and share", " and upload", " and sync", " and then",
        " to main", " to the main", " into ",
    ];
    let mut cut_at = msg.len();
    for c in cuts {
        if let Some(i) = ml.find(c) {
            cut_at = cut_at.min(i);
        }
    }
    msg.truncate(cut_at);
    let msg = msg.trim().trim_matches('"').trim().to_string();
    if msg.is_empty() || msg == "my" || msg == "all" || msg == "the" {
        None
    } else {
        Some(msg)
    }
}
