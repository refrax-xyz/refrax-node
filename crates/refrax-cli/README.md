# refrax

```
refrax devnet up                 registry + 16 nodes on localhost
refrax devnet wipe <n>           delete a devnet node's facets, watch it get slashed
refrax node start                run a storage node
refrax put <file>                encode, register, scatter, certify
refrax get <object> [-o path]    rebuild from any 5 facets
refrax audit <object>            challenge every holder once
refrax ls | status               objects, committee, stake
refrax inspect <file>            facets and roots, no network
refrax keys new | show           node identity (solana-keygen format)
```

Install: `cargo install --path crates/refrax-cli`. Colours follow `NO_COLOR` and turn off when piped.
