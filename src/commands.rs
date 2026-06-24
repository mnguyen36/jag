//! User-facing command handlers. Each maps one CLI subcommand to operations on
//! the modules above and prints human-readable output.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};

use crate::agent::{create_agent, list_agents};
use crate::commit::{commit_tree, read_commit, write_commit};
use crate::index::{seed_index_from_tree, Index};
use crate::objects::read_object;
use crate::reconcile::{apply_reconcile, find_contention, plan_reconcile};
use crate::refs::{current_lane, head_commit, list_lanes, read_lane, write_head, write_lane};
use crate::repo::{Config, Repo, JAG_DIR};
use crate::status::compute_status;
use crate::tree::flatten_tree;
use crate::worktree::{hash_file, materialize, walk_worktree};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(10)]
}

// --- init ----------------------------------------------------------------
pub fn init(dir: PathBuf) -> Result<()> {
    let jagdir = dir.join(JAG_DIR);
    if jagdir.exists() {
        bail!("already a jag repository: {}", jagdir.display());
    }
    fs::create_dir_all(jagdir.join("objects"))?;
    fs::create_dir_all(jagdir.join("refs").join("lanes"))?;
    fs::create_dir_all(jagdir.join("agents"))?;

    let repo = Repo::new(dir, None);
    repo.write_config(&Config {
        default_agent: "main".to_string(),
        default_lane: "main".to_string(),
        version: 1,
    })?;
    create_agent(&repo, "main", Some("main"), "main", now())?;

    println!("Initialized empty JAG repository in {}", jagdir.display());
    println!("  default agent: main    default lane: main");
    println!("Start a concurrent agent with:  jag agent start <name>");
    Ok(())
}

// --- add -----------------------------------------------------------------
fn normalize_rel(repo: &Repo, p: &str) -> Result<String> {
    let pb = PathBuf::from(p);
    let abs = if pb.is_absolute() {
        pb
    } else {
        std::env::current_dir()?.join(pb)
    };
    let abs = abs.canonicalize().unwrap_or(abs);
    let rel = abs.strip_prefix(&repo.root).unwrap_or(&abs);
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

pub fn add(repo: &Repo, paths: &[String]) -> Result<()> {
    let agent = repo.agent();
    let mut idx = Index::load(repo, Some(&agent))?;
    let wt = walk_worktree(repo)?;

    let mut targets: Vec<String> = Vec::new();
    for p in paths {
        if p == "." || p == "*" {
            targets.extend(wt.keys().cloned());
            for tracked in idx.entries.keys().cloned().collect::<Vec<_>>() {
                if !wt.contains_key(&tracked) {
                    targets.push(tracked);
                }
            }
            continue;
        }
        let rel = normalize_rel(repo, p)?;
        let mut matched = false;
        let dir_prefix = format!("{rel}/");
        for key in wt.keys() {
            if key == &rel || key.starts_with(&dir_prefix) {
                targets.push(key.clone());
                matched = true;
            }
        }
        if idx.entries.contains_key(&rel) && !wt.contains_key(&rel) {
            targets.push(rel.clone()); // staged deletion
            matched = true;
        }
        if !matched {
            bail!("pathspec did not match any files: {p}");
        }
    }

    let mut staged = 0usize;
    for rel in targets {
        match wt.get(&rel) {
            Some(full) => {
                let oid = hash_file(repo, full, true)?;
                idx.add(&rel, &oid, "100644");
                staged += 1;
            }
            None => {
                idx.remove(&rel);
                staged += 1;
            }
        }
    }
    idx.save()?;
    println!("staged {staged} path(s) for agent '{agent}'");
    Ok(())
}

// --- commit --------------------------------------------------------------
pub fn commit(repo: &Repo, message: &str) -> Result<()> {
    let agent = repo.agent();
    let idx = Index::load(repo, Some(&agent))?;
    let tree = crate::tree::build_tree_from_paths(repo, &idx.path_oids())?;

    let lane = current_lane(repo, Some(&agent))?;
    let parent = match &lane {
        Some(l) => read_lane(repo, l)?,
        None => head_commit(repo, Some(&agent))?,
    };

    if let Some(p) = &parent {
        if commit_tree(repo, Some(p))?.as_deref() == Some(tree.as_str()) {
            println!("nothing to commit (working set matches {})", short(p));
            return Ok(());
        }
    }

    let parents: Vec<String> = parent.into_iter().collect();
    let oid = write_commit(repo, &tree, &parents, &agent, lane.as_deref(), message, now())?;
    match &lane {
        Some(l) => write_lane(repo, l, &oid)?,
        None => write_head(repo, &oid, Some(&agent))?,
    }
    let lane_label = lane.unwrap_or_else(|| "detached".to_string());
    println!(
        "[{} {} {}] {}",
        agent,
        lane_label,
        short(&oid),
        message.lines().next().unwrap_or("")
    );
    Ok(())
}

// --- status --------------------------------------------------------------
pub fn status(repo: &Repo) -> Result<()> {
    let agent = repo.agent();
    let lane = current_lane(repo, Some(&agent))?.unwrap_or_else(|| "(detached)".to_string());
    println!("agent {agent}  on lane {lane}");

    let others: Vec<String> = list_agents(repo)?
        .into_iter()
        .filter(|a| a != &agent)
        .collect();
    if !others.is_empty() {
        println!("concurrent agents: {}", others.join(", "));
    }

    let st = compute_status(repo, &agent)?;
    if st.staged.is_empty() && st.unstaged.is_empty() && st.untracked.is_empty() {
        println!("\nnothing to commit, working tree clean");
        return Ok(());
    }
    if !st.staged.is_empty() {
        println!("\nStaged (will be committed):");
        for (p, c) in &st.staged {
            println!("    {:<9} {}", c.label(), p);
        }
    }
    if !st.unstaged.is_empty() {
        println!("\nNot staged (changed since `jag add`):");
        for (p, c) in &st.unstaged {
            println!("    {:<9} {}", c.label(), p);
        }
    }
    if !st.untracked.is_empty() {
        println!("\nUntracked:");
        for p in &st.untracked {
            println!("    {p}");
        }
    }
    Ok(())
}

// --- log -----------------------------------------------------------------
pub fn log(repo: &Repo, limit: usize) -> Result<()> {
    let agent = repo.agent();
    let mut oid = head_commit(repo, Some(&agent))?;
    if oid.is_none() {
        println!("no commits yet");
        return Ok(());
    }
    let mut count = 0;
    while let Some(o) = oid {
        if count >= limit {
            break;
        }
        let c = read_commit(repo, &o)?;
        println!("commit {o}");
        println!("  agent: {}    lane: {}", c.agent, c.lane.clone().unwrap_or_default());
        if c.parents.len() > 1 {
            let shorts: Vec<String> = c.parents.iter().map(|p| short(p).to_string()).collect();
            println!("  merge: {}", shorts.join(" "));
        }
        println!("  date:  {}", fmt_time(c.time));
        for line in c.message.lines() {
            println!("    {line}");
        }
        println!();
        count += 1;
        oid = c.parents.into_iter().next();
    }
    Ok(())
}

// --- diff ----------------------------------------------------------------
pub fn diff(repo: &Repo, staged: bool) -> Result<()> {
    let agent = repo.agent();
    let idx = Index::load(repo, Some(&agent))?;
    if staged {
        let hc = head_commit(repo, Some(&agent))?;
        let tree = commit_tree(repo, hc.as_deref())?;
        let head_entries = match tree {
            Some(t) => flatten_tree(repo, &t, "")?,
            None => BTreeMap::new(),
        };
        diff_maps(repo, &head_entries, &idx.path_oids(), "HEAD", "index")
    } else {
        let wt = walk_worktree(repo)?;
        let mut wt_oids = BTreeMap::new();
        for (p, full) in &wt {
            wt_oids.insert(p.clone(), ("100644".to_string(), hash_file(repo, full, false)?));
        }
        diff_maps(repo, &idx.path_oids(), &wt_oids, "index", "worktree")
    }
}

fn blob_lines(repo: &Repo, oid: &str) -> Result<Vec<String>> {
    let (_, data) = read_object(repo, oid)?;
    Ok(String::from_utf8_lossy(&data)
        .lines()
        .map(|s| s.to_string())
        .collect())
}

fn diff_maps(
    repo: &Repo,
    a: &BTreeMap<String, (String, String)>,
    b: &BTreeMap<String, (String, String)>,
    alabel: &str,
    blabel: &str,
) -> Result<()> {
    let mut keys: BTreeSet<&String> = a.keys().collect();
    keys.extend(b.keys());
    let mut any = false;
    for k in keys {
        let av = a.get(k).map(|x| x.1.as_str());
        let bv = b.get(k).map(|x| x.1.as_str());
        if av == bv {
            continue;
        }
        any = true;
        println!("--- {alabel}/{k}");
        println!("+++ {blabel}/{k}");
        let al = match av {
            Some(o) => blob_lines(repo, o)?,
            None => vec![],
        };
        let bl = match bv {
            Some(o) => blob_lines(repo, o)?,
            None => vec![],
        };
        for line in lcs_diff(&al, &bl) {
            println!("{line}");
        }
        println!();
    }
    if !any {
        println!("no differences");
    }
    Ok(())
}

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

// --- cat-file ------------------------------------------------------------
pub fn cat_file(repo: &Repo, oid: &str) -> Result<()> {
    let (kind, data) = read_object(repo, oid)?;
    println!("# {} ({} bytes)", kind.as_str(), data.len());
    print!("{}", String::from_utf8_lossy(&data));
    Ok(())
}

// --- agent ---------------------------------------------------------------
pub fn agent_start(repo: &Repo, name: &str, lane: Option<String>, base: &str) -> Result<()> {
    let created = create_agent(repo, name, lane.as_deref(), base, now())?;
    println!("started agent '{name}' on lane '{created}' (forked from '{base}')");
    println!("run as this agent with:  JAG_AGENT={name} jag <cmd>   (or --agent {name})");
    Ok(())
}

pub fn agent_list(repo: &Repo) -> Result<()> {
    let current = repo.agent();
    for a in list_agents(repo)? {
        let lane = current_lane(repo, Some(&a))?.unwrap_or_default();
        let tip = head_commit(repo, Some(&a))?
            .map(|o| short(&o).to_string())
            .unwrap_or_else(|| "-".to_string());
        let marker = if a == current { "*" } else { " " };
        println!("{marker} {a:<16} lane={lane:<16} tip={tip}");
    }
    Ok(())
}

pub fn agent_who(repo: &Repo) -> Result<()> {
    println!("{}", repo.agent());
    Ok(())
}

pub fn agent_use(repo: &Repo, name: &str) -> Result<()> {
    if !list_agents(repo)?.iter().any(|a| a == name) {
        bail!("no such agent: {name}");
    }
    let mut cfg = repo.config()?;
    cfg.default_agent = name.to_string();
    repo.write_config(&cfg)?;
    println!("default agent is now '{name}'");
    Ok(())
}

// --- lane ----------------------------------------------------------------
pub fn lane_list(repo: &Repo) -> Result<()> {
    let lanes = list_lanes(repo)?;
    if lanes.is_empty() {
        println!("no lanes yet (no commits)");
        return Ok(());
    }
    let cur = current_lane(repo, Some(&repo.agent()))?;
    for (name, tip) in lanes {
        let marker = if Some(&name) == cur.as_ref() { "*" } else { " " };
        println!("{marker} {name:<20} {}", short(&tip));
    }
    Ok(())
}

pub fn lane_new(repo: &Repo, name: &str, base: &str) -> Result<()> {
    if read_lane(repo, name)?.is_some() {
        bail!("lane already exists: {name}");
    }
    match read_lane(repo, base)? {
        Some(tip) => {
            write_lane(repo, name, &tip)?;
            println!("created lane '{name}' at {} (from '{base}')", short(&tip));
        }
        None => println!("base lane '{base}' has no commits; '{name}' will appear on first commit"),
    }
    Ok(())
}

// --- checkout ------------------------------------------------------------
pub fn checkout(repo: &Repo, lane: &str) -> Result<()> {
    let agent = repo.agent();
    let tip = read_lane(repo, lane)?;
    write_head(repo, &format!("ref: refs/lanes/{lane}"), Some(&agent))?;

    let tree = commit_tree(repo, tip.as_deref())?;
    let new_paths = match &tree {
        Some(t) => flatten_tree(repo, t, "")?,
        None => BTreeMap::new(),
    };

    // remove files this agent tracked but the target lane doesn't have
    let idx = Index::load(repo, Some(&agent))?;
    for path in idx.entries.keys() {
        if !new_paths.contains_key(path) {
            let _ = fs::remove_file(repo.root.join(path));
        }
    }
    for (path, (_, oid)) in &new_paths {
        materialize(repo, oid, &repo.root.join(path))?;
    }
    seed_index_from_tree(repo, &agent, tree.as_deref())?;
    println!(
        "agent '{agent}' switched to lane '{lane}' ({} file(s) materialized)",
        new_paths.len()
    );
    Ok(())
}

// --- reconcile -----------------------------------------------------------
pub fn reconcile(
    repo: &Repo,
    into: &str,
    message: Option<String>,
    lanes: Vec<String>,
) -> Result<()> {
    let plan = plan_reconcile(repo, into, lanes)?;
    if plan.sources.is_empty() {
        println!("no source lanes to reconcile into '{into}'");
        return Ok(());
    }
    if !plan.conflicts.is_empty() {
        println!(
            "contention — {} path(s) changed differently by multiple sources:",
            plan.conflicts.len()
        );
        for (path, srcvals) in &plan.conflicts {
            let producers: Vec<String> = srcvals
                .iter()
                .map(|(s, o)| match o {
                    Some(oid) => format!("{s}={}", &oid[..oid.len().min(8)]),
                    None => format!("{s}=deleted"),
                })
                .collect();
            println!("    {path}  [{}]", producers.join(", "));
        }
        println!("\nReconcile aborted. Resolve by checking out a lane, editing the path,");
        println!("and committing, then re-run `jag reconcile`.");
        bail!("unresolved contention");
    }
    let oid = apply_reconcile(repo, &plan, message.as_deref(), "reconciler", now())?;
    println!(
        "reconciled [{}] into '{}'  ->  {}",
        plan.sources.join(", "),
        into,
        short(&oid)
    );
    println!("run `jag checkout {into}` to materialize the merged tree");
    Ok(())
}

// --- contention ----------------------------------------------------------
pub fn contention(repo: &Repo) -> Result<()> {
    let c = find_contention(repo)?;
    if c.is_empty() {
        println!("no contention — concurrent work is currently non-overlapping");
        return Ok(());
    }
    println!("contention — paths claimed by multiple agents/lanes with differing content:");
    for (path, producers) in c {
        println!("    {path}");
        let parts: Vec<String> = producers
            .iter()
            .map(|(who, oid)| format!("{who}={}", &oid[..oid.len().min(8)]))
            .collect();
        println!("        {}", parts.join("  "));
    }
    Ok(())
}

// --- time formatting (no external deps) ----------------------------------
fn fmt_time(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

/// Howard Hinnant's days-from-civil inverse: epoch-days -> (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}
