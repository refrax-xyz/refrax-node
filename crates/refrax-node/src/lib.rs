//! # refrax-node
//!
//! Everything that runs on a machine:
//!
//! * [`node`] — the storage node: accepts facets assigned to it, answers storage challenges,
//!   audits its committee peers and votes to slash the ones that fail.
//! * [`registry`] — a local registry that mirrors the accounts of the Refrax programs on Solana
//!   (nodes, stake, objects, certificates, slashing votes) and produces a slot beacon.
//!   It backs the devnet until the programs are deployed.
//! * [`client`] — put, get and audit objects against a registry and its nodes.
//! * [`devnet`] — one command that starts a registry and a full committee on localhost.

#![forbid(unsafe_code)]

pub mod client;
pub mod devnet;
pub mod node;
pub mod registry;
pub mod store;
pub mod types;

/// Crate version, reported by `/v1/health`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Ticker of the token that is staked and slashed.
pub const TOKEN: &str = "REFRAX";
