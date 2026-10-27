//! Object manifests and ids.
//!
//! The manifest is what gets registered on Solana: size, facet length, the sixteen facet roots and
//! the object root over them. It is a few hundred bytes no matter how large the object is.

use crate::erasure::Encoded;
use crate::merkle::{self, MerkleTree};
use crate::{Hash, CHUNK, FACETS, PROTOCOL_VERSION, REBUILD};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// A 32-byte object id, shown in base58 like a Solana address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(pub [u8; 32]);

impl ObjectId {
    /// `7xKX…gAsU`, for logs and tables.
    pub fn short(&self) -> String {
        let s = self.to_string();
        format!("{}…{}", &s[..4], &s[s.len() - 4..])
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&bs58::encode(self.0).into_string())
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({self})")
    }
}

/// Error returned when a string is not a base58 32-byte id.
#[derive(Debug, Error)]
#[error("not a base58-encoded 32-byte id")]
pub struct IdParseError;

impl FromStr for ObjectId {
    type Err = IdParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let v = bs58::decode(s).into_vec().map_err(|_| IdParseError)?;
        Ok(ObjectId(v.try_into().map_err(|_| IdParseError)?))
    }
}

impl Serialize for ObjectId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Why a manifest was rejected.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    /// Unknown protocol version.
    #[error("unsupported manifest version {0}")]
    Version(u8),
    /// Not exactly 16 facet roots.
    #[error("expected {FACETS} facet roots, got {0}")]
    FacetCount(usize),
    /// Facet length is not a positive multiple of the chunk size.
    #[error("facet length {0} is not a positive multiple of {CHUNK}")]
    ShardLen(u32),
    /// The size does not fit in five facets of this length.
    #[error("size {size} does not fit in {REBUILD} facets of {shard_len} bytes")]
    Size {
        /// declared size
        size: u64,
        /// declared facet length
        shard_len: u32,
    },
    /// `root` is not the merkle root of `facet_roots`.
    #[error("object root does not match the facet roots")]
    Root,
}

/// The public commitment to an object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Protocol version.
    pub version: u8,
    /// Original size in bytes.
    pub size: u64,
    /// Length of every facet in bytes.
    pub shard_len: u32,
    /// Merkle root of each facet over 1 KiB chunks, in facet order.
    pub facet_roots: Vec<Hash>,
    /// Merkle root over `facet_roots`.
    pub root: Hash,
}

/// Merkle root of one facet.
pub fn facet_root(bytes: &[u8]) -> Hash {
    MerkleTree::from_chunks(bytes, CHUNK).root()
}

impl Manifest {
    /// Commit to an encoded object.
    pub fn from_encoded(e: &Encoded) -> Self {
        let facet_roots: Vec<Hash> = e.facets.iter().map(|f| facet_root(f)).collect();
        let root = MerkleTree::from_leaves(facet_roots.clone()).root();
        Manifest {
            version: PROTOCOL_VERSION,
            size: e.size,
            shard_len: e.shard_len as u32,
            facet_roots,
            root,
        }
    }

    /// The object id: H("refrax/object/v1" ‖ root ‖ size ‖ shard_len).
    pub fn id(&self) -> ObjectId {
        ObjectId(
            Hash::digest(&[
                b"refrax/object/v1",
                &self.root.0,
                &self.size.to_le_bytes(),
                &self.shard_len.to_le_bytes(),
            ])
            .0,
        )
    }

    /// Leaves per facet tree.
    pub fn chunks_per_facet(&self) -> usize {
        self.shard_len as usize / CHUNK
    }

    /// Check internal consistency. Registries and nodes run this before accepting a manifest.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ManifestError::Version(self.version));
        }
        if self.facet_roots.len() != FACETS {
            return Err(ManifestError::FacetCount(self.facet_roots.len()));
        }
        if self.shard_len == 0 || !(self.shard_len as usize).is_multiple_of(CHUNK) {
            return Err(ManifestError::ShardLen(self.shard_len));
        }
        if self.size > self.shard_len as u64 * REBUILD as u64 {
            return Err(ManifestError::Size {
                size: self.size,
                shard_len: self.shard_len,
            });
        }
        if MerkleTree::from_leaves(self.facet_roots.clone()).root() != self.root {
            return Err(ManifestError::Root);
        }
        Ok(())
    }

    /// Does `bytes` match facet `index` of this manifest?
    pub fn check_facet(&self, index: usize, bytes: &[u8]) -> bool {
        index < FACETS
            && bytes.len() == self.shard_len as usize
            && facet_root(bytes) == self.facet_roots[index]
    }

    /// Proof that facet `index`'s root is part of the object root.
    pub fn facet_proof(&self, index: usize) -> Vec<Hash> {
        MerkleTree::from_leaves(self.facet_roots.clone()).proof(index)
    }

    /// Verify a facet-root proof against this manifest's root.
    pub fn verify_facet_proof(&self, index: usize, facet_root: Hash, proof: &[Hash]) -> bool {
        merkle::verify(&self.root, index, FACETS, facet_root, proof)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::erasure::encode_object;

    #[test]
    fn manifest_validates_and_ids_round_trip() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let enc = encode_object(&data);
        let m = Manifest::from_encoded(&enc);
        m.validate().unwrap();
        let id = m.id();
        assert_eq!(id.to_string().parse::<ObjectId>().unwrap(), id);
        for i in 0..FACETS {
            assert!(m.check_facet(i, &enc.facets[i]));
            assert!(m.verify_facet_proof(i, m.facet_roots[i], &m.facet_proof(i)));
        }
        assert!(!m.check_facet(0, &enc.facets[1]));
    }

    #[test]
    fn tampering_is_caught() {
        let enc = encode_object(b"refrax");
        let mut m = Manifest::from_encoded(&enc);
        m.facet_roots[3] = Hash::digest(&[b"x"]);
        assert_eq!(m.validate(), Err(ManifestError::Root));
        let mut m2 = Manifest::from_encoded(&enc);
        m2.size = 1 << 40;
        assert!(matches!(m2.validate(), Err(ManifestError::Size { .. })));
    }

    #[test]
    fn json_shape() {
        let m = Manifest::from_encoded(&encode_object(b"json"));
        let v: serde_json::Value = serde_json::to_value(&m).unwrap();
        assert_eq!(v["facet_roots"].as_array().unwrap().len(), FACETS);
        assert_eq!(serde_json::from_value::<Manifest>(v).unwrap(), m);
    }
}
