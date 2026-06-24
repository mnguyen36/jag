//! Wire types shared by the jag server and client. Plain JSON over HTTP.
//!
//! Endpoints (base = the remote URL, e.g. `http://127.0.0.1:9418`):
//!   GET  /refs            -> RefsResponse        (lane tips)
//!   GET  /closure/<oid>   -> ClosureResponse     (objects reachable from oid)
//!   GET  /object/<oid>    -> raw object bytes | 404
//!   POST /object/<oid>    <- raw object bytes     (verified + stored)
//!   POST /missing         <- MissingRequest  -> MissingResponse
//!   POST /ref/<lane>      <- RefUpdateRequest -> RefUpdateResponse | 409

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct RefsResponse {
    pub lanes: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
pub struct ClosureResponse {
    pub oids: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct MissingRequest {
    pub oids: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct MissingResponse {
    pub missing: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct RefUpdateRequest {
    /// The tip the client believed the remote was at (informational).
    pub old: Option<String>,
    /// The tip the client wants the lane to point at.
    pub new: String,
}

#[derive(Serialize, Deserialize)]
pub struct RefUpdateResponse {
    pub ok: bool,
    pub reason: Option<String>,
}
