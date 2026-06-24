//! Content-addressable object store.
//!
//! An object is `"<kind> <len>\0"` followed by its payload, SHA-256 hashed to
//! produce the object id (oid), then zlib-compressed on disk at
//! `objects/ab/cdef…`. This mirrors Git's model so the design is familiar, but
//! trees and commits are stored as readable text (see `tree.rs`/`commit.rs`).

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::repo::Repo;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjectKind {
    Blob,
    Tree,
    Commit,
}

impl ObjectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectKind::Blob => "blob",
            ObjectKind::Tree => "tree",
            ObjectKind::Commit => "commit",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "blob" => ObjectKind::Blob,
            "tree" => ObjectKind::Tree,
            "commit" => ObjectKind::Commit,
            other => bail!("unknown object kind: {other}"),
        })
    }
}

fn obj_path(repo: &Repo, oid: &str) -> PathBuf {
    repo.objects_dir().join(&oid[..2]).join(&oid[2..])
}

/// Hash `data` as an object of `kind`, optionally writing it to the store.
/// Writes are content-addressed and idempotent: an existing object is left
/// untouched, which is what makes concurrent writers safe.
pub fn hash_object(repo: &Repo, data: &[u8], kind: ObjectKind, write: bool) -> Result<String> {
    let header = format!("{} {}", kind.as_str(), data.len());
    let mut store = Vec::with_capacity(header.len() + 1 + data.len());
    store.extend_from_slice(header.as_bytes());
    store.push(0);
    store.extend_from_slice(data);

    let mut hasher = Sha256::new();
    hasher.update(&store);
    let oid = hex(&hasher.finalize());

    if write {
        let path = obj_path(repo, &oid);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(&store)?;
            let compressed = enc.finish()?;
            fs::write(&path, compressed)?;
        }
    }
    Ok(oid)
}

/// Read an object, returning its kind and raw payload (header stripped).
pub fn read_object(repo: &Repo, oid: &str) -> Result<(ObjectKind, Vec<u8>)> {
    let path = obj_path(repo, oid);
    if !path.exists() {
        bail!("object not found: {oid}");
    }
    let compressed = fs::read(&path)?;
    let mut dec = ZlibDecoder::new(&compressed[..]);
    let mut raw = Vec::new();
    dec.read_to_end(&mut raw)?;

    let nul = raw
        .iter()
        .position(|&b| b == 0)
        .context("malformed object: missing header terminator")?;
    let header = std::str::from_utf8(&raw[..nul])?;
    let kind = ObjectKind::parse(header.split(' ').next().unwrap_or(""))?;
    Ok((kind, raw[nul + 1..].to_vec()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}
