# Operating the SPL token forwarder

The SPL token forwarder holds AnomaPay's wrapped SPL tokens in escrow and executes the wrap and unwrap calls the adapter forwards to it. Its one authority, recorded in its config PDA at initialization, is the **owner**, which upgrades the program, rotates the logic ref and, while the adapter is paused, closes the forwarder's accounts. The adapter's own procedures (its deployment, owner, pause and kind table) are in the adapter repository's [docs/OPERATIONS.md](https://github.com/anoma/solana-protocol-adapter/blob/anthony/arm-v2-port/solana-pa-prototype/docs/OPERATIONS.md).

Commands run through `./scripts/dev.sh`, which enters the Nix shell; cluster operations take `--cluster <localnet|devnet|mainnet>` (`scripts/ops.sh` lists every flag). devnet and mainnet operations go through the operator's RPC provider: pass `--url <rpc>` or set `DEVNET_RPC_URL` / `MAINNET_RPC_URL`; there is no public-endpoint default. The forwarder's commands take their parameters as `STF_*` environment variables (`scripts/forwarder.ts` lists them).

## The owner

The forwarder's owner is set by its `initialize` (`STF_OWNER`) and stored in its config, as the adapter's is in its state: its upgrade authority is its own PDA, its `upgrade`, `reinitialize`, `transfer_ownership` and `renounce_ownership` are owner-only, and it emits `OwnershipTransferred` and `Upgraded`, as the EVM V2 forwarder's OwnableUpgradeable and UUPS do. Ownership transfers and renouncement are made by hand: no repository command changes an authority on a live cluster.

## Deploy and initialize

```sh
export STF_LOGIC_REF=<32-byte hex verifying key of the AnomaPay resource logic>
export STF_OWNER=<the owner's pubkey>
export STF_TOKEN_MINT=<base58 mint>          # optional: also creates the mint's escrow ATA
./scripts/dev.sh deploy --cluster devnet     # publishes the IDL, stops before init
# by hand: give the forwarder's canonical IDL account to the owner
./scripts/dev.sh forwarder init --cluster devnet
```

A first deploy reads the forwarder's keypair path from the uncommitted `env/<cluster>.keys.env` ([env/README.md](../env/README.md)). On devnet and mainnet, `deploy` publishes the forwarder's IDL and stops before `initialize`, for the reason the adapter's deploy does: `initialize` gives the upgrade authority, which alone creates the program's canonical metadata accounts, to the program, so the deployer gives the IDL account to the owner (`program-metadata set-authority`) before initializing. On localnet it initializes at once.

The program's upgrade authority, the deployer, initializes the config, as the EVM proxy runs its initializer at deployment; no other signer can. It hands the upgrade authority to the program's PDA. The config pins the adapter program id (`env/<cluster>.env`), the logic ref and the owner. A wrap is only executed when the adapter forwards it for a resource carrying that logic ref. One escrow authority, a PDA of the forwarder, owns every mint's escrow: the associated token account of the authority and the mint, as the EVM forwarder holds every token at its own address. `forwarder init` with `STF_TOKEN_MINT` creates a mint's escrow account, and the same command adds further mints later.

## Nonce bitmaps

A wrap's replay protection is a per-user, per-256-nonce-word bitmap account. The adapter forwards no signer to the forwarder, so the forwarder cannot create that account during a wrap; the submitter creates it with the permissionless `init_nonce_bitmap` instruction (any payer), which does nothing when the word's bitmap exists, so a submitter sends it with every wrap without reading the chain; the wrap fails with `NonceBitmapMissing` when the bitmap is absent. The init fits in the settlement transaction itself, after the ed25519 instructions the wraps' inputs point at; the client's `settlement_calls` builds it so.

## Rotating the logic ref

The logic ref changes whenever the resource circuit is rebuilt. It is rotated as the EVM forwarder's is: the owner upgrades the proxy to an implementation whose `reinitializer(n)` writes the new ref. The forwarder's config records the version it was last initialized at; `reinitialize` writes the new ref only while that version is below the build's `CONFIG_VERSION`, and then records it, so each build rotates once. To rotate, raise `CONFIG_VERSION` by one in a new build, upgrade the program in place, and reinitialize, both with the owner's wallet:

```sh
./scripts/dev.sh upgrade --cluster <c>                                                                # owner wallet
STF_LOGIC_REF=<new 32-byte hex verifying key> ./scripts/dev.sh forwarder reinitialize --cluster <c>   # owner wallet
```

Escrow and nonce bitmaps are untouched. The instruction emits `Initialized` with the new version, as OpenZeppelin's reinitializer does; read the new ref from the config account. Resources wrapped under the previous ref leave through the new one once the adapter's kind table lists the previous version as an alias of the new one (anoma/risc0-kind-tables ADR-0008, rule R2): a transaction converts each into a resource under the new ref, which then unwraps. Until that table's commitment is installed (the adapter's `set-kind-table`), they stay in escrow and can neither unwrap nor convert.

## Checking a devnet deployment with a wrap and an unwrap

`./scripts/dev.sh integration-test --e2e` settles a wrap and an unwrap with real proofs (`crates/integration-test/tests/wrap_unwrap.rs`), proven by the queue (`QUEUE_BASE_URL`, `QUEUE_AUTH_TOKEN`) or on this machine (`E2E_PROVER=local`). By default it runs on a local runtime, where devnet's Groth16 verifier checks the proofs and the forwarder is deployed and initialized for the logic ref the tests build. With `E2E_RUNTIME=devnet` and `DEVNET_RPC_URL` it runs on a fork of devnet, against the forwarder and the adapter devnet runs and the state they hold; it checks first that the devnet forwarder serves the devnet adapter and that logic ref.

## Upgrading the forwarder

The forwarder is upgraded in place, as the EVM forwarder's proxy is upgraded through `upgradeToAndCall`: the program id, the config, the escrow and the nonce bitmaps stay. The owner upgrades it with `./scripts/dev.sh upgrade --cluster <c>`, which writes the new build to a buffer and calls the forwarder's `upgrade` (before `initialize`, while the deployer's wallet is still the upgrade authority, it upgrades through the loader); `--prebuilt` ships `verify-build`'s deterministic build. A release that changes an account layout ships owner-only migration instructions, the counterpart of the call the EVM owner passes to `upgradeToAndCall`, which run once, right after the upgrade, and a test that runs the upgrade path from the previous build.

### From the release with an emergency committee

The release before this one stored an emergency committee and an emergency caller in the config, the EVM V1 forwarder's emergency mechanism, which the EVM V2 forwarder does not have. This release drops both, so its config is 64 bytes shorter. After the upgrade, the owner rewrites the config once:

```sh
./scripts/dev.sh upgrade --cluster <c>                                 # owner wallet
./scripts/dev.sh forwarder migrate-config --cluster <c>                # owner wallet
```

`migrate_config` keeps the adapter, the logic ref, the version and the owner, and returns the rent the smaller account no longer needs to the owner. Until it runs, wraps and unwraps settle, since both layouts start with the adapter and the logic ref, and every owner-only instruction refuses the config with `ConfigNotMigrated`; a second run fails with `NotPreviousLayout`.

## After a flaw in a circuit version

When a circuit version of the transfer resource lets false proofs pass, the adapter's owner denies its logic ref and the forwarder's owner upgrades the forwarder to the emergency-migrating build, whose resources with that logic ref then migrate to a fixed version without moving tokens. [EMERGENCY_MIGRATION.md](EMERGENCY_MIGRATION.md) is the procedure and its limits.

## Retiring the forwarder

Retirement closes accounts for good, so on a live cluster it is done by hand. The closing instructions recover the rent of accounts EVM storage does not have, so they have no EVM counterpart; they are the owner's, who can already replace the program's code, and each refuses while the adapter is not paused. Once the adapter is paused, the owner signs, in order: `close_nonce_bitmaps_batch` over every nonce bitmap the forwarder owns (closing them ends wrap replay protection; the forwarder cannot be initialized again, its upgrade authority being its own PDA), `close_escrow` for each mint's escrow (drains it to a token account the owner names and closes it), then `close_config`, which also ends the forwarder's ownership: with no config, neither `upgrade` nor `reinitialize` can run. The program stays on chain, its upgrade authority its own PDA.
