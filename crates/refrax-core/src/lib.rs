//! # refrax-core
//!
//! The math and the wire formats every Refrax component agrees on:
//!
//! * [`erasure`] cuts an object into [`FACETS`] facets with a systematic Reed–Solomon code over
//!   GF(2⁸). Any [`REBUILD`] of them restore the object.
//! * [`merkle`] commits to every facet in [`CHUNK`]-sized leaves, and to the object through the
//!   sixteen facet roots.
//! * [`identity`] holds ed25519 node identities, stored in the same JSON format as `solana-keygen`.
//! * [`challenge`] derives unpredictable storage challenges from a beacon and checks the answers.
//!
//! Nothing in this crate touches the network or the disk, so it can be reused by nodes, clients,
//! auditors and on-chain verifiers alike.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod challenge;
pub mod erasure;
pub mod gf256;
pub mod hash;
pub mod identity;
pub mod merkle;
pub mod object;

pub use challenge::{Challenge, StorageProof};
pub use erasure::{Codec, CodecError, Encoded};
pub use hash::Hash;
pub use identity::{Keypair, Pubkey, Signature};
pub use object::{Manifest, ManifestError, ObjectId};

/// Facets every object is cut into. Each one lives on a different node of the committee.
pub const FACETS: usize = 16;

/// Facets that are enough to rebuild an object. The other `FACETS - REBUILD` may be offline.
pub const REBUILD: usize = 5;

/// Size of a merkle leaf and of a storage-proof answer, in bytes.
pub const CHUNK: usize = 1024;

/// Committee signatures needed to certify an object or slash a node: two thirds of 16, rounded up.
pub const QUORUM: usize = 11;

/// Protocol version written into every manifest.
pub const PROTOCOL_VERSION: u8 = 1;
