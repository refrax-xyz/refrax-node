//! The storage node.
//!
//! * `PUT  /v1/facets/{object}/{facet}` — accept a facet if this node holds that seat and the
//!   bytes match the registered facet root; answer with a signed receipt.
//! * `GET  /v1/facets/{object}/{facet}` — serve a facet.
//! * `POST /v1/challenge` — answer a storage challenge with a chunk and its merkle path.
//! * `GET  /v1/health`, `GET /metrics` — status for operators and Prometheus.
//! * `DELETE /v1/facets` — devnet only (`--allow-wipe`): delete everything, then get slashed.
//!
//! In the background the node audits its committee peers and votes to slash any peer that
//! misses two challenges in a row.

use crate::registry::RegistryClient;
use crate::store::FacetStore;
use crate::types::*;
use crate::VERSION;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use refrax_core::challenge::{self, Challenge, StorageProof};
use refrax_core::identity::{ack_message, register_message, slash_message};
use refrax_core::{Keypair, Manifest, ObjectId, Pubkey, FACETS};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Largest facet a node accepts (objects up to 5 × this size).
pub const MAX_FACET: usize = 256 * 1024 * 1024;

/// Misses in a row before a node votes to slash a peer.
pub const MISSES_BEFORE_VOTE: u32 = 2;

/// Objects audited per round, picked at random among those this node is seated on.
pub const OBJECTS_PER_ROUND: usize = 4;

/// How a node runs.
pub struct NodeConfig {
    /// Address to listen on. Port 0 picks a free port.
    pub listen: SocketAddr,
    /// URL other parties use to reach this node. Derived from `listen` when empty.
    pub public_url: Option<String>,
    /// Registry base URL.
    pub registry: String,
    /// Where facets are kept.
    pub data_dir: PathBuf,
    /// Node identity.
    pub keypair: Arc<Keypair>,
    /// Stake to bond on registration.
    pub stake: u64,
    /// Enable `DELETE /v1/facets` (devnet demos only).
    pub allow_wipe: bool,
    /// Pause between audit rounds.
    pub audit_every: Duration,
}

#[derive(Default)]
struct Metrics {
    facets_accepted: AtomicU64,
    facets_rejected: AtomicU64,
    challenges_answered: AtomicU64,
    challenges_missed: AtomicU64,
    audits_passed: AtomicU64,
    audits_failed: AtomicU64,
    votes_cast: AtomicU64,
}

struct NodeState {
    keypair: Arc<Keypair>,
    me: Pubkey,
    store: FacetStore,
    registry: RegistryClient,
    http: reqwest::Client,
    records: Mutex<HashMap<ObjectId, ObjectRecord>>,
    metrics: Metrics,
    allow_wipe: bool,
}

/// A running node.
pub struct NodeHandle {
    /// Public URL.
    pub url: String,
    /// Node identity.
    pub pubkey: Pubkey,
    /// Bound address.
    pub addr: SocketAddr,
    /// Server task.
    pub server: JoinHandle<()>,
    /// Auditor task.
    pub auditor: JoinHandle<()>,
}

impl NodeHandle {
    /// Stop both tasks.
    pub fn abort(&self) {
        self.server.abort();
        self.auditor.abort();
    }
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("storage error: {e}"),
        )
    }
}

fn bad(status: StatusCode, msg: impl Into<String>) -> ApiError {
    ApiError(status, msg.into())
}

impl NodeState {
    async fn record(&self, id: &ObjectId) -> Result<ObjectRecord, ApiError> {
        if let Some(r) = self
            .records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
        {
            return Ok(r.clone());
        }
        let rec = self.registry.object(id).await.map_err(|e| {
            bad(
                StatusCode::BAD_GATEWAY,
                format!("cannot load object from registry: {e}"),
            )
        })?;
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(*id, rec.clone());
        Ok(rec)
    }
}

fn parse_id(s: &str) -> Result<ObjectId, ApiError> {
    s.parse()
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "bad object id"))
}

async fn health(State(s): State<Arc<NodeState>>) -> Result<Json<NodeHealth>, ApiError> {
    let (facets, bytes) = s.store.stats().await?;
    Ok(Json(NodeHealth {
        pubkey: s.me,
        version: VERSION.to_string(),
        facets,
        bytes,
        registry: s.registry.url().to_string(),
        wipe_enabled: s.allow_wipe,
    }))
}

async fn metrics(State(s): State<Arc<NodeState>>) -> Result<Response, ApiError> {
    let (facets, bytes) = s.store.stats().await?;
    let m = &s.metrics;
    let rows: [(&str, &str, &str, u64); 9] = [
        (
            "refrax_facets_stored",
            "gauge",
            "Facets on disk.",
            facets as u64,
        ),
        (
            "refrax_bytes_stored",
            "gauge",
            "Bytes of facets on disk.",
            bytes,
        ),
        (
            "refrax_facets_accepted_total",
            "counter",
            "Facets accepted and signed for.",
            m.facets_accepted.load(Relaxed),
        ),
        (
            "refrax_facets_rejected_total",
            "counter",
            "Facets refused (wrong seat or root).",
            m.facets_rejected.load(Relaxed),
        ),
        (
            "refrax_challenges_answered_total",
            "counter",
            "Storage challenges answered.",
            m.challenges_answered.load(Relaxed),
        ),
        (
            "refrax_challenges_missed_total",
            "counter",
            "Challenges for facets this node does not have.",
            m.challenges_missed.load(Relaxed),
        ),
        (
            "refrax_audits_passed_total",
            "counter",
            "Peer challenges that verified.",
            m.audits_passed.load(Relaxed),
        ),
        (
            "refrax_audits_failed_total",
            "counter",
            "Peer challenges that failed.",
            m.audits_failed.load(Relaxed),
        ),
        (
            "refrax_votes_cast_total",
            "counter",
            "Slash votes sent to the registry.",
            m.votes_cast.load(Relaxed),
        ),
    ];
    let mut out = String::new();
    for (name, kind, help, value) in rows {
        out.push_str(&format!(
            "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}\n"
        ));
    }
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], out).into_response())
}

async fn put_facet(
    State(s): State<Arc<NodeState>>,
    Path((object, facet)): Path<(String, u8)>,
    body: Bytes,
) -> Result<Json<Ack>, ApiError> {
    let id = parse_id(&object)?;
    if facet as usize >= FACETS {
        return Err(bad(StatusCode::BAD_REQUEST, "facet index out of range"));
    }
    let rec = s.record(&id).await?;
    let seat = rec.committee[facet as usize];
    if seat != s.me {
        s.metrics.facets_rejected.fetch_add(1, Relaxed);
        return Err(bad(
            StatusCode::FORBIDDEN,
            format!("facet {facet} is assigned to {seat}"),
        ));
    }
    if !rec.manifest.check_facet(facet as usize, &body) {
        s.metrics.facets_rejected.fetch_add(1, Relaxed);
        return Err(bad(
            StatusCode::UNPROCESSABLE_ENTITY,
            "facet does not match the registered root",
        ));
    }
    s.store.put(&id, facet, &body).await?;
    s.metrics.facets_accepted.fetch_add(1, Relaxed);
    let signature = s.keypair.sign(&ack_message(
        &id,
        facet,
        &rec.manifest.facet_roots[facet as usize],
    ));
    debug!(object = %id.short(), facet, "stored facet");
    Ok(Json(Ack {
        facet,
        node: s.me,
        signature,
    }))
}

async fn get_facet(
    State(s): State<Arc<NodeState>>,
    Path((object, facet)): Path<(String, u8)>,
) -> Result<Response, ApiError> {
    let id = parse_id(&object)?;
    match s.store.get(&id, facet).await? {
        Some(bytes) => {
            Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
        }
        None => Err(bad(StatusCode::NOT_FOUND, "facet not held by this node")),
    }
}

async fn answer(
    State(s): State<Arc<NodeState>>,
    Json(ch): Json<Challenge>,
) -> Result<Json<StorageProof>, ApiError> {
    let Some(bytes) = s.store.get(&ch.object, ch.facet).await? else {
        s.metrics.challenges_missed.fetch_add(1, Relaxed);
        return Err(bad(StatusCode::NOT_FOUND, "facet not held by this node"));
    };
    let proof = challenge::prove(&bytes, ch.chunk)
        .ok_or_else(|| bad(StatusCode::BAD_REQUEST, "chunk out of range"))?;
    s.metrics.challenges_answered.fetch_add(1, Relaxed);
    Ok(Json(proof))
}

async fn wipe(State(s): State<Arc<NodeState>>) -> Result<Json<serde_json::Value>, ApiError> {
    if !s.allow_wipe {
        return Err(bad(
            StatusCode::FORBIDDEN,
            "wipe is disabled; start the node with --allow-wipe (devnet only)",
        ));
    }
    let n = s.store.wipe().await?;
    warn!(facets = n, "wiped every facet on request");
    Ok(Json(serde_json::json!({ "wiped": n })))
}

fn router(state: Arc<NodeState>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/facets/{object}/{facet}", put(put_facet).get(get_facet))
        .route("/v1/facets", axum::routing::delete(wipe))
        .route("/v1/challenge", post(answer))
        .layer(DefaultBodyLimit::max(MAX_FACET))
        .with_state(state)
}

/// Challenge `url` and check the answer against `manifest`.
pub async fn challenge_peer(
    http: &reqwest::Client,
    url: &str,
    ch: &Challenge,
    manifest: &Manifest,
) -> Result<(), String> {
    let resp = http
        .post(format!("{url}/v1/challenge"))
        .json(ch)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|_| "unreachable".to_string())?;
    if !resp.status().is_success() {
        return Err(if resp.status() == StatusCode::NOT_FOUND {
            "facet missing".into()
        } else {
            format!("http {}", resp.status().as_u16())
        });
    }
    let proof: StorageProof = resp
        .json()
        .await
        .map_err(|_| "malformed proof".to_string())?;
    if challenge::verify(manifest, ch, &proof) {
        Ok(())
    } else {
        Err("proof does not verify".into())
    }
}

async fn audit_round(
    s: &NodeState,
    misses: &mut HashMap<(ObjectId, u8), u32>,
    voted: &mut HashSet<(ObjectId, u8, u64)>,
) -> anyhow::Result<()> {
    let objects = s.registry.objects().await?;
    let mut mine: Vec<&ObjectRecord> = objects
        .iter()
        .filter(|o| o.status == ObjectStatus::Certified && o.committee.contains(&s.me))
        .collect();
    if mine.is_empty() {
        return Ok(());
    }
    // A random sample, so every object gets audited without every round touching all of them.
    for i in (1..mine.len()).rev() {
        mine.swap(i, rand::random::<usize>() % (i + 1));
    }
    mine.truncate(OBJECTS_PER_ROUND);
    let info = s.registry.info().await?;
    let urls: HashMap<Pubkey, String> = s
        .registry
        .nodes()
        .await?
        .into_iter()
        .map(|n| (n.pubkey, n.url))
        .collect();
    for rec in mine {
        audit_object(s, rec, &info, &urls, misses, voted).await;
    }
    Ok(())
}

async fn audit_object(
    s: &NodeState,
    rec: &ObjectRecord,
    info: &RegistryInfo,
    urls: &HashMap<Pubkey, String>,
    misses: &mut HashMap<(ObjectId, u8), u32>,
    voted: &mut HashSet<(ObjectId, u8, u64)>,
) {
    for (f, holder) in rec.committee.iter().enumerate() {
        if *holder == s.me {
            continue;
        }
        let f = f as u8;
        let ch = Challenge::derive(info.beacon, rec.id, f, rec.manifest.chunks_per_facet());
        let result = match urls.get(holder) {
            Some(url) => challenge_peer(&s.http, url, &ch, &rec.manifest).await,
            None => Err("not registered".into()),
        };
        let key = (rec.id, f);
        match result {
            Ok(()) => {
                misses.remove(&key);
                s.metrics.audits_passed.fetch_add(1, Relaxed);
            }
            Err(why) => {
                s.metrics.audits_failed.fetch_add(1, Relaxed);
                let m = misses.entry(key).or_insert(0);
                *m += 1;
                debug!(peer = %holder.short(), facet = f, %why, misses = *m, "peer failed a challenge");
                if *m >= MISSES_BEFORE_VOTE && voted.insert((rec.id, f, info.epoch)) {
                    let vote = SlashVote {
                        accused: *holder,
                        object: rec.id,
                        facet: f,
                        epoch: info.epoch,
                        voter: s.me,
                        signature: s
                            .keypair
                            .sign(&slash_message(holder, &rec.id, f, info.epoch)),
                    };
                    s.metrics.votes_cast.fetch_add(1, Relaxed);
                    match s.registry.vote(&vote).await {
                        Ok(r) => {
                            info!(peer = %holder.short(), facet = f, votes = r.votes, needed = r.needed, "voted to slash")
                        }
                        Err(e) => warn!(peer = %holder.short(), "vote rejected: {e}"),
                    }
                }
            }
        }
    }
}

async fn audit_loop(s: Arc<NodeState>, every: Duration) {
    let mut misses = HashMap::new();
    let mut voted = HashSet::new();
    loop {
        let jitter = rand::random::<u64>() % (every.as_millis() as u64 / 2 + 1);
        tokio::time::sleep(every + Duration::from_millis(jitter)).await;
        if let Err(e) = audit_round(&s, &mut misses, &mut voted).await {
            debug!("audit round skipped: {e}");
        }
    }
}

/// Bind, register with the registry, and start serving and auditing in the background.
pub async fn spawn(cfg: NodeConfig) -> anyhow::Result<NodeHandle> {
    let listener = TcpListener::bind(cfg.listen)
        .await
        .map_err(|e| anyhow::anyhow!("cannot listen on {}: {e}", cfg.listen))?;
    let addr = listener.local_addr()?;
    let url = cfg.public_url.clone().unwrap_or_else(|| {
        let host = if addr.ip().is_unspecified() {
            "127.0.0.1".to_string()
        } else {
            addr.ip().to_string()
        };
        format!("http://{host}:{}", addr.port())
    });
    let store = FacetStore::open(&cfg.data_dir).await?;
    let registry = RegistryClient::new(&cfg.registry);
    let me = cfg.keypair.pubkey();

    let req = RegisterNode {
        pubkey: me,
        url: url.clone(),
        stake: cfg.stake,
        signature: cfg.keypair.sign(&register_message(&me, &url, cfg.stake)),
    };
    let mut attempt = 0;
    loop {
        match registry.register_node(&req).await {
            Ok(_) => break,
            Err(e) if attempt < 60 && e.to_string().contains("not reachable") => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => return Err(e),
        }
    }

    let state = Arc::new(NodeState {
        keypair: cfg.keypair,
        me,
        store,
        registry,
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?,
        records: Mutex::new(HashMap::new()),
        metrics: Metrics::default(),
        allow_wipe: cfg.allow_wipe,
    });
    let app = router(Arc::clone(&state));
    let server = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!("node server stopped: {e}");
        }
    });
    let auditor = tokio::spawn(audit_loop(state, cfg.audit_every));
    Ok(NodeHandle {
        url,
        pubkey: me,
        addr,
        server,
        auditor,
    })
}
