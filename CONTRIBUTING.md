# Contributing to refrax-node

Thanks for helping. This project stores other people's data, so changes are judged first on whether they
keep data safe and the protocol unambiguous, then on everything else.

## Setup

```sh
git clone https://github.com/refrax-xyz/refrax-node && cd refrax-node
cargo test --workspace        # includes a 16-node devnet over real HTTP
```

Rust stable is pinned in `rust-toolchain.toml`.

## Before you open a pull request

- `cargo test --workspace` passes.
- `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- `cargo fmt --all` has been run.
- New behaviour has a test. Bugs get a test that fails without the fix.
- If you touched the node, the client or the CLI, try it on `refrax devnet up`.

## Protocol changes

Anything that changes a hash input, a signed message, a wire type or a parameter in
[docs/protocol.md](docs/protocol.md) is a protocol change. For those:

1. Open an issue first and describe the change in terms of the spec.
2. Update `docs/protocol.md` in the same pull request as the code.
3. Use a new domain tag (`refrax/…/v2`) rather than changing the meaning of an existing one.

## Code layout and style

- `refrax-core` stays pure: no I/O, no async, no global state. If it needs the network or the disk, it
  belongs in `refrax-node`.
- No `unsafe` (`#![forbid(unsafe_code)]` is set in the libraries).
- Errors say what went wrong and what to do about it; `refrax` prints them to people.
- Keep dependencies few. Say why a new one is worth it in the pull request.

## Commits

Small, focused commits with a message that says what changed and why, e.g.
`node: refuse facets whose length does not match shard_len`.

## License

By contributing you agree that your contributions are licensed under the [Apache-2.0](LICENSE) license.
