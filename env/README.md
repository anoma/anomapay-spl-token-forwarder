# Program addresses and keypairs

The forwarder reads its address at compile time, and so does the adapter
crate it builds against: their `declare_id!`s take
`FORWARDER_PROGRAM_ID` and `PROTOCOL_ADAPTER_PROGRAM_ID`. The scripts
export both from the files here:

- `localnet.env`: the addresses on a local runtime, where the tests load the
  forwarder and the adapter.
- `<cluster>.env` (`devnet.env`, and `mainnet.env` from the first mainnet
  deploy): the forwarder deployed to that cluster, and the adapter of the same
  cluster it is initialized with.

The forwarder's keypair is never committed. It is needed only to create the
program at its address, on its first deploy to a cluster; upgrades go by
address. Each operator names the keypair path in the uncommitted
`<cluster>.keys.env`:

```
FORWARDER_PROGRAM_KEYPAIR=/path/outside/the/repository/spl_token_forwarder-keypair.json
```

A deploy refuses a keypair whose address is not the one `<cluster>.env` gives
the forwarder.
