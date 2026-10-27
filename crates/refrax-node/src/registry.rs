//! The registry: who stores what, with how much at stake.
//!
//! On mainnet this state lives in the `refrax_registry` and `refrax_staking` programs on Solana.
//! Until they are deployed, [`Registry`] keeps the same accounts in memory and serves them over
//! HTTP, and [`RegistryClient`] talks to it. Nodes and clients only see the HTTP shape, so moving
//! to the on-chain programs swaps the backend without touching them.

use crate::types::*;
use crate::VERSION;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use refrax_core::identity::{ack_message, register_message, slash_message};
use refrax_core::{Hash, Manifest, ObjectId, Pubkey, Signature, FACETS, QUORUM};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

/// Target slot time, as on Solana.
pub const SLOT_MS: u64 = 400;
/// Slots per epoch on devnet (mainnet Solana epochs are 432,000 slots).
pub const SLOTS_PER_EPOCH: u64 = 432;
/// Minimum stake to sit on a committee.
pub const MIN_STAKE: u64 = 1_000;
/// Tokens removed by one slash.
pub const SLASH_AMOUNT: u64 = 500;

/// Why the registry refused a request.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// Bad signature.
    #[error("signature does not verify")]
    Signature,
    /// Not enough nodes for a full committee.
    #[error("committee has {0} of {FACETS} seats filled")]
    Committee(usize),
    /// Unknown object.
    #[error("object not found")]
    NotFound,
    /// Malformed object id.
    #[error("bad object id")]
    BadId,
    /// Manifest failed validation.
    #[error("manifest rejected: {0}")]
    Manifest(#[from] refrax_core::ManifestError),
    /// Not enough valid acks.
    #[error("{have} valid signatures, {QUORUM} needed")]
    Quorum {
        /// valid acks
        have: usize,
    },
    /// Vote rejected.
    #[error("vote rejected: {0}")]
    Vote(&'static str),
    /// Stake below the minimum.
    #[error("stake {0} is below the minimum of {MIN_STAKE}")]
    Stake(u64),
}

impl RegistryError {
    fn status(&self) -> StatusCode {
        match self {
            RegistryError::NotFound => StatusCode::NOT_FOUND,
            RegistryError::Committee(_) => StatusCode::CONFLICT,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl IntoResponse for RegistryError {
    fn into_response(self) -> Response {
        (
            self.status(),
            Json(serde_json::json!({ "error": self.to_string() })),
        )
            .into_response()
    }
}

type VoteKey = (Pubkey, ObjectId, u8, u64);

struct State0 {
    slot: u64,
    beacon: Hash,
    nodes: Vec<NodeEntry>,
    objects: BTreeMap<ObjectId, ObjectRecord>,
    order: Vec<ObjectId>,
    votes: HashMap<VoteKey, BTreeMap<Pubkey, Signature>>,
    settled: HashSet<VoteKey>,
    events: VecDeque<EventRecord>,
    seq: u64,
}

/// In-memory registry that mirrors the on-chain accounts.
pub struct Registry {
    cluster: String,
    state: Mutex<State0>,
    tx: broadcast::Sender<EventRecord>,
}

impl Registry {
    /// A fresh registry at slot 0.
    pub fn new(cluster: &str) -> Arc<Self> {
        let (tx, _) = broadcast::channel(1024);
        Arc::new(Registry {
            cluster: cluster.to_string(),
            state: Mutex::new(State0 {
                slot: 0,
                beacon: Hash::digest(&[b"refrax/genesis/v1", cluster.as_bytes()]),
                nodes: Vec::new(),
                objects: BTreeMap::new(),
                order: Vec::new(),
                votes: HashMap::new(),
                settled: HashSet::new(),
                events: VecDeque::new(),
                seq: 0,
            }),
            tx,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State0> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Advance one slot every [`SLOT_MS`] and roll the beacon forward.
    pub fn spawn_clock(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(SLOT_MS));
            loop {
                tick.tick().await;
                let mut s = me.lock();
                s.slot += 1;
                s.beacon = Hash::digest(&[b"refrax/beacon/v1", &s.beacon.0, &s.slot.to_le_bytes()]);
            }
        })
    }

    fn emit(&self, s: &mut State0, event: Event) {
        s.seq += 1;
        let rec = EventRecord {
            seq: s.seq,
            slot: s.slot,
            event,
        };
        s.events.push_back(rec.clone());
        if s.events.len() > 1000 {
            s.events.pop_front();
        }
        let _ = self.tx.send(rec);
    }

    /// Live event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<EventRecord> {
        self.tx.subscribe()
    }

    fn committee_of(s: &State0) -> Vec<Pubkey> {
        let mut eligible: Vec<&NodeEntry> =
            s.nodes.iter().filter(|n| n.stake >= MIN_STAKE).collect();
        eligible.sort_by(|a, b| {
            b.stake
                .cmp(&a.stake)
                .then(a.joined_slot.cmp(&b.joined_slot))
        });
        eligible
            .into_iter()
            .take(FACETS)
            .map(|n| n.pubkey)
            .collect()
    }

    /// Cluster summary.
    pub fn info(&self) -> RegistryInfo {
        let s = self.lock();
        RegistryInfo {
            cluster: self.cluster.clone(),
            version: VERSION.to_string(),
            slot: s.slot,
            epoch: s.slot / SLOTS_PER_EPOCH,
            beacon: s.beacon,
            nodes: s.nodes.len(),
            objects: s.objects.len(),
            certified: s
                .objects
                .values()
                .filter(|o| o.status == ObjectStatus::Certified)
                .count(),
            committee: Self::committee_of(&s),
        }
    }

    /// All nodes in join order.
    pub fn nodes(&self) -> Vec<NodeEntry> {
        self.lock().nodes.clone()
    }

    /// Bond stake and join, or update the endpoint of a known node.
    pub fn register_node(&self, req: RegisterNode) -> Result<NodeEntry, RegistryError> {
        if !req.pubkey.verify(
            &register_message(&req.pubkey, &req.url, req.stake),
            &req.signature,
        ) {
            return Err(RegistryError::Signature);
        }
        if req.stake < MIN_STAKE {
            return Err(RegistryError::Stake(req.stake));
        }
        let mut s = self.lock();
        if let Some(n) = s.nodes.iter_mut().find(|n| n.pubkey == req.pubkey) {
            n.url = req.url;
            return Ok(n.clone());
        }
        let entry = NodeEntry {
            pubkey: req.pubkey,
            url: req.url.clone(),
            stake: req.stake,
            slashed: 0,
            joined_slot: s.slot,
        };
        s.nodes.push(entry.clone());
        self.emit(
            &mut s,
            Event::NodeJoined {
                node: req.pubkey,
                url: req.url,
                stake: req.stake,
            },
        );
        Ok(entry)
    }

    /// Record a manifest and seat the current committee on it.
    pub fn register_object(&self, manifest: Manifest) -> Result<ObjectRecord, RegistryError> {
        manifest.validate()?;
        let id = manifest.id();
        let mut s = self.lock();
        if let Some(existing) = s.objects.get(&id) {
            return Ok(existing.clone());
        }
        let committee = Self::committee_of(&s);
        if committee.len() < FACETS {
            return Err(RegistryError::Committee(committee.len()));
        }
        let rec = ObjectRecord {
            id,
            manifest,
            committee,
            status: ObjectStatus::Registered,
            acks: Vec::new(),
            registered_slot: s.slot,
            certified_slot: None,
        };
        let size = rec.manifest.size;
        s.objects.insert(id, rec.clone());
        s.order.push(id);
        self.emit(&mut s, Event::ObjectRegistered { object: id, size });
        Ok(rec)
    }

    /// One object.
    pub fn object(&self, id: &ObjectId) -> Option<ObjectRecord> {
        self.lock().objects.get(id).cloned()
    }

    /// All objects, oldest first.
    pub fn objects(&self) -> Vec<ObjectRecord> {
        let s = self.lock();
        s.order
            .iter()
            .filter_map(|id| s.objects.get(id).cloned())
            .collect()
    }

    /// Accept a certificate: at least [`QUORUM`] valid receipts from the seated committee.
    pub fn certify(&self, id: &ObjectId, acks: Vec<Ack>) -> Result<ObjectRecord, RegistryError> {
        let mut s = self.lock();
        let rec = s.objects.get(id).ok_or(RegistryError::NotFound)?.clone();
        let mut valid: BTreeMap<u8, Ack> = rec.acks.iter().map(|a| (a.facet, a.clone())).collect();
        for a in acks {
            let f = a.facet as usize;
            if f >= FACETS || rec.committee[f] != a.node {
                continue;
            }
            if a.node.verify(
                &ack_message(id, a.facet, &rec.manifest.facet_roots[f]),
                &a.signature,
            ) {
                valid.insert(a.facet, a);
            }
        }
        let have = valid.len();
        let slot = s.slot;
        let entry = s.objects.get_mut(id).expect("checked above");
        // Receipts accumulate across calls, like signatures on an on-chain certificate account.
        entry.acks = valid.into_values().collect();
        if have < QUORUM {
            return Err(RegistryError::Quorum { have });
        }
        let newly = entry.status != ObjectStatus::Certified;
        if newly {
            entry.status = ObjectStatus::Certified;
            entry.certified_slot = Some(slot);
        }
        let out = entry.clone();
        if newly {
            self.emit(
                &mut s,
                Event::ObjectCertified {
                    object: *id,
                    signatures: out.acks.len(),
                },
            );
        }
        Ok(out)
    }

    /// Count a committee member's vote against a peer. At [`QUORUM`] votes the peer is slashed
    /// once for that object, facet and epoch.
    pub fn vote(&self, v: SlashVote) -> Result<VoteResult, RegistryError> {
        if !v.voter.verify(
            &slash_message(&v.accused, &v.object, v.facet, v.epoch),
            &v.signature,
        ) {
            return Err(RegistryError::Signature);
        }
        let mut s = self.lock();
        let epoch = s.slot / SLOTS_PER_EPOCH;
        if v.epoch + 1 < epoch || v.epoch > epoch {
            return Err(RegistryError::Vote("stale epoch"));
        }
        let rec = s.objects.get(&v.object).ok_or(RegistryError::NotFound)?;
        let f = v.facet as usize;
        if f >= FACETS || rec.committee[f] != v.accused {
            return Err(RegistryError::Vote("accused does not hold that facet"));
        }
        if v.voter == v.accused || !rec.committee.contains(&v.voter) {
            return Err(RegistryError::Vote("voter is not a committee peer"));
        }
        let key: VoteKey = (v.accused, v.object, v.facet, v.epoch);
        if s.settled.contains(&key) {
            return Ok(VoteResult {
                votes: QUORUM,
                needed: QUORUM,
                slashed: false,
            });
        }
        let ballots = s.votes.entry(key).or_default();
        let first = ballots.is_empty();
        ballots.insert(v.voter, v.signature);
        let votes = ballots.len();
        if first {
            self.emit(
                &mut s,
                Event::VoteOpened {
                    accused: v.accused,
                    voter: v.voter,
                    object: v.object,
                    facet: v.facet,
                },
            );
        }
        if votes < QUORUM {
            return Ok(VoteResult {
                votes,
                needed: QUORUM,
                slashed: false,
            });
        }
        s.votes.remove(&key);
        s.settled.insert(key);
        let (amount, stake) = match s.nodes.iter_mut().find(|n| n.pubkey == v.accused) {
            Some(n) => {
                let amount = SLASH_AMOUNT.min(n.stake);
                n.stake -= amount;
                n.slashed += amount;
                (amount, n.stake)
            }
            None => (0, 0),
        };
        self.emit(
            &mut s,
            Event::Slashed {
                node: v.accused,
                object: v.object,
                amount,
                stake,
                votes,
            },
        );
        Ok(VoteResult {
            votes,
            needed: QUORUM,
            slashed: true,
        })
    }

    /// Events after sequence number `since`.
    pub fn events_since(&self, since: u64) -> Vec<EventRecord> {
        self.lock()
            .events
            .iter()
            .filter(|e| e.seq > since)
            .cloned()
            .collect()
    }
}

// ───────────────────────────── HTTP ─────────────────────────────

#[derive(Deserialize)]
struct Since {
    #[serde(default)]
    since: u64,
}

fn parse_id(s: &str) -> Result<ObjectId, RegistryError> {
    s.parse().map_err(|_| RegistryError::BadId)
}

/// HTTP routes for the registry.
pub fn router(reg: Arc<Registry>) -> Router {
    Router::new()
        .route(
            "/v1/info",
            get(|State(r): State<Arc<Registry>>| async move { Json(r.info()) }),
        )
        .route(
            "/v1/nodes",
            get(|State(r): State<Arc<Registry>>| async move { Json(r.nodes()) }).post(
                |State(r): State<Arc<Registry>>, Json(req): Json<RegisterNode>| async move {
                    r.register_node(req).map(Json)
                },
            ),
        )
        .route(
            "/v1/objects",
            get(|State(r): State<Arc<Registry>>| async move { Json(r.objects()) }).post(
                |State(r): State<Arc<Registry>>, Json(req): Json<RegisterObject>| async move {
                    r.register_object(req.manifest).map(Json)
                },
            ),
        )
        .route(
            "/v1/objects/{id}",
            get(
                |State(r): State<Arc<Registry>>, Path(id): Path<String>| async move {
                    let id = parse_id(&id)?;
                    r.object(&id).map(Json).ok_or(RegistryError::NotFound)
                },
            ),
        )
        .route(
            "/v1/objects/{id}/certify",
            post(
                |State(r): State<Arc<Registry>>,
                 Path(id): Path<String>,
                 Json(req): Json<Certify>| async move {
                    let id = parse_id(&id)?;
                    r.certify(&id, req.acks).map(Json)
                },
            ),
        )
        .route(
            "/v1/slash",
            post(
                |State(r): State<Arc<Registry>>, Json(v): Json<SlashVote>| async move {
                    r.vote(v).map(Json)
                },
            ),
        )
        .route(
            "/v1/events",
            get(
                |State(r): State<Arc<Registry>>, Query(q): Query<Since>| async move {
                    Json(r.events_since(q.since))
                },
            ),
        )
        .with_state(reg)
}

// ───────────────────────────── client ─────────────────────────────

/// HTTP client for a registry.
#[derive(Clone, Debug)]
pub struct RegistryClient {
    base: String,
    http: reqwest::Client,
}

impl RegistryClient {
    /// Client for the registry at `base` (e.g. `http://127.0.0.1:7700`).
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("http client");
        RegistryClient {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    /// Base URL.
    pub fn url(&self) -> &str {
        &self.base
    }

    async fn read<T: DeserializeOwned>(resp: reqwest::Response) -> anyhow::Result<T> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp.json().await?);
        }
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let msg = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("request failed");
        anyhow::bail!("registry {}: {msg}", status.as_u16())
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .map_err(|e| self.unreachable(e))?;
        Self::read(resp).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let resp = self
            .http
            .post(format!("{}{path}", self.base))
            .json(body)
            .send()
            .await
            .map_err(|e| self.unreachable(e))?;
        Self::read(resp).await
    }

    fn unreachable(&self, e: reqwest::Error) -> anyhow::Error {
        anyhow::anyhow!(
            "registry at {} is not reachable ({e}). Is `refrax devnet up` running?",
            self.base
        )
    }

    /// `GET /v1/info`
    pub async fn info(&self) -> anyhow::Result<RegistryInfo> {
        self.get("/v1/info").await
    }

    /// `GET /v1/nodes`
    pub async fn nodes(&self) -> anyhow::Result<Vec<NodeEntry>> {
        self.get("/v1/nodes").await
    }

    /// `POST /v1/nodes`
    pub async fn register_node(&self, req: &RegisterNode) -> anyhow::Result<NodeEntry> {
        self.post("/v1/nodes", req).await
    }

    /// `GET /v1/objects`
    pub async fn objects(&self) -> anyhow::Result<Vec<ObjectRecord>> {
        self.get("/v1/objects").await
    }

    /// `GET /v1/objects/{id}`
    pub async fn object(&self, id: &ObjectId) -> anyhow::Result<ObjectRecord> {
        self.get(&format!("/v1/objects/{id}")).await
    }

    /// `POST /v1/objects`
    pub async fn register_object(&self, manifest: &Manifest) -> anyhow::Result<ObjectRecord> {
        self.post(
            "/v1/objects",
            &RegisterObject {
                manifest: manifest.clone(),
            },
        )
        .await
    }

    /// `POST /v1/objects/{id}/certify`
    pub async fn certify(&self, id: &ObjectId, acks: Vec<Ack>) -> anyhow::Result<ObjectRecord> {
        self.post(&format!("/v1/objects/{id}/certify"), &Certify { acks })
            .await
    }

    /// `POST /v1/slash`
    pub async fn vote(&self, v: &SlashVote) -> anyhow::Result<VoteResult> {
        self.post("/v1/slash", v).await
    }

    /// `GET /v1/events?since=`
    pub async fn events(&self, since: u64) -> anyhow::Result<Vec<EventRecord>> {
        self.get(&format!("/v1/events?since={since}")).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refrax_core::erasure::encode_object;
    use refrax_core::Keypair;

    fn join(reg: &Registry, n: usize) -> Vec<Keypair> {
        (0..n)
            .map(|i| {
                let kp = Keypair::from_seed([i as u8 + 1; 32]);
                let url = format!("http://node-{i}");
                let signature = kp.sign(&register_message(&kp.pubkey(), &url, 2500));
                reg.register_node(RegisterNode {
                    pubkey: kp.pubkey(),
                    url,
                    stake: 2500,
                    signature,
                })
                .unwrap();
                kp
            })
            .collect()
    }

    #[test]
    fn needs_a_full_committee() {
        let reg = Registry::new("test");
        join(&reg, 15);
        let m = Manifest::from_encoded(&encode_object(b"x"));
        assert!(matches!(
            reg.register_object(m),
            Err(RegistryError::Committee(15))
        ));
    }

    #[test]
    fn certify_needs_quorum_of_seated_members() {
        let reg = Registry::new("test");
        let keys = join(&reg, 16);
        let m = Manifest::from_encoded(&encode_object(b"hello"));
        let rec = reg.register_object(m.clone()).unwrap();
        let acks: Vec<Ack> = (0..16u8)
            .map(|f| {
                let kp = keys
                    .iter()
                    .find(|k| k.pubkey() == rec.committee[f as usize])
                    .unwrap();
                Ack {
                    facet: f,
                    node: kp.pubkey(),
                    signature: kp.sign(&ack_message(&rec.id, f, &m.facet_roots[f as usize])),
                }
            })
            .collect();
        assert!(matches!(
            reg.certify(&rec.id, acks[..10].to_vec()),
            Err(RegistryError::Quorum { have: 10 })
        ));
        let done = reg.certify(&rec.id, acks[10..].to_vec()).unwrap();
        assert_eq!(done.status, ObjectStatus::Certified);
        assert_eq!(done.acks.len(), 16);
    }

    #[test]
    fn eleven_votes_slash_once() {
        let reg = Registry::new("test");
        let keys = join(&reg, 16);
        let m = Manifest::from_encoded(&encode_object(b"slash me"));
        let rec = reg.register_object(m).unwrap();
        let accused = rec.committee[4];
        let voters: Vec<&Keypair> = keys.iter().filter(|k| k.pubkey() != accused).collect();
        let vote = |kp: &Keypair| SlashVote {
            accused,
            object: rec.id,
            facet: 4,
            epoch: 0,
            voter: kp.pubkey(),
            signature: kp.sign(&slash_message(&accused, &rec.id, 4, 0)),
        };
        for kp in &voters[..10] {
            assert!(!reg.vote(vote(kp)).unwrap().slashed);
        }
        // a repeated vote does not count twice
        assert_eq!(reg.vote(vote(voters[0])).unwrap().votes, 10);
        assert!(reg.vote(vote(voters[10])).unwrap().slashed);
        assert!(!reg.vote(vote(voters[11])).unwrap().slashed);
        let node = reg
            .nodes()
            .into_iter()
            .find(|n| n.pubkey == accused)
            .unwrap();
        assert_eq!((node.stake, node.slashed), (2000, 500));
        // the accused cannot vote, and nobody can vote for the wrong facet
        let own = keys.iter().find(|k| k.pubkey() == accused).unwrap();
        assert!(reg.vote(vote(own)).is_err());
    }
}
