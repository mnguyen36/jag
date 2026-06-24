//! The shared working tree: one physical folder all agents read and write.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;

use crate::objects::{hash_object, read_object, ObjectKind};
use crate::repo::Repo;

const DEFAULT_IGNORES: &[&str] = &[".jag", ".git", "node_modules", "__pycache__", "target", ".env"];

pub fn load_ignores(repo: &Repo) -> Vec<String> {
    let mut pats: Vec<String> = DEFAULT_IGNORES.iter().map(|s| s.to_string()).collect();
    if let Ok(content) = fs::read_to_string(repo.root.join(".jagignore")) {
        for line in content.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                pats.push(line.to_string());
            }
        }
    }
    pats
}

fn is_ignored(rel: &str, pats: &[String]) -> bool {
    let rel = rel.trim_matches('/');
    if rel.is_empty() {
        return false;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    for pat in pats {
        if parts.iter().any(|p| p == pat) {
            return true;
        }
        if glob_match(pat, rel) {
            return true;
        }
        if let Some(last) = parts.last() {
            if glob_match(pat, last) {
                return true;
            }
        }
    }
    false
}

/// Minimal glob supporting `*` and `?`.
fn glob_match(pat: &str, text: &str) -> bool {
    fn helper(p: &[u8], t: &[u8]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        match p[0] {
            b'*' => helper(&p[1..], t) || (!t.is_empty() && helper(p, &t[1..])),
            b'?' => !t.is_empty() && helper(&p[1..], &t[1..]),
            c => !t.is_empty() && t[0] == c && helper(&p[1..], &t[1..]),
        }
    }
    helper(pat.as_bytes(), text.as_bytes())
}

/// All non-ignored files as `relpath -> absolute path`.
pub fn walk_worktree(repo: &Repo) -> Result<BTreeMap<String, PathBuf>> {
    let pats = load_ignores(repo);
    let mut out = BTreeMap::new();
    let walker = WalkDir::new(&repo.root).into_iter().filter_entry(|e| {
        if e.path() == repo.root {
            return true;
        }
        match e.path().strip_prefix(&repo.root) {
            Ok(rel) => !is_ignored(&rel.to_string_lossy().replace('\\', "/"), &pats),
            Err(_) => false,
        }
    });
    for entry in walker {
        let entry = entry?;
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(&repo.root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, entry.path().to_path_buf());
        }
    }
    Ok(out)
}

pub fn hash_file(repo: &Repo, full: &Path, write: bool) -> Result<String> {
    let data = fs::read(full)?;
    hash_object(repo, &data, ObjectKind::Blob, write)
}

/// Write a blob's contents to a path in the working tree.
pub fn materialize(repo: &Repo, oid: &str, full: &Path) -> Result<()> {
    let (_, data) = read_object(repo, oid)?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(full, data)?;
    Ok(())
}
