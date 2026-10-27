# HTTP API

All bodies are JSON unless stated. Errors are `{"error": "<reason>"}` with a 4xx or 5xx status.
Hashes are hex; ids, public keys and signatures are base58. Examples are real responses from a devnet.

## Node

### `GET /v1/health`

```json
{
  "pubkey": "DABraZU11NsaFwViArQnFdvXfABto6DFtNLy8woRPkZj",
  "version": "0.1.0",
  "facets": 1,
  "bytes": 29696,
  "registry": "http://127.0.0.1:7700",
  "wipe_enabled": true
}
```

### `PUT /v1/facets/{object}/{facet}`

Body: the raw facet, `application/octet-stream`, exactly `shard_len` bytes.

| Status | When |
|---|---|
| 200 | Stored. Body is the signed receipt. |
| 403 | This node does not hold seat `facet` for that object |
| 422 | Length or merkle root does not match the manifest |
| 502 | The registry could not be reached to load the manifest |

```json
{
  "facet": 0,
  "node": "DABraZU11NsaFwViArQnFdvXfABto6DFtNLy8woRPkZj",
  "signature": "5xbz9NzLBwkKjKou5v7dgqUcDpTpRoBE66sHS9Qgv6iKqyq6Z5krLKHgxfNbsh75RLNbdmxsbuncVYnYfTKPx4Dk"
}
```

The signature is over `"refrax/ack/v1" ‖ object ‖ facet ‖ facet_root` ([protocol §3](protocol.md#3-identities-and-signed-messages)).

### `GET /v1/facets/{object}/{facet}`

Returns the facet bytes, or 404.

### `POST /v1/challenge`

```json
{
  "object": "BbBw21CYd9ukRes16jP3sv98PCdJmv4kPeBKe8gPgoJW",
  "facet": 6,
  "beacon": "0dc5485c857c0235b968e0ad46c334a040d078e05215f9d5ffd2d999c8920128",
  "chunk": 17
}
```

Response: the chunk (hex) and its merkle path.

```json
{
  "chunk": 17,
  "data": "8f1e0c…  (2048 hex characters)",
  "siblings": ["9a41…", "c07d…", "…"]
}
```

404 if the node does not hold the facet. The auditor checks the answer against the registered root
([protocol §7](protocol.md#7-proof-of-storage)).

### `DELETE /v1/facets`

Devnet only, and only with `--allow-wipe`: deletes every facet. Returns `{"wiped": 1}`; 403 otherwise.

### `GET /metrics`

Prometheus text format. See [running-a-node.md](running-a-node.md#7-monitoring).

## Registry

### `GET /v1/info`

```json
{
  "cluster": "devnet-local",
  "version": "0.1.0",
  "slot": 261,
  "epoch": 0,
  "beacon": "0dc5485c857c0235b968e0ad46c334a040d078e05215f9d5ffd2d999c8920128",
  "nodes": 16,
  "objects": 1,
  "certified": 1,
  "committee": ["DABraZU11NsaFwViArQnFdvXfABto6DFtNLy8woRPkZj", "…16 seats"]
}
```

### `GET /v1/nodes` · `POST /v1/nodes`

```json
{
  "pubkey": "Evctq1qjDSYMNRnzVbuKsDdknFPdG2nHsuqa6H97pzRf",
  "url": "http://127.0.0.1:7707",
  "stake": 2000,
  "slashed": 500,
  "joined_slot": 1
}
```

`POST` body: `{pubkey, url, stake, signature}`, signature over
`"refrax/register/v1" ‖ pubkey ‖ stake ‖ url`. Stake must be at least 1,000.

### `GET /v1/objects` · `POST /v1/objects` · `GET /v1/objects/{id}`

`POST` body: `{"manifest": {…}}`. 409 while the committee has fewer than 16 seats.

```json
{
  "id": "BbBw21CYd9ukRes16jP3sv98PCdJmv4kPeBKe8gPgoJW",
  "manifest": {
    "version": 1,
    "size": 145995,
    "shard_len": 29696,
    "facet_roots": ["d6d2bd610300c72c808e2f0cb9bddfd4c0fc352ee8302492f7af06d69a5a1e36", "…16 roots"],
    "root": "54b69fe8292009d7f05e362f18b02f4029718836cc95c7494d7cbe01f1836dc5"
  },
  "committee": ["DABraZU11NsaFwViArQnFdvXfABto6DFtNLy8woRPkZj", "…16 seats"],
  "status": "certified",
  "acks": [{ "facet": 0, "node": "DABr…PkZj", "signature": "5xbz…x4Dk" }, "…"],
  "registered_slot": 6,
  "certified_slot": 6
}
```

### `POST /v1/objects/{id}/certify`

Body: `{"acks": [...]}`. Receipts accumulate; at 11 valid ones the object is certified. 400 with
`"N valid signatures, 11 needed"` below quorum.

### `POST /v1/slash`

Body: `{accused, object, facet, epoch, voter, signature}`, signature over
`"refrax/slash/v1" ‖ accused ‖ object ‖ facet ‖ epoch`. Response:

```json
{ "votes": 11, "needed": 11, "slashed": true }
```

### `GET /v1/events?since=<seq>`

Everything that happened after `seq`:

```json
{
  "seq": 20,
  "slot": 24,
  "event": {
    "kind": "slashed",
    "node": "Evctq1qjDSYMNRnzVbuKsDdknFPdG2nHsuqa6H97pzRf",
    "object": "BbBw21CYd9ukRes16jP3sv98PCdJmv4kPeBKe8gPgoJW",
    "amount": 500,
    "stake": 2000,
    "votes": 11
  }
}
```

Kinds: `node_joined`, `object_registered`, `object_certified`, `vote_opened`, `slashed`.
