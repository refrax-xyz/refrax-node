//! Node identities: ed25519 keys shown in base58, stored like `solana-keygen` keypairs
//! (a JSON array of 64 bytes: secret key then public key), so an operator can reuse one file.
//!
//! Every signed message starts with a domain tag, so a signature for one purpose can never be
//! replayed as another.

use crate::{Hash, ObjectId};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

/// Errors reading or parsing keys.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// The keypair file is not a JSON array of 64 bytes.
    #[error("keypair must be a JSON array of 64 bytes (solana-keygen format)")]
    Format,
    /// The public half does not belong to the secret half.
    #[error("keypair public key does not match its secret key")]
    Mismatch,
    /// Not a base58 32-byte public key.
    #[error("not a base58 ed25519 public key")]
    Pubkey,
    /// File system error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// An ed25519 public key, displayed in base58.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pubkey(pub [u8; 32]);

/// An ed25519 signature, displayed in base58.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature(pub [u8; 64]);

/// A node's signing key.
pub struct Keypair(SigningKey);

impl Keypair {
    /// A fresh random key from the operating system's RNG.
    pub fn generate() -> Self {
        Keypair(SigningKey::generate(&mut rand::rngs::OsRng))
    }

    /// Deterministic key from a 32-byte seed (tests and devnet fixtures).
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Keypair(SigningKey::from_bytes(&seed))
    }

    /// The public key.
    pub fn pubkey(&self) -> Pubkey {
        Pubkey(self.0.verifying_key().to_bytes())
    }

    /// Sign `msg`.
    pub fn sign(&self, msg: &[u8]) -> Signature {
        Signature(self.0.sign(msg).to_bytes())
    }

    /// `[s0, …, s31, p0, …, p31]`, the solana-keygen file format.
    pub fn to_json(&self) -> String {
        let mut bytes = self.0.to_bytes().to_vec();
        bytes.extend_from_slice(&self.pubkey().0);
        serde_json::to_string(&bytes).expect("bytes serialize")
    }

    /// Parse the solana-keygen file format.
    pub fn from_json(s: &str) -> Result<Self, IdentityError> {
        let bytes: Vec<u8> = serde_json::from_str(s).map_err(|_| IdentityError::Format)?;
        if bytes.len() != 64 {
            return Err(IdentityError::Format);
        }
        let secret: [u8; 32] = bytes[..32].try_into().expect("32 bytes");
        let kp = Keypair(SigningKey::from_bytes(&secret));
        if kp.pubkey().0[..] != bytes[32..] {
            return Err(IdentityError::Mismatch);
        }
        Ok(kp)
    }

    /// Read a keypair file.
    pub fn read(path: &Path) -> Result<Self, IdentityError> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    /// Write a keypair file, readable by the owner only on Unix.
    pub fn write(&self, path: &Path) -> Result<(), IdentityError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_json())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

impl Pubkey {
    /// Check `sig` over `msg`.
    pub fn verify(&self, msg: &[u8], sig: &Signature) -> bool {
        match VerifyingKey::from_bytes(&self.0) {
            Ok(vk) => vk
                .verify(msg, &ed25519_dalek::Signature::from_bytes(&sig.0))
                .is_ok(),
            Err(_) => false,
        }
    }

    /// `9WzD…AWWM`, for logs and tables.
    pub fn short(&self) -> String {
        let s = self.to_string();
        format!("{}…{}", &s[..4], &s[s.len() - 4..])
    }
}

/// What a node signs when it accepts a facet. Collected into an object's certificate.
pub fn ack_message(object: &ObjectId, facet: u8, facet_root: &Hash) -> Vec<u8> {
    [
        b"refrax/ack/v1".as_slice(),
        &object.0,
        &[facet],
        &facet_root.0,
    ]
    .concat()
}

/// What a committee member signs when a peer fails a storage challenge.
pub fn slash_message(accused: &Pubkey, object: &ObjectId, facet: u8, epoch: u64) -> Vec<u8> {
    [
        b"refrax/slash/v1".as_slice(),
        &accused.0,
        &object.0,
        &[facet],
        &epoch.to_le_bytes(),
    ]
    .concat()
}

/// What a node signs to join the network at `url`.
pub fn register_message(node: &Pubkey, url: &str, stake: u64) -> Vec<u8> {
    [
        b"refrax/register/v1".as_slice(),
        &node.0,
        &stake.to_le_bytes(),
        url.as_bytes(),
    ]
    .concat()
}

macro_rules! base58_type {
    ($t:ty, $n:expr, $err:expr) => {
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&bs58::encode(self.0).into_string())
            }
        }
        impl fmt::Debug for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($t), self)
            }
        }
        impl FromStr for $t {
            type Err = IdentityError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let v = bs58::decode(s).into_vec().map_err(|_| $err)?;
                let arr: [u8; $n] = v.try_into().map_err(|_| $err)?;
                Ok(Self(arr))
            }
        }
        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.to_string())
            }
        }
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

base58_type!(Pubkey, 32, IdentityError::Pubkey);
base58_type!(Signature, 64, IdentityError::Format);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solana_keygen_format_round_trip() {
        let kp = Keypair::generate();
        let json = kp.to_json();
        let back = Keypair::from_json(&json).unwrap();
        assert_eq!(back.pubkey(), kp.pubkey());
        let arr: Vec<u8> = serde_json::from_str(&json).unwrap();
        assert_eq!(arr.len(), 64);
        assert_eq!(&arr[32..], &kp.pubkey().0);
    }

    #[test]
    fn mismatched_halves_are_rejected() {
        let a = Keypair::generate();
        let b = Keypair::generate();
        let mut bytes: Vec<u8> = serde_json::from_str(&a.to_json()).unwrap();
        bytes[32..].copy_from_slice(&b.pubkey().0);
        assert!(matches!(
            Keypair::from_json(&serde_json::to_string(&bytes).unwrap()),
            Err(IdentityError::Mismatch)
        ));
    }

    #[test]
    fn signatures_are_domain_separated() {
        let kp = Keypair::from_seed([3; 32]);
        let obj = ObjectId([1; 32]);
        let ack = ack_message(&obj, 4, &Hash::default());
        let sig = kp.sign(&ack);
        assert!(kp.pubkey().verify(&ack, &sig));
        let slash = slash_message(&kp.pubkey(), &obj, 4, 0);
        assert!(!kp.pubkey().verify(&slash, &sig));
    }

    #[test]
    fn base58_round_trip() {
        let kp = Keypair::from_seed([9; 32]);
        let pk = kp.pubkey();
        assert_eq!(pk.to_string().parse::<Pubkey>().unwrap(), pk);
        let sig = kp.sign(b"x");
        assert_eq!(sig.to_string().parse::<Signature>().unwrap(), sig);
        assert!(pk.short().contains('…'));
    }
}
