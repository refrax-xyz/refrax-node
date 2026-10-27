//! JSON types shared by the node API, the registry API and the client.

use refrax_core::{Hash, Manifest, ObjectId, Pubkey, Signature};
use serde::{Deserialize, Serialize};

/// A node as recorded by the registry (the `NodeAccount` of the staking program).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeEntry {
    /// Node identity.
    pub pubkey: Pubkey,
    /// Public HTTP endpoint.
    pub url: String,
    /// Active stake, in whole tokens.
    pub stake: u64,
    /// Total stake slashed so far.
    pub slashed: u64,
    /// Slot the node joined at.
    pub joined_slot: u64,
}

/// Join request, signed by the node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterNode {
    /// Node identity.
    pub pubkey: Pubkey,
    /// Public HTTP endpoint.
    pub url: String,
    /// Stake to bond.
    pub stake: u64,
    /// Signature over `register_message(pubkey, url, stake)`.
    pub signature: Signature,
}

/// Lifecycle of an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectStatus {
    /// Manifest recorded, facets not yet confirmed.
    Registered,
    /// A quorum of the committee signed for their facets.
    Certified,
}

/// A node's signed receipt for one facet.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ack {
    /// Facet index.
    pub facet: u8,
    /// Node that stored it.
    pub node: Pubkey,
    /// Signature over `ack_message(object, facet, facet_root)`.
    pub signature: Signature,
}

/// An object as recorded by the registry (the `ObjectAccount` of the registry program).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectRecord {
    /// Object id.
    pub id: ObjectId,
    /// Public commitment.
    pub manifest: Manifest,
    /// `committee[i]` holds facet `i`.
    pub committee: Vec<Pubkey>,
    /// Registered or certified.
    pub status: ObjectStatus,
    /// Receipts that make up the certificate.
    pub acks: Vec<Ack>,
    /// Slot of registration.
    pub registered_slot: u64,
    /// Slot of certification.
    pub certified_slot: Option<u64>,
}

/// Cluster summary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistryInfo {
    /// Cluster name.
    pub cluster: String,
    /// Software version.
    pub version: String,
    /// Current slot.
    pub slot: u64,
    /// Current epoch.
    pub epoch: u64,
    /// Beacon for storage challenges at this slot.
    pub beacon: Hash,
    /// Registered nodes.
    pub nodes: usize,
    /// Registered objects.
    pub objects: usize,
    /// Certified objects.
    pub certified: usize,
    /// Current committee, in seat order.
    pub committee: Vec<Pubkey>,
}

/// Body of `POST /v1/objects`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterObject {
    /// The object's manifest.
    pub manifest: Manifest,
}

/// Body of `POST /v1/objects/{id}/certify`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Certify {
    /// Receipts collected from the committee.
    pub acks: Vec<Ack>,
}

/// A committee member's vote to slash a peer that failed a storage challenge.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SlashVote {
    /// Node that failed.
    pub accused: Pubkey,
    /// Object audited.
    pub object: ObjectId,
    /// Facet audited.
    pub facet: u8,
    /// Epoch of the failure.
    pub epoch: u64,
    /// Voting member.
    pub voter: Pubkey,
    /// Signature over `slash_message(accused, object, facet, epoch)`.
    pub signature: Signature,
}

/// Registry answer to a vote.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoteResult {
    /// Distinct votes so far.
    pub votes: usize,
    /// Votes needed.
    pub needed: usize,
    /// Whether this vote completed the slash.
    pub slashed: bool,
}

/// Something that happened on the registry.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A node bonded stake and joined.
    NodeJoined {
        /// node
        node: Pubkey,
        /// endpoint
        url: String,
        /// bonded stake
        stake: u64,
    },
    /// A manifest was recorded.
    ObjectRegistered {
        /// object
        object: ObjectId,
        /// size in bytes
        size: u64,
    },
    /// An object reached its signature quorum.
    ObjectCertified {
        /// object
        object: ObjectId,
        /// valid signatures
        signatures: usize,
    },
    /// The first vote against a node for one facet.
    VoteOpened {
        /// accused node
        accused: Pubkey,
        /// first voter
        voter: Pubkey,
        /// object
        object: ObjectId,
        /// facet
        facet: u8,
    },
    /// Stake was slashed.
    Slashed {
        /// node
        node: Pubkey,
        /// object whose facet it could not prove
        object: ObjectId,
        /// tokens removed
        amount: u64,
        /// stake left
        stake: u64,
        /// votes that decided it
        votes: usize,
    },
}

/// An event with its sequence number and slot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventRecord {
    /// Monotonic sequence number.
    pub seq: u64,
    /// Slot it happened in.
    pub slot: u64,
    /// What happened.
    pub event: Event,
}

/// `GET /v1/health` on a node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeHealth {
    /// Node identity.
    pub pubkey: Pubkey,
    /// Software version.
    pub version: String,
    /// Facets on disk.
    pub facets: usize,
    /// Bytes on disk.
    pub bytes: u64,
    /// Registry the node reports to.
    pub registry: String,
    /// Whether the devnet-only wipe endpoint is on.
    pub wipe_enabled: bool,
}
