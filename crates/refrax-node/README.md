# refrax-node

Everything that runs on a machine: the storage node (`node`), the registry that mirrors the Solana program
accounts (`registry`), the client operations behind `refrax put / get / audit` (`client`), and a local
cluster in one call (`devnet`).

`tests/devnet.rs` starts 16 nodes over real HTTP, takes 11 of them offline, and wipes one until the
committee slashes it.

HTTP API: [docs/api.md](../../docs/api.md). Architecture: [docs/architecture.md](../../docs/architecture.md).
