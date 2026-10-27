# Security

refrax-node is a **devnet preview** and has **not been audited**. Do not use it to store anything you
cannot afford to lose.

## Reporting a vulnerability

Please report privately through
[GitHub security advisories](https://github.com/refrax-xyz/refrax-node/security/advisories/new).
Do not open a public issue.

Include what is affected (crate, file, endpoint), how to reproduce it, and what an attacker gains. We aim to
acknowledge reports within 72 hours and to agree on a disclosure date with you.

## In scope

- Accepting a facet, receipt, vote or proof that should be rejected
- Signatures or messages that can be replayed across purposes or objects
- Ways to slash an honest node, or to keep stake while not storing data
- Reading or corrupting another object's facets
- Crashes or unbounded resource use reachable from the HTTP API

## Out of scope

- The devnet-only `DELETE /v1/facets` endpoint (it is off unless `--allow-wipe` is passed)
- The local registry trusting its own clock and beacon (it is a devnet stand-in for the Solana programs)
- Denial of service that needs more bandwidth than the attacker has

## Keys

A node key in `solana-keygen` format controls the node's identity and stake. If one leaks, stop the node,
move the stake, and start with a new key.
