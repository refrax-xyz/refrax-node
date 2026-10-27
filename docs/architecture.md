# Architecture

```mermaid
flowchart LR
    subgraph client["Client · refrax put / get / audit"]
        enc["encode<br/>16 facets"]
        man["manifest<br/>roots + id"]
    end
    subgraph registry["Registry · Solana programs (devnet: local mirror)"]
        nodes[("nodes<br/>stake")]
        objects[("objects<br/>committee, receipts")]
        votes[("slash votes")]
        beacon(("beacon<br/>every slot"))
    end
    subgraph committee["Committee · 16 storage nodes"]
        n0["node 01<br/>facet 00"]
        n1["node 02<br/>facet 01"]
        nx["…"]
        n15["node 16<br/>facet 15"]
    end
    enc --> man
    man -- register --> objects
    enc -- "facet i → seat i" --> committee
    committee -- signed receipts --> client
    client -- certificate --> objects
    committee -- "challenge peers (beacon)" --> committee
    committee -- votes --> votes
    votes -- "11 votes → slash" --> nodes
    beacon -.-> committee
```

## Crates

| Crate | Depends on | Does |
|---|---|---|
| `refrax-core` | sha2, ed25519-dalek, bs58, serde | The math: GF(2⁸), Reed–Solomon, merkle, manifests, ids, keys, challenges. No I/O, no async. |
| `refrax-node` | core, tokio, axum, reqwest | Node server, auditor, facet store, registry (server + client), client operations, devnet. |
| `refrax-cli` | core, node, clap | The `refrax` binary and its terminal output. |

The core is kept free of I/O so the same code can run in a node, a client, a browser (WASM) or an on-chain
verifier.

## Store

```mermaid
sequenceDiagram
    participant C as Client
    participant R as Registry
    participant N as Node (seat i)
    C->>C: encode_object(bytes) → 16 facets
    C->>C: Manifest::from_encoded → roots, id
    C->>R: POST /v1/objects {manifest}
    R->>R: validate, seat current committee
    R-->>C: ObjectRecord (committee[0..16])
    par 16 facets in parallel
        C->>N: PUT /v1/facets/{id}/{i}  (shard_len bytes)
        N->>R: GET /v1/objects/{id}  (cached)
        N->>N: seat == me? root(bytes) == facet_root[i]?
        N->>N: write <data>/facets/{id}/{i}.facet
        N-->>C: Ack {facet, node, signature}
    end
    C->>C: verify each receipt
    C->>R: POST /v1/objects/{id}/certify {acks}
    R->>R: ≥ 11 valid receipts from seated nodes
    R-->>C: status: certified
```

## Read

```mermaid
sequenceDiagram
    participant C as Client
    participant R as Registry
    participant N as Nodes
    C->>R: GET /v1/objects/{id}, GET /v1/nodes
    C->>N: GET facets 0–4 first (5 in flight)
    N-->>C: bytes, or an error
    C->>C: check each facet against its root
    C->>N: on failure, ask the next parity seat
    C->>C: 5 good facets → decode → truncate to size
```

## Audit and slash

```mermaid
sequenceDiagram
    participant A as Auditor (committee peer)
    participant R as Registry
    participant T as Target seat
    loop every audit round
        A->>R: objects, info (beacon, epoch), nodes
        A->>A: Challenge::derive(beacon, object, facet)
        A->>T: POST /v1/challenge
        alt holds the facet
            T-->>A: chunk + merkle path
            A->>A: verify against facet_root
        else wiped, offline or wrong
            T-->>A: 404 / timeout / bad proof
            A->>A: misses += 1
            opt misses ≥ 2, first vote this epoch
                A->>R: POST /v1/slash (signed vote)
                R->>R: 11 votes → stake −500
            end
        end
    end
```

## The devnet

`refrax devnet up` starts, in one process:

* a registry on `:7700` with a 400 ms slot clock and the beacon,
* 16 nodes on `:7701–7716`, each with its own key in `.refrax/devnet/node-NN/keypair.json`,
  data directory and auditor (`--allow-wipe` on, so `refrax devnet wipe N` works).

Keys persist between runs; facets and registry state start fresh each time, like
`solana-test-validator --reset`.

## What changes on mainnet

Only the registry backend. Nodes and clients talk to the HTTP API in [api.md](api.md); on mainnet that API
is served by an indexer in front of the `refrax_registry` and `refrax_staking` programs, writes become
Solana transactions signed by the same ed25519 keys, and the beacon becomes a recent slot hash.
