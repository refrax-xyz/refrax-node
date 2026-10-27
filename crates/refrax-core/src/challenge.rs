//! Proof of storage.
//!
//! An auditor takes a public beacon (a recent Solana slot hash on mainnet, the local registry's
//! beacon on devnet) and derives which 1 KiB chunk of a facet the holder must return. The answer
//! is the chunk plus its merkle path to the facet root that was registered on chain. Since the
//! chunk is unknown until the beacon exists, a node that deleted the facet cannot answer.

use crate::merkle::{self, MerkleTree};
use crate::{Hash, Manifest, ObjectId, CHUNK};
use serde::{Deserialize, Serialize};

/// Which chunk of which facet must be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Challenge {
    /// Object under audit.
    pub object: ObjectId,
    /// Facet index, 0–15.
    pub facet: u8,
    /// Beacon the chunk index was derived from.
    pub beacon: Hash,
    /// Chunk the holder must return.
    pub chunk: u32,
}

impl Challenge {
    /// chunk = H("refrax/challenge/v1" ‖ beacon ‖ object ‖ facet) mod chunks
    pub fn derive(beacon: Hash, object: ObjectId, facet: u8, chunks: usize) -> Challenge {
        let seed = Hash::digest(&[b"refrax/challenge/v1", &beacon.0, &object.0, &[facet]]);
        let chunk = (seed.to_u64() % chunks.max(1) as u64) as u32;
        Challenge {
            object,
            facet,
            beacon,
            chunk,
        }
    }
}

/// A holder's answer to a [`Challenge`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageProof {
    /// Chunk index answered.
    pub chunk: u32,
    /// The chunk bytes, hex encoded on the wire.
    #[serde(with = "hex_bytes")]
    pub data: Vec<u8>,
    /// Merkle path from the chunk to the facet root.
    pub siblings: Vec<Hash>,
}

/// Answer a challenge from the stored facet.
pub fn prove(facet: &[u8], chunk: u32) -> Option<StorageProof> {
    let start = chunk as usize * CHUNK;
    let data = facet.get(start..start + CHUNK)?.to_vec();
    let tree = MerkleTree::from_chunks(facet, CHUNK);
    Some(StorageProof {
        chunk,
        siblings: tree.proof(chunk as usize),
        data,
    })
}

/// Check an answer against the manifest registered for the object.
pub fn verify(manifest: &Manifest, challenge: &Challenge, proof: &StorageProof) -> bool {
    let facet = challenge.facet as usize;
    let Some(root) = manifest.facet_roots.get(facet) else {
        return false;
    };
    let expected = Challenge::derive(
        challenge.beacon,
        challenge.object,
        challenge.facet,
        manifest.chunks_per_facet(),
    );
    proof.chunk == expected.chunk
        && challenge.chunk == expected.chunk
        && proof.data.len() == CHUNK
        && merkle::verify(
            root,
            proof.chunk as usize,
            manifest.chunks_per_facet(),
            merkle::leaf(&proof.data),
            &proof.siblings,
        )
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        hex::decode(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::erasure::encode_object;

    fn setup() -> (Manifest, Vec<Vec<u8>>) {
        let data: Vec<u8> = (0..60_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let enc = encode_object(&data);
        (Manifest::from_encoded(&enc), enc.facets)
    }

    #[test]
    fn honest_holder_passes() {
        let (m, facets) = setup();
        for round in 0..20u8 {
            let beacon = Hash::digest(&[&[round]]);
            for f in 0..16u8 {
                let ch = Challenge::derive(beacon, m.id(), f, m.chunks_per_facet());
                let proof = prove(&facets[f as usize], ch.chunk).unwrap();
                assert!(verify(&m, &ch, &proof));
            }
        }
    }

    #[test]
    fn wrong_chunk_or_bytes_fail() {
        let (m, facets) = setup();
        let ch = Challenge::derive(Hash::digest(&[b"b"]), m.id(), 3, m.chunks_per_facet());
        let other = (ch.chunk + 1) % m.chunks_per_facet() as u32;
        let wrong_index = prove(&facets[3], other).unwrap();
        assert!(!verify(&m, &ch, &wrong_index));
        let mut forged = prove(&facets[3], ch.chunk).unwrap();
        forged.data[0] ^= 1;
        assert!(!verify(&m, &ch, &forged));
        let other_facet = prove(&facets[4], ch.chunk).unwrap();
        assert!(!verify(&m, &ch, &other_facet));
    }

    #[test]
    fn beacon_changes_the_chunk() {
        let (m, _) = setup();
        let picks: std::collections::HashSet<u32> = (0..64u8)
            .map(|i| {
                Challenge::derive(Hash::digest(&[&[i]]), m.id(), 0, m.chunks_per_facet()).chunk
            })
            .collect();
        assert!(picks.len() > 3, "challenges should spread across chunks");
    }
}
