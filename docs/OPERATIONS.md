# Operating the SPL token forwarder

The SPL token forwarder holds AnomaPay's wrapped SPL tokens in escrow and executes the wrap and unwrap calls the adapter forwards to it. It has two authorities, both recorded in its config PDA at initialization: the **owner**, which upgrades the program and rotates the logic ref, and the **emergency committee**, which names the emergency caller and closes accounts. The adapter's own procedures (its deployment, owner, pause, kind table and the settlement lookup table's creation) are in the adapter repository's [docs/OPERATIONS.md](https://github.com/anoma/solana-protocol-adapter/blob/anthony/arm-v2-port/solana-pa-prototype/docs/OPERATIONS.md).

Commands run through `./scripts/dev.sh`, which enters the Nix shell; cluster operations take `--cluster <localnet|devnet|mainnet>` (`scripts/ops.sh` lists every flag). devnet and mainnet operations go through the operator's RPC provider: pass `--url <rpc>` or set `DEVNET_RPC_URL` / `MAINNET_RPC_URL`; there is no public-endpoint default. The forwarder's commands take their parameters as `STF_*` environment variables (`scripts/forwarder.ts` lists them).

## The owner

The forwarder's owner is set by its `initialize` (`STF_OWNER`) and stored in its config, as the adapter's is in its state: its upgrade authority is its own PDA, its `upgrade`, `reinitialize`, `transfer_ownership` and `renounce_ownership` are owner-only, and it emits `OwnershipTransferred` and `Upgraded`, as the EVM V2 forwarder's OwnableUpgradeable and UUPS do. Ownership transfers and renouncement are made by hand: no repository command changes an authority on a live cluster.

## Deploy and initialize

```sh
export STF_LOGIC_REF=<32-byte hex verifying key of the AnomaPay resource logic>
export STF_EMERGENCY_COMMITTEE=<base58 pubkey>
export STF_OWNER=<the owner's pubkey>
export STF_TOKEN_MINT=<base58 mint>          # optional: also creates the mint's escrow ATA
./scripts/dev.sh deploy --cluster devnet     # publishes the IDL, stops before init
# by hand: give the forwarder's canonical IDL account to the owner
./scripts/dev.sh forwarder init --cluster devnet
```

A first deploy reads the forwarder's keypair path from the uncommitted `env/<cluster>.keys.env` ([env/README.md](../env/README.md)). On devnet and mainnet, `deploy` publishes the forwarder's IDL and stops before `initialize`, for the reason the adapter's deploy does: `initialize` gives the upgrade authority, which alone creates the program's canonical metadata accounts, to the program, so the deployer gives the IDL account to the owner (`program-metadata set-authority`) before initializing. On localnet it initializes at once.

The program's upgrade authority, the deployer, initializes the config, as the EVM proxy runs its initializer at deployment; no other signer can. It hands the upgrade authority to the program's PDA. The config pins the adapter program id (`env/<cluster>.env`), the logic ref, the committee and the owner. A wrap is only executed when the adapter forwards it for a resource carrying that logic ref. One escrow authority, a PDA of the forwarder, owns every mint's escrow: the associated token account of the authority and the mint, as the EVM forwarder holds every token at its own address. `forwarder init` with `STF_TOKEN_MINT` creates a mint's escrow account, and the same command adds further mints later. Add each new mint's escrow account to the settlement lookup table as well.

## The settlement lookup table

The adapter's operator creates the deployment's settlement lookup table and adds the adapter's fixed accounts. The forwarder's fixed accounts (the forwarder, its config, the instructions sysvar, its event authority and escrow authority, and the SPL token program) and each supported mint's escrow account join it from here, signed by the table's authority:

```sh
PA_LOOKUP_TABLE=<address> STF_TOKEN_MINTS=<mint>,<mint> \
  ./scripts/dev.sh lookup-table --cluster devnet
```

A table entry need not exist on chain: the forwarder's keys go in before the forwarder is deployed, and a mint's escrow account before `forwarder init` creates it. Extending is idempotent; a rerun adds only what is missing.

## Nonce bitmaps

A wrap's replay protection is a per-user, per-256-nonce-word bitmap account. The adapter forwards no signer to the forwarder, so the forwarder cannot create that account during a wrap; the submitter creates it with the permissionless `init_nonce_bitmap` instruction (any payer) when the word's bitmap does not exist, and the wrap fails with `NonceBitmapMissing` when it is absent. The init fits in the settlement transaction itself, after the ed25519 instruction the wrap input points at; the integration tests settle the first wrap that way.

## Rotating the logic ref

The logic ref changes whenever the resource circuit is rebuilt. It is rotated as the EVM forwarder's is: the owner upgrades the proxy to an implementation whose `reinitializer(n)` writes the new ref. The forwarder's config records the version it was last initialized at; `reinitialize` writes the new ref only while that version is below the build's `CONFIG_VERSION`, and then records it, so each build rotates once. To rotate, raise `CONFIG_VERSION` by one in a new build, upgrade the program in place, and reinitialize, both with the owner's wallet:

```sh
./scripts/dev.sh upgrade --cluster <c>                                                                # owner wallet
STF_LOGIC_REF=<new 32-byte hex verifying key> ./scripts/dev.sh forwarder reinitialize --cluster <c>   # owner wallet
```

Escrow, nonce bitmaps and the committee are untouched. The instruction emits `Initialized` with the new version, as OpenZeppelin's reinitializer does; read the new ref from the config account. Resources wrapped under the previous ref leave through the new one once the adapter's kind table lists the previous version as an alias of the new one (anoma/risc0-kind-tables ADR-0008, rule R2): a transaction converts each into a resource under the new ref, which then unwraps. Until that table's commitment is installed (the adapter's `set-kind-table`), they stay in escrow and can neither unwrap nor convert. The emergency path below is for a stopped adapter only.

## Checking a devnet deployment with a wrap and an unwrap

`./scripts/dev.sh integration-test --e2e` settles a wrap and an unwrap with real proofs on a fork of devnet, against the forwarder and the adapter devnet runs and the state they hold (`crates/integration-test/tests/wrap_unwrap.rs`). It needs `DEVNET_RPC_URL` and the proving queue (`QUEUE_BASE_URL`, `QUEUE_AUTH_TOKEN`); it checks first that the devnet forwarder serves the devnet adapter and the logic ref the tests build.

## Upgrading the forwarder

The forwarder is upgraded in place, as the EVM forwarder's proxy is upgraded through `upgradeToAndCall`: the program id, the config, the escrow and the nonce bitmaps stay. The owner upgrades it with `./scripts/dev.sh upgrade --cluster <c>`, which writes the new build to a buffer and calls the forwarder's `upgrade` (before `initialize`, while the deployer's wallet is still the upgrade authority, it upgrades through the loader); `--prebuilt` ships `verify-build`'s deterministic build. A release that changes an account layout ships owner-only migration instructions, the counterpart of the call the EVM owner passes to `upgradeToAndCall`, which run once, right after the upgrade, and a test that runs the upgrade path from the previous build.

## Emergency committee

The committee and its emergency caller carry over the EVM V1 forwarder's emergency mechanism; the EVM V2 forwarder has none (it relies on its owner's upgrades), and anoma/dos-pm#86 tracks whether this forwarder keeps, replaces or drops it. As built:

Once the adapter is paused (the adapter's `pause`), the committee names an emergency caller, once, and that caller withdraws from escrow directly without going through the adapter:

```sh
STF_TOKEN_MINT=<mint> STF_RECIPIENT=<owner> STF_AMOUNT=<raw units> \
  ./scripts/dev.sh forwarder emergency-withdraw --cluster <c>                                # caller wallet
```

Naming the emergency caller grants a key the right to withdraw escrowed funds, so, like every authority change on a live cluster, it is done by hand: no repository command or script builds it. The committee constructs and signs the forwarder's `set_emergency_caller(caller)` itself from the IDL (accounts: the committee as signer, the config, the paused adapter's PAState). The program refuses it while the adapter is not paused and refuses a second one. The forwarder's other committee instructions, `close_escrow` (drain an escrow to a recipient and close it), `close_nonce_bitmaps_batch` and `close_config`, refuse while the adapter is not paused, and are likewise made by hand.

## Retiring the forwarder

Retirement closes accounts for good, so on a live cluster it is done by hand. Once the adapter is paused, the committee signs, in order: `close_nonce_bitmaps_batch` over every nonce bitmap the forwarder owns (closing them ends wrap replay protection; the forwarder cannot be initialized again, its upgrade authority being its own PDA), `close_escrow` for each mint's escrow (drains it to the committee's token account and closes it), then `close_config`, which also ends the forwarder's ownership: with no config, neither `upgrade` nor `reinitialize` can run. The program stays on chain, its upgrade authority its own PDA.
