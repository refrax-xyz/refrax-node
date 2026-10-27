# Refrax protocol

Version 1 · devnet preview. This document describes exactly what `refrax-core` and `refrax-node`
implement. If the code and this document disagree, it is a bug in one of them; please open an issue.

## Parameters

| Name | Value | Meaning |
|---|---|---|
| `FACETS` (n) | 16 | Facets per object, one per committee seat |
| `REBUILD` (k) | 5 | Facets that are enough to rebuild an object |
| `CHUNK` | 1024 bytes | Merkle leaf size and storage-proof answer size |
| `QUORUM` | 11 | ⌈⅔ · 16⌉: receipts to certify, votes to slash |
| `MIN_STAKE` | 1,000 REFRAX | Minimum stake to sit on a committee |
| `SLASH_AMOUNT` | 500 REFRAX | Stake removed per slash |
| Slot | 400 ms | Beacon period |
| Epoch | 432 slots on devnet | Slashing window (mainnet Solana epochs are 432,000 slots) |
| `PROTOCOL_VERSION` | 1 | Written into every manifest |

All hashes are SHA-256. All integers are little-endian. `‖` is concatenation.

## 1. Encoding

### Facet length

```
shard_len = ceil( ceil(size / k) / CHUNK ) · CHUNK        (at least CHUNK)
```

The object is zero-padded to `k · shard_len` bytes and cut into `k` data shards of `shard_len` bytes.

### Code

Arithmetic is in GF(2⁸) with reducing polynomial x⁸ + x⁴ + x³ + x² + 1 (`0x11d`) and generator 2.
Addition is XOR.

The generator matrix G is `n × k`:

```
G[i] = e_i                                  for i < k   (identity: the data facets)
G[k + r][c] = 1 / ((k + r) ⊕ c)             for r < n − k, c < k   (Cauchy rows: parity)
```

`x_r = k + r` and `y_c = c` are disjoint sets, so every entry is defined. Every square submatrix of a
Cauchy matrix is invertible, so any `k` rows of G are invertible: **any 5 facets rebuild the object**
(an MDS code). Facets 0–4 are the data itself, so a reader that gets them back needs no decoding.

Decoding picks `k` distinct facets, inverts the matching `k × k` rows of G by Gauss–Jordan elimination,
multiplies, concatenates, and truncates to `size`.

Storage overhead is n / k = 3.2×.

## 2. Commitments

### Merkle trees

```
leaf(data)  = H(0x00 ‖ data)
node(l, r)  = H(0x01 ‖ l ‖ r)
```

Levels are built left to right in pairs. An unpaired last node is **promoted** to the next level
unchanged (never duplicated), so two different leaf lists cannot produce the same root.

A proof is the list of sibling hashes from the leaf up. The verifier recomputes the path from
`(index, leaf_count)`: at each level, a node with an odd index takes its sibling on the left, an even
index takes it on the right if one exists, and a promoted node consumes nothing. Every sibling must be
consumed. A proof is therefore bound to one position.

### Facet root and object root

```
facet_root[i] = merkle root over facet i cut into CHUNK-byte leaves
root          = merkle root over facet_root[0..16] as leaves
```

### Manifest

What gets registered. It is a few hundred bytes for any object size.

```json
{
  "version": 1,
  "size": 145995,
  "shard_len": 29696,
  "facet_roots": ["d6d2bd61…1e36", "f0573e25…470f", "… 16 in total"],
  "root": "54b69fe8292009d7f05e362f18b02f4029718836cc95c7494d7cbe01f1836dc5"
}
```

A manifest is valid when: `version = 1`; exactly 16 facet roots; `shard_len` is a positive multiple of
`CHUNK`; `size ≤ k · shard_len`; and `root` is the merkle root of `facet_roots`.

### Object id

```
id = H("refrax/object/v1" ‖ root ‖ size:u64 ‖ shard_len:u32)
```

Shown in base58, like a Solana address: `BbBw21CYd9ukRes16jP3sv98PCdJmv4kPeBKe8gPgoJW`.

## 3. Identities and signed messages

Node identities are ed25519 keys. Public keys and signatures are shown in base58. Keypair files use the
`solana-keygen` format: a JSON array of 64 bytes, the 32-byte secret seed followed by the 32-byte public
key. The public half is checked against the secret half on load.

Every signed message starts with a domain tag, so a signature for one purpose cannot be replayed as
another:

| Message | Bytes | Signed by |
|---|---|---|
| Register | `"refrax/register/v1" ‖ node ‖ stake:u64 ‖ url` | the joining node |
| Receipt (ack) | `"refrax/ack/v1" ‖ object ‖ facet:u8 ‖ facet_root` | the node seated on that facet |
| Slash vote | `"refrax/slash/v1" ‖ accused ‖ object ‖ facet:u8 ‖ epoch:u64` | a committee peer |

## 4. Committees

The committee is the 16 nodes with stake ≥ `MIN_STAKE`, ordered by stake (descending), then by join
slot. When an object is registered, the current committee is written into its record:
`committee[i]` holds facet `i`. Registration fails while fewer than 16 seats can be filled.

## 5. Storing an object

1. The client encodes the object and builds the manifest.
2. The client registers the manifest. The registry validates it and records the committee.
3. The client sends facet `i` to `committee[i]`, all 16 in parallel.
4. A node accepts facet `i` only if it holds seat `i` for that object and the bytes have length
   `shard_len` and merkle root `facet_root[i]`. It stores the facet and returns a signed receipt.
5. The client submits the receipts. The registry keeps every receipt whose signer holds that seat and
   whose signature verifies. With **11 or more**, the object is **certified**. Receipts accumulate across
   submissions.

## 6. Reading an object

The reader fetches data facets 0–4 first, then parity facets in random order, keeping at most 5 requests
in flight. Each facet is checked against its registered root before use. Once 5 facets verify, the reader
decodes (or, if they are facets 0–4, concatenates) and truncates to `size`.

## 7. Proof of storage

### Beacon

The registry produces a beacon every slot:

```
beacon_0 = H("refrax/genesis/v1" ‖ cluster)
beacon_s = H("refrax/beacon/v1" ‖ beacon_{s−1} ‖ s:u64)
```

On mainnet the beacon is planned to be a recent Solana slot hash, which no single party controls. The
devnet beacon is produced by the local registry.

### Challenge

```
seed  = H("refrax/challenge/v1" ‖ beacon ‖ object ‖ facet:u8)
chunk = (first 8 bytes of seed as u64) mod (shard_len / CHUNK)
```

The holder answers with the chunk bytes and the merkle path from that chunk to `facet_root[facet]`.
The auditor recomputes `chunk` from the beacon and checks: same chunk index, exactly `CHUNK` bytes, and the
path verifies against the registered facet root. The chunk is unknown until the beacon exists, so a
node that deleted the facet cannot answer.

### Audits and slashing

Every committee member runs audit rounds (every 2 s plus jitter on devnet). In each round it samples up
to 4 certified objects it is seated on and challenges every other seat with the current beacon.

* A seat that fails **twice in a row** for the same object and facet gets a signed slash vote from that
  auditor, at most one vote per auditor per object, facet and epoch.
* The registry accepts a vote only from a committee peer of the accused (not the accused itself), for the
  facet the accused actually holds, for the current or previous epoch.
* At **11 distinct votes** the accused loses `SLASH_AMOUNT` of stake, once per object, facet and epoch.
  A node whose stake falls below `MIN_STAKE` drops out of future committees.

## 8. Mapping to Solana

The registry exposes the state the on-chain programs will hold. Planned account layout:

| Account | Seeds | Holds |
|---|---|---|
| `NodeAccount` | `["node", node]` | url, stake, slashed, joined slot |
| `ObjectAccount` | `["object", id]` | manifest, committee, status, receipts |
| `VoteAccount` | `["vote", accused, object, facet, epoch]` | voters until quorum |

Nodes and clients only depend on the HTTP shape in [api.md](api.md), so moving the registry on chain
changes the backend, not the node.
