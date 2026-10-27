//! Put, get and audit objects: what `refrax put`, `refrax get` and `refrax audit` do.

use crate::node::challenge_peer;
use crate::registry::RegistryClient;
use crate::types::*;
use anyhow::Context;
use refrax_core::erasure::{decode_object, Encoded};
use refrax_core::identity::ack_message;
use refrax_core::{Challenge, Pubkey, FACETS, REBUILD};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

/// A registry plus an HTTP client for its nodes.
#[derive(Clone, Debug)]
pub struct Network {
    /// Registry client.
    pub registry: RegistryClient,
    http: reqwest::Client,
}

/// Result of scattering facets to the committee.
#[derive(Debug, Default)]
pub struct Scatter {
    /// Verified receipts.
    pub acks: Vec<Ack>,
    /// Facets that were not accepted, with the reason.
    pub failed: Vec<(u8, String)>,
}

/// Result of fetching an object.
#[derive(Debug)]
pub struct Fetched {
    /// The object bytes.
    pub data: Vec<u8>,
    /// Facets used to rebuild it.
    pub used: Vec<u8>,
    /// Facets that could not be fetched or did not verify.
    pub failed: Vec<(u8, String)>,
}

/// One line of an audit.
#[derive(Debug)]
pub struct AuditRow {
    /// Facet index.
    pub facet: u8,
    /// Seated node.
    pub holder: Pubkey,
    /// Round-trip time.
    pub elapsed: Duration,
    /// `Ok` or why the proof failed.
    pub result: Result<(), String>,
}

impl Network {
    /// Connect to the registry at `url`.
    pub fn new(url: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("http client");
        Network {
            registry: RegistryClient::new(url),
            http,
        }
    }

    /// Pubkey → URL for every registered node.
    pub async fn urls(&self) -> anyhow::Result<HashMap<Pubkey, String>> {
        Ok(self
            .registry
            .nodes()
            .await?
            .into_iter()
            .map(|n| (n.pubkey, n.url))
            .collect())
    }

    /// Send facet `i` to `committee[i]` for all 16 in parallel and keep the receipts that verify.
    pub async fn scatter(&self, rec: &ObjectRecord, enc: &Encoded) -> anyhow::Result<Scatter> {
        let urls = self.urls().await?;
        let facets = Arc::new(enc.facets.clone());
        let mut set = JoinSet::new();
        for f in 0..FACETS {
            let holder = rec.committee[f];
            let url = urls.get(&holder).cloned();
            let (http, facets, id) = (self.http.clone(), Arc::clone(&facets), rec.id);
            set.spawn(async move {
                let f8 = f as u8;
                let Some(url) = url else {
                    return (f8, Err("node not registered".to_string()));
                };
                let resp = http
                    .put(format!("{url}/v1/facets/{id}/{f}"))
                    .body(facets[f].clone())
                    .send()
                    .await;
                let out = match resp {
                    Ok(r) if r.status().is_success() => {
                        r.json::<Ack>().await.map_err(|e| e.to_string())
                    }
                    Ok(r) => {
                        let code = r.status().as_u16();
                        let body: serde_json::Value = r.json().await.unwrap_or_default();
                        Err(format!(
                            "{code} {}",
                            body.get("error").and_then(|e| e.as_str()).unwrap_or("")
                        ))
                    }
                    Err(_) => Err("unreachable".to_string()),
                };
                (f8, out)
            });
        }
        let mut out = Scatter::default();
        while let Some(joined) = set.join_next().await {
            let (f, res) = joined?;
            match res {
                Ok(ack)
                    if ack.facet == f
                        && ack.node == rec.committee[f as usize]
                        && ack.node.verify(
                            &ack_message(&rec.id, f, &rec.manifest.facet_roots[f as usize]),
                            &ack.signature,
                        ) =>
                {
                    out.acks.push(ack)
                }
                Ok(_) => out.failed.push((f, "receipt does not verify".into())),
                Err(e) => out.failed.push((f, e)),
            }
        }
        out.acks.sort_by_key(|a| a.facet);
        out.failed.sort_by_key(|(f, _)| *f);
        Ok(out)
    }

    /// Fetch facets until any [`REBUILD`] of them verify, then rebuild the object.
    /// Data facets are tried first, so a healthy committee needs no decoding at all.
    pub async fn fetch(&self, rec: &ObjectRecord) -> anyhow::Result<Fetched> {
        let urls = self.urls().await?;
        let mut order: Vec<usize> = (0..REBUILD).collect();
        let mut parity: Vec<usize> = (REBUILD..FACETS).collect();
        parity.sort_by_key(|_| rand::random::<u32>());
        order.extend(parity);
        let mut queue = order.into_iter();
        let mut set = JoinSet::new();
        let spawn = |set: &mut JoinSet<(usize, Result<Vec<u8>, String>)>, f: usize| {
            let url = urls.get(&rec.committee[f]).cloned();
            let (http, id) = (self.http.clone(), rec.id);
            set.spawn(async move {
                let Some(url) = url else {
                    return (f, Err("node not registered".to_string()));
                };
                match http.get(format!("{url}/v1/facets/{id}/{f}")).send().await {
                    Ok(r) if r.status().is_success() => (
                        f,
                        r.bytes()
                            .await
                            .map(|b| b.to_vec())
                            .map_err(|e| e.to_string()),
                    ),
                    Ok(r) => (f, Err(format!("http {}", r.status().as_u16()))),
                    Err(_) => (f, Err("unreachable".to_string())),
                }
            });
        };
        for f in queue.by_ref().take(REBUILD) {
            spawn(&mut set, f);
        }
        let mut good: Vec<(usize, Vec<u8>)> = Vec::new();
        let mut failed = Vec::new();
        while let Some(joined) = set.join_next().await {
            let (f, res) = joined?;
            match res {
                Ok(bytes) if rec.manifest.check_facet(f, &bytes) => good.push((f, bytes)),
                Ok(_) => failed.push((f as u8, "does not match the registered root".to_string())),
                Err(e) => failed.push((f as u8, e)),
            }
            if good.len() >= REBUILD {
                set.abort_all();
                break;
            }
            if good.len() + set.len() < REBUILD {
                if let Some(next) = queue.next() {
                    spawn(&mut set, next);
                }
            }
        }
        if good.len() < REBUILD {
            anyhow::bail!("only {} of {REBUILD} facets could be fetched; the object cannot be rebuilt right now", good.len());
        }
        good.sort_by_key(|(f, _)| *f);
        let refs: Vec<(usize, &[u8])> = good.iter().map(|(f, b)| (*f, b.as_slice())).collect();
        let data = decode_object(&refs, rec.manifest.size, rec.manifest.shard_len as usize)
            .context("decode failed")?;
        failed.sort_by_key(|(f, _)| *f);
        Ok(Fetched {
            data,
            used: good.iter().map(|(f, _)| *f as u8).collect(),
            failed,
        })
    }

    /// Challenge every seat once with the current beacon.
    pub async fn audit(&self, rec: &ObjectRecord) -> anyhow::Result<Vec<AuditRow>> {
        let info = self.registry.info().await?;
        let urls = self.urls().await?;
        let mut set = JoinSet::new();
        for f in 0..FACETS {
            let holder = rec.committee[f];
            let url = urls.get(&holder).cloned();
            let ch = Challenge::derive(
                info.beacon,
                rec.id,
                f as u8,
                rec.manifest.chunks_per_facet(),
            );
            let (http, manifest) = (self.http.clone(), rec.manifest.clone());
            set.spawn(async move {
                let t = Instant::now();
                let result = match url {
                    Some(url) => challenge_peer(&http, &url, &ch, &manifest).await,
                    None => Err("not registered".into()),
                };
                AuditRow {
                    facet: f as u8,
                    holder,
                    elapsed: t.elapsed(),
                    result,
                }
            });
        }
        let mut rows = Vec::new();
        while let Some(r) = set.join_next().await {
            rows.push(r?);
        }
        rows.sort_by_key(|r| r.facet);
        Ok(rows)
    }

    /// Node status.
    pub async fn health(&self, url: &str) -> anyhow::Result<NodeHealth> {
        let r = self
            .http
            .get(format!("{url}/v1/health"))
            .timeout(Duration::from_secs(3))
            .send()
            .await?;
        Ok(r.error_for_status()?.json().await?)
    }

    /// Ask a devnet node to delete everything (needs `--allow-wipe`).
    pub async fn wipe(&self, url: &str) -> anyhow::Result<usize> {
        let r = self.http.delete(format!("{url}/v1/facets")).send().await?;
        if !r.status().is_success() {
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            anyhow::bail!(
                "{}",
                body.get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("wipe refused")
            );
        }
        let body: serde_json::Value = r.json().await?;
        Ok(body.get("wiped").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
    }
}
