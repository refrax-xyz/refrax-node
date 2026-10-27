# Changelog

All notable changes to this project. The format follows [Keep a Changelog](https://keepachangelog.com),
and the project uses [Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.1.0] · devnet preview

First public release.

### Added
- `refrax-core`: systematic Reed–Solomon (16, 5) over GF(2⁸), SHA-256 merkle commitments with
  domain-separated leaves, manifests and object ids, ed25519 identities in `solana-keygen` format,
  beacon-derived storage challenges.
- Storage node: seat and root checks on every facet, signed receipts, atomic facet store.
- Local registry mirroring the planned on-chain accounts: nodes and stake, objects and committees,
  certificates at 11 of 16 receipts, slash votes and slashing at 11 votes.
- Peer audits every round; two misses in a row lead to a signed slash vote.
- `refrax` CLI: `devnet up`, `devnet wipe`, `node start`, `put`, `get`, `audit`, `ls`, `status`,
  `inspect`, `keys`.
- Prometheus metrics at `/metrics`.
- Docker image, compose devnet, hardened systemd unit.
- CI on Linux and macOS with a devnet smoke test.
- Release builds for Linux (x86_64, arm64), macOS (Apple silicon, Intel) and Windows (x86_64), each
  with a SHA-256 file; node image on `ghcr.io/refrax-xyz/refrax-node`.
- `install.sh`: one-line install of the right build, checksum verified.
