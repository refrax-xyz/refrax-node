//! A 32-byte SHA-256 digest with hex formatting.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;

/// A SHA-256 digest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// Hash the concatenation of `parts`.
    pub fn digest(parts: &[&[u8]]) -> Hash {
        let mut h = Sha256::new();
        for p in parts {
            h.update(p);
        }
        Hash(h.finalize().into())
    }

    /// Full lowercase hex.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// `3f2a…09c1`, for logs and tables.
    pub fn short(&self) -> String {
        let h = self.to_hex();
        format!("{}…{}", &h[..4], &h[60..])
    }

    /// The first eight bytes as a little-endian integer.
    pub fn to_u64(&self) -> u64 {
        u64::from_le_bytes(self.0[..8].try_into().expect("8 bytes"))
    }
}

/// Error returned when a string is not 64 hex characters.
#[derive(Debug, thiserror::Error)]
#[error("expected 64 hex characters")]
pub struct HashParseError;

impl FromStr for Hash {
    type Err = HashParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s).map_err(|_| HashParseError)?;
        Ok(Hash(bytes.try_into().map_err(|_| HashParseError)?))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.short())
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let h = Hash::digest(&[b"refrax"]);
        assert_eq!(h.to_hex().parse::<Hash>().unwrap(), h);
        assert_eq!(h.short().chars().count(), 9);
    }

    #[test]
    fn digest_concatenates() {
        assert_eq!(Hash::digest(&[b"ab", b"c"]), Hash::digest(&[b"abc"]));
    }
}
