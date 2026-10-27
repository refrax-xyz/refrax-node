# refrax-core

The math every Refrax component shares. No I/O, no async, `#![forbid(unsafe_code)]`.

| Module | |
|---|---|
| `gf256` | GF(2⁸) arithmetic, tables built at compile time |
| `erasure` | Systematic Reed–Solomon (16, 5) with a Cauchy matrix: any 5 facets rebuild the object |
| `merkle` | SHA-256 trees with domain-separated leaves, promotion instead of duplication, position-bound proofs |
| `object` | Manifests and object ids |
| `identity` | ed25519 keys in `solana-keygen` format, domain-separated messages |
| `challenge` | Beacon-derived storage challenges and their verification |

```sh
cargo run -p refrax-core --example roundtrip -- <file>
```

Specification: [docs/protocol.md](../../docs/protocol.md).
