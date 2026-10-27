# Running a node

> Devnet preview. There is no public registry yet; these steps work against any registry you can reach,
> including one started with `refrax devnet up --host 0.0.0.0` on another machine.

## Requirements

| | Minimum | Comfortable |
|---|---|---|
| CPU | 2 cores | 4 cores |
| Memory | 1 GB | 4 GB |
| Disk | 50 GB SSD | 1 TB+ NVMe |
| Network | 50 Mbit/s, public IPv4 or a reverse proxy | 1 Gbit/s |
| OS | Linux or macOS, x86_64 or arm64 | Ubuntu 22.04+ / Debian 12 |

A facet is at most 256 MiB, so one object is at most 1.25 GiB.

## 1. Install

From the latest release (Linux x86_64 / arm64, macOS Apple silicon / Intel):

```sh
curl -fsSL https://raw.githubusercontent.com/refrax-xyz/refrax-node/main/install.sh | sh
refrax --version
```

The script checks the archive against its published SHA-256. `REFRAX_VERSION=v0.1.0` pins a version,
`REFRAX_INSTALL_DIR=/usr/local/bin` changes the target directory. Windows builds are `.zip` files on the
[releases page](https://github.com/refrax-xyz/refrax-node/releases).

From source:

```sh
git clone https://github.com/refrax-xyz/refrax-node && cd refrax-node
cargo install --path crates/refrax-cli
```

## 2. Create the node identity

```sh
refrax keys new                          # writes ~/.config/refrax/node.json (mode 0600)
refrax keys show                         # prints the public key
```

The file uses the `solana-keygen` format, so an existing Solana keypair works too. This key is the
node's identity and the owner of its stake. **Back it up.** Losing it means losing the node; leaking it
means someone else can sign receipts and votes as you.

## 3. Start

```sh
refrax node start \
  --registry https://registry.example \
  --listen 127.0.0.1:7701 \
  --public-url https://node.example.com \
  --data-dir /var/lib/refrax \
  --stake 2500
```

| Flag | Environment | Default | |
|---|---|---|---|
| `--registry` | `REFRAX_REGISTRY` | `http://127.0.0.1:7700` | Registry to join |
| `--key` | `REFRAX_KEY` | `~/.config/refrax/node.json` | Identity |
| `--data-dir` | `REFRAX_DATA_DIR` | `./refrax-data` | Facets live in `<dir>/facets` |
| `--listen` | `REFRAX_LISTEN` | `0.0.0.0:7701` | Bind address |
| `--public-url` | `REFRAX_PUBLIC_URL` | derived from `--listen` | URL clients and peers use |
| `--stake` | `REFRAX_STAKE` | `2500` | Stake to bond (≥ 1,000) |
| `--audit-every` | | `2` | Seconds between audit rounds |
| `--allow-wipe` | | off | Enables `DELETE /v1/facets`. **Devnet demos only** |
| | `REFRAX_LOG` | `refrax_node=info` | Log filter (`refrax_node=debug` for every challenge) |

On start the node signs a registration message with its key, joins the registry, serves facets, and
starts auditing its committee peers.

## 4. TLS

The node speaks plain HTTP. Put a TLS terminator in front and pass its URL as `--public-url`.
With Caddy:

```
node.example.com {
    reverse_proxy 127.0.0.1:7701
    request_body {
        max_size 256MB
    }
}
```

## 5. systemd

[`deploy/refrax-node.service`](../deploy/refrax-node.service) runs the node as an unprivileged user with
a read-only system and `/var/lib/refrax` as the only writable path. Settings come from
`/etc/refrax/node.env` ([example](../deploy/node.env.example)).

```sh
sudo useradd --system --home /var/lib/refrax refrax
sudo install -d -o refrax -g refrax /var/lib/refrax /etc/refrax
sudo install -m 0755 "$(which refrax)" /usr/local/bin/refrax
sudo -u refrax refrax keys new --out /var/lib/refrax/keypair.json
sudo cp deploy/node.env.example /etc/refrax/node.env      # then edit it
sudo cp deploy/refrax-node.service /etc/systemd/system/
sudo systemctl enable --now refrax-node
journalctl -u refrax-node -f
```

## 6. Docker

```sh
docker run -d --name refrax --restart unless-stopped \
  -p 7701:7701 -v refrax-data:/data \
  -e REFRAX_REGISTRY=https://registry.example \
  -e REFRAX_PUBLIC_URL=https://node.example.com \
  ghcr.io/refrax-xyz/refrax-node:0.1
```

Images are published to `ghcr.io/refrax-xyz/refrax-node` on every release (`latest`, `0.1`, `0.1.0`).
To build your own: `docker build -t refrax-node .`

The image runs as uid 10001 and creates `/data/keypair.json` on first start. Keep the volume: it holds
the identity and the facets.

A whole devnet in a container:

```sh
docker compose -f deploy/docker-compose.devnet.yml up --build
```

## 7. Monitoring

`GET /v1/health` returns the identity, version, facets and bytes stored. `GET /metrics` is in Prometheus
text format:

| Metric | Type | |
|---|---|---|
| `refrax_facets_stored` | gauge | Facets on disk |
| `refrax_bytes_stored` | gauge | Bytes on disk |
| `refrax_facets_accepted_total` | counter | Facets accepted and signed for |
| `refrax_facets_rejected_total` | counter | Wrong seat or wrong root |
| `refrax_challenges_answered_total` | counter | Proofs this node produced |
| `refrax_challenges_missed_total` | counter | Challenges for facets it does not have. **Alert on this.** |
| `refrax_audits_passed_total` | counter | Peer proofs that verified |
| `refrax_audits_failed_total` | counter | Peer proofs that failed |
| `refrax_votes_cast_total` | counter | Slash votes sent |

```yaml
# prometheus.yml
scrape_configs:
  - job_name: refrax
    static_configs:
      - targets: ["127.0.0.1:7701"]
```

## 8. How a node loses stake

A node is slashed when 11 committee peers each see it miss two challenges in a row for the same facet.
In practice that means the facet is gone, the disk is corrupt, or the node has been unreachable for
several audit rounds. Keep the data directory on reliable storage, keep the node reachable, and watch
`refrax_challenges_missed_total`.

You can watch the whole thing happen safely on a devnet:

```sh
refrax devnet up                 # terminal 1
refrax put ./some-file           # terminal 2
refrax devnet wipe 7
refrax status                    # node-07: stake 2,000 REFRAX, slashed 500
```
