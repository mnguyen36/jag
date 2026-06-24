//! Tree objects: snapshots of a directory hierarchy.
//!
//! Serialized as one line per entry, tab-separated from the name so names may
//! contain spaces: `<mode> <kind> <oid>\t<name>`. Text rather than packed
//! binary, so an agent can `jag cat-file <tree>` and read it directly.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};

use crate::objects::{hash_object, read_object, ObjectKind};
use crate::repo::Repo;

#[derive(Clone, Debug)]
pub struct TreeEntry {
    pub mode: String,
    pub kind: ObjectKind,
    pub oid: String,
}

/// Write a single-level tree from `name -> entry`.
pub fn write_tree(repo: &Repo, entries: &BTreeMap<String, TreeEntry>) -> Result<String> {
    let mut out = String::new();
    for (name, e) in entries {
        out.push_str(&format!("{} {} {}\t{}\n", e.mode, e.kind.as_str(), e.oid, name));
    }
    hash_object(repo, out.as_bytes(), ObjectKind::Tree, true)
}

/// Read a single-level tree into `name -> entry`.
pub fn read_tree(repo: &Repo, oid: &str) -> Result<BTreeMap<String, TreeEntry>> {
    let (kind, data) = read_object(repo, oid)?;
    if kind != ObjectKind::Tree {
        bail!("object {oid} is not a tree");
    }
    let text = String::from_utf8(data)?;
    let mut entries = BTreeMap::new();
    for line in text.lines() {
        let (meta, name) = line
            .split_once('\t')
            .ok_or_else(|| anyhow!("malformed tree line: {line}"))?;
        let mut parts = meta.split(' ');
        let mode = parts.next().unwrap_or("").to_string();
        let kind = ObjectKind::parse(parts.next().unwrap_or(""))?;
        let oid = parts.next().unwrap_or("").to_string();
        entries.insert(name.to_string(), TreeEntry { mode, kind, oid });
    }
    Ok(entries)
}

enum Node {
    Leaf { mode: String, oid: String },
    Dir(BTreeMap<String, Node>),
}

/// Build nested trees from a flat `{"a/b/c.txt": (mode, oid)}` map and return
/// the root tree oid.
pub fn build_tree_from_paths(
    repo: &Repo,
    path_oids: &BTreeMap<String, (String, String)>,
) -> Result<String> {
    let mut root: BTreeMap<String, Node> = BTreeMap::new();
    for (path, (mode, oid)) in path_oids {
        let parts: Vec<&str> = path.split('/').collect();
        insert_node(&mut root, &parts, mode, oid);
    }
    write_node(repo, &root)
}

fn insert_node(dir: &mut BTreeMap<String, Node>, parts: &[&str], mode: &str, oid: &str) {
    if parts.len() == 1 {
        dir.insert(
            parts[0].to_string(),
            Node::Leaf {
                mode: mode.to_string(),
                oid: oid.to_string(),
            },
        );
        return;
    }
    let entry = dir
        .entry(parts[0].to_string())
        .or_insert_with(|| Node::Dir(BTreeMap::new()));
    if let Node::Dir(sub) = entry {
        insert_node(sub, &parts[1..], mode, oid);
    } else {
        let mut sub = BTreeMap::new();
        insert_node(&mut sub, &parts[1..], mode, oid);
        *entry = Node::Dir(sub);
    }
}

fn write_node(repo: &Repo, dir: &BTreeMap<String, Node>) -> Result<String> {
    let mut entries = BTreeMap::new();
    for (name, node) in dir {
        match node {
            Node::Leaf { mode, oid } => {
                entries.insert(
                    name.clone(),
                    TreeEntry {
                        mode: mode.clone(),
                        kind: ObjectKind::Blob,
                        oid: oid.clone(),
                    },
                );
            }
            Node::Dir(sub) => {
                let sub_oid = write_node(repo, sub)?;
                entries.insert(
                    name.clone(),
                    TreeEntry {
                        mode: "040000".to_string(),
                        kind: ObjectKind::Tree,
                        oid: sub_oid,
                    },
                );
            }
        }
    }
    write_tree(repo, &entries)
}

/// Recursively flatten a tree to `{path: (mode, oid)}`.
pub fn flatten_tree(
    repo: &Repo,
    oid: &str,
    prefix: &str,
) -> Result<BTreeMap<String, (String, String)>> {
    let mut result = BTreeMap::new();
    for (name, e) in read_tree(repo, oid)? {
        let path = format!("{prefix}{name}");
        match e.kind {
            ObjectKind::Tree => {
                result.extend(flatten_tree(repo, &e.oid, &format!("{path}/"))?);
            }
            _ => {
                result.insert(path, (e.mode, e.oid));
            }
        }
    }
    Ok(result)
}
