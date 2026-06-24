//! High-level clone / fetch / push, built on the HTTP client and the local
//! object store.

use anyhow::{anyhow, bail, Result};

use crate::client::Client;
use crate::graph::{is_ancestor, reachable};
use crate::objects::{object_exists, read_raw, write_raw};
use crate::refs::{read_lane, write_lane};
use crate::repo::Repo;

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(10)]
}

/// One fetched lane's outcome.
pub struct FetchResult {
    pub lane: String,
    pub status: String,
    pub tip: String,
}

/// Download every remote lane's objects and update local lanes. Fast-forwards
/// (or creates) local lanes; a diverged lane is imported as `<remote>/<lane>`
/// so it can be brought in with `jag reconcile <remote>/<lane>`.
pub fn fetch(repo: &Repo, remote_name: &str, url: &str) -> Result<Vec<FetchResult>> {
    let client = Client::new(url);
    let refs = client.get_refs()?;
    let mut out = Vec::new();

    for (lane, tip) in &refs {
        // Pull the object closure the remote computed for this tip.
        for oid in client.get_closure(tip)? {
            if !object_exists(repo, &oid) {
                let bytes = client
                    .get_object(&oid)?
                    .ok_or_else(|| anyhow!("remote is missing object {oid}"))?;
                write_raw(repo, &oid, &bytes)?;
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let status = match read_lane(repo, lane)? {
            None => {
                write_lane(repo, lane, tip)?;
                // Seed the undo journal so a clone can `undo` back to this tip.
                crate::journal::record(repo, lane, tip, "fetch", now)?;
                "created".to_string()
            }
            Some(local) if &local == tip => "up-to-date".to_string(),
            Some(local) if is_ancestor(repo, &local, tip)? => {
                write_lane(repo, lane, tip)?;
                crate::journal::record(repo, lane, tip, "fetch", now)?;
                "fast-forward".to_string()
            }
            Some(_) => {
                let track = format!("{remote_name}/{lane}");
                write_lane(repo, &track, tip)?;
                format!("diverged (tracked as '{track}')")
            }
        };
        out.push(FetchResult {
            lane: lane.clone(),
            status,
            tip: tip.clone(),
        });
    }
    Ok(out)
}

/// Upload a lane: send objects the remote lacks, then ask it to move the ref.
pub fn push(repo: &Repo, url: &str, lane: &str) -> Result<()> {
    let client = Client::new(url);
    let local_tip =
        read_lane(repo, lane)?.ok_or_else(|| anyhow!("no local lane '{lane}' to push"))?;

    let remote_tip = client.get_refs()?.get(lane).cloned();
    let closure: Vec<String> = reachable(repo, &[local_tip.clone()])?.into_iter().collect();
    let missing = client.missing(&closure)?;
    for oid in &missing {
        let raw = read_raw(repo, oid)?.ok_or_else(|| anyhow!("local object missing {oid}"))?;
        client.post_object(oid, &raw)?;
    }

    let res = client.update_ref(lane, remote_tip.as_deref(), &local_tip)?;
    if res.ok {
        println!(
            "pushed lane '{lane}' -> {} ({} object(s) uploaded)",
            short(&local_tip),
            missing.len()
        );
        Ok(())
    } else {
        bail!(
            "push rejected: {}",
            res.reason.unwrap_or_else(|| "remote diverged".to_string())
        );
    }
}
