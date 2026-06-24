//! `jag serve` — a small blocking HTTP server exposing one repository's object
//! store and lanes so other JAG repos can clone/fetch/push against it.
//!
//! Several worker threads pull from the listener concurrently (so multiple
//! agents can sync at once); object writes are atomic and idempotent, and lane
//! updates are serialized behind a mutex with a fast-forward / contention rule.

use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{anyhow, Result};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::graph::{is_ancestor, reachable};
use crate::objects::{object_exists, read_raw, write_raw};
use crate::protocol::{
    ClosureResponse, MissingRequest, MissingResponse, RefUpdateRequest, RefUpdateResponse,
    RefsResponse,
};
use crate::refs::{list_lanes, read_lane, write_lane};
use crate::repo::Repo;

pub fn serve(repo: &Repo, addr: &str, threads: usize) -> Result<()> {
    let server = Arc::new(Server::http(addr).map_err(|e| anyhow!("cannot bind {addr}: {e}"))?);
    let ref_lock = Arc::new(Mutex::new(()));
    println!("jag server for {} listening on http://{addr}", repo.root.display());
    println!("download it with:  jag dl http://{addr} <dir>");

    let mut handles = Vec::new();
    for _ in 0..threads.max(1) {
        let server = server.clone();
        let repo = repo.clone();
        let ref_lock = ref_lock.clone();
        handles.push(thread::spawn(move || loop {
            match server.recv() {
                Ok(req) => {
                    if let Err(e) = handle(&repo, &ref_lock, req) {
                        eprintln!("jag serve: request error: {e}");
                    }
                }
                Err(_) => break,
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

fn handle(repo: &Repo, ref_lock: &Mutex<()>, mut req: Request) -> std::io::Result<()> {
    let method = req.method().clone();
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("").trim_matches('/').to_string();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let mut body = Vec::new();
    if matches!(method, Method::Post) {
        let _ = req.as_reader().read_to_end(&mut body);
    }

    let (code, payload, ctype): (u16, Vec<u8>, &str) = match (&method, segs.as_slice()) {
        // --- web dashboard ---
        (Method::Get, []) | (Method::Get, ["index.html"]) => {
            (200, crate::web::INDEX.as_bytes().to_vec(), "text/html; charset=utf-8")
        }
        (Method::Get, ["api", "overview"]) => match crate::web::overview(repo) {
            Ok(v) => (200, serde_json::to_vec(&v).unwrap_or_default(), "application/json"),
            Err(e) => (500, e.to_string().into_bytes(), "text/plain"),
        },
        // --- sync protocol ---
        (Method::Get, ["refs"]) => match build_refs(repo) {
            Ok(b) => (200, b, "application/json"),
            Err(e) => (500, e.into_bytes(), "text/plain"),
        },
        (Method::Get, ["closure", oid]) => match build_closure(repo, oid) {
            Ok(b) => (200, b, "application/json"),
            Err(e) => (500, e.into_bytes(), "text/plain"),
        },
        (Method::Get, ["object", oid]) => match read_raw(repo, oid) {
            Ok(Some(bytes)) => (200, bytes, "application/octet-stream"),
            Ok(None) => (404, Vec::new(), "text/plain"),
            Err(e) => (500, e.to_string().into_bytes(), "text/plain"),
        },
        (Method::Post, ["object", oid]) => match write_raw(repo, oid, &body) {
            Ok(()) => (200, b"ok".to_vec(), "text/plain"),
            Err(e) => (400, e.to_string().into_bytes(), "text/plain"),
        },
        (Method::Post, ["missing"]) => match build_missing(repo, &body) {
            Ok(b) => (200, b, "application/json"),
            Err(e) => (400, e.to_string().into_bytes(), "text/plain"),
        },
        // Lane names may contain '/' (e.g. "minh/newlane"), which splits into
        // several path segments — rejoin everything after "ref".
        (Method::Post, ["ref", lane_parts @ ..]) if !lane_parts.is_empty() => {
            let (c, p) = update_ref(repo, ref_lock, &lane_parts.join("/"), &body);
            (c, p, "application/json")
        }
        _ => (404, b"not found".to_vec(), "text/plain"),
    };

    eprintln!("{} /{path} -> {code}", method_str(&method));
    let header = Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes())
        .unwrap_or_else(|_| Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..]).unwrap());
    req.respond(Response::from_data(payload).with_status_code(code).with_header(header))
}

fn build_refs(repo: &Repo) -> std::result::Result<Vec<u8>, String> {
    let lanes = list_lanes(repo).map_err(|e| e.to_string())?;
    serde_json::to_vec(&RefsResponse { lanes }).map_err(|e| e.to_string())
}

fn build_closure(repo: &Repo, oid: &str) -> std::result::Result<Vec<u8>, String> {
    let set = reachable(repo, &[oid.to_string()]).map_err(|e| e.to_string())?;
    let oids = set.into_iter().collect();
    serde_json::to_vec(&ClosureResponse { oids }).map_err(|e| e.to_string())
}

fn build_missing(repo: &Repo, body: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let req: MissingRequest = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let missing = req
        .oids
        .into_iter()
        .filter(|oid| !object_exists(repo, oid))
        .collect();
    serde_json::to_vec(&MissingResponse { missing }).map_err(|e| e.to_string())
}

fn update_ref(repo: &Repo, ref_lock: &Mutex<()>, lane: &str, body: &[u8]) -> (u16, Vec<u8>) {
    let req: RefUpdateRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return (400, e.to_string().into_bytes()),
    };
    let _guard = ref_lock.lock().unwrap();

    if !object_exists(repo, &req.new) {
        return json_409("the new tip's objects were not uploaded first");
    }
    let current = match read_lane(repo, lane) {
        Ok(c) => c,
        Err(e) => return (500, e.to_string().into_bytes()),
    };
    let accept = match &current {
        None => true,
        Some(cur) if cur == &req.new => true,
        Some(cur) => is_ancestor(repo, cur, &req.new).unwrap_or(false),
    };
    if accept {
        if let Err(e) = write_lane(repo, lane, &req.new) {
            return (500, e.to_string().into_bytes());
        }
        let body = serde_json::to_vec(&RefUpdateResponse {
            ok: true,
            reason: None,
        })
        .unwrap_or_default();
        (200, body)
    } else {
        json_409("non-fast-forward: remote lane has diverged; fetch + reconcile, then push")
    }
}

fn json_409(reason: &str) -> (u16, Vec<u8>) {
    let body = serde_json::to_vec(&RefUpdateResponse {
        ok: false,
        reason: Some(reason.to_string()),
    })
    .unwrap_or_default();
    (409, body)
}

fn method_str(m: &Method) -> &'static str {
    match m {
        Method::Get => "GET",
        Method::Post => "POST",
        _ => "OTHER",
    }
}
