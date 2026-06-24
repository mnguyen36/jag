//! Blocking HTTP client for talking to a `jag serve` endpoint.

use std::io::Read;

use anyhow::{anyhow, Result};

use crate::protocol::{
    ClosureResponse, MissingRequest, MissingResponse, RefUpdateRequest, RefUpdateResponse,
    RefsResponse,
};

pub struct Client {
    base: String,
}

impl Client {
    pub fn new(url: &str) -> Self {
        Client {
            base: url.trim_end_matches('/').to_string(),
        }
    }

    /// Remote lane tips.
    pub fn get_refs(&self) -> Result<std::collections::BTreeMap<String, String>> {
        let body = ureq::get(&format!("{}/refs", self.base))
            .call()
            .map_err(reqerr)?
            .into_string()?;
        let parsed: RefsResponse = serde_json::from_str(&body)?;
        Ok(parsed.lanes)
    }

    /// All objects reachable from `oid`, as computed by the remote.
    pub fn get_closure(&self, oid: &str) -> Result<Vec<String>> {
        let body = ureq::get(&format!("{}/closure/{oid}", self.base))
            .call()
            .map_err(reqerr)?
            .into_string()?;
        let parsed: ClosureResponse = serde_json::from_str(&body)?;
        Ok(parsed.oids)
    }

    /// Raw object bytes, or `None` if the remote doesn't have it (404).
    pub fn get_object(&self, oid: &str) -> Result<Option<Vec<u8>>> {
        match ureq::get(&format!("{}/object/{oid}", self.base)).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader().read_to_end(&mut buf)?;
                Ok(Some(buf))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(anyhow!("GET object {oid}: {e}")),
        }
    }

    /// Upload raw object bytes.
    pub fn post_object(&self, oid: &str, bytes: &[u8]) -> Result<()> {
        ureq::post(&format!("{}/object/{oid}", self.base))
            .send_bytes(bytes)
            .map_err(reqerr)?;
        Ok(())
    }

    /// Of `oids`, which the remote is missing.
    pub fn missing(&self, oids: &[String]) -> Result<Vec<String>> {
        let body = serde_json::to_string(&MissingRequest {
            oids: oids.to_vec(),
        })?;
        let resp = ureq::post(&format!("{}/missing", self.base))
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(reqerr)?
            .into_string()?;
        let parsed: MissingResponse = serde_json::from_str(&resp)?;
        Ok(parsed.missing)
    }

    /// Request the remote move `lane` to `new`. A 409 is reported as
    /// `ok = false` with a reason rather than an error.
    pub fn update_ref(
        &self,
        lane: &str,
        old: Option<&str>,
        new: &str,
    ) -> Result<RefUpdateResponse> {
        let body = serde_json::to_string(&RefUpdateRequest {
            old: old.map(|s| s.to_string()),
            new: new.to_string(),
        })?;
        match ureq::post(&format!("{}/ref/{lane}", self.base))
            .set("Content-Type", "application/json")
            .send_string(&body)
        {
            Ok(resp) => Ok(serde_json::from_str(&resp.into_string()?)?),
            Err(ureq::Error::Status(409, resp)) => {
                let s = resp.into_string().unwrap_or_default();
                Ok(serde_json::from_str(&s).unwrap_or(RefUpdateResponse {
                    ok: false,
                    reason: Some("remote diverged".to_string()),
                }))
            }
            Err(e) => Err(anyhow!("update ref {lane}: {e}")),
        }
    }
}

fn reqerr(e: ureq::Error) -> anyhow::Error {
    anyhow!("{e}")
}
