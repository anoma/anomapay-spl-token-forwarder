# Moving the SPL token forwarder into its own repository

The SPL token forwarder moves out of solana-protocol-adapter into this repository, with its client code and its tests, as the ERC20 forwarder lives apart from pa-evm (anoma/dos-pm#90). Its tests are rewritten in Rust on the adapter's integration-test harness, which first learns to pass a forwarder the accounts and instructions its calls need. The adapter repository keeps the block-time and test forwarders, and gains tests for the adapter behaviours that only the SPL forwarder's tests covered.

The reference is anomapay-erc20-forwarder's `next` branch (V2): a contracts directory, a bindings crate holding the deployment record, and an integration-test crate that uses pa-evm's environments and adds its forwarder through the setup closure.

## What this repository holds

| Path | Contents | From |
|---|---|---|
| `programs/spl-token-forwarder/` | The Anchor program, with its Rust unit tests | solana-protocol-adapter `solana-pa-prototype/programs/spl-token-forwarder/` |
| `crates/client/` | The forwarder's Rust bindings: instruction builders, PDA derivation, wire formats (`WrapMessage`, wrap and unwrap inputs), events, constants, program address, IDL | anoma-pa-solana-client (`forwarder.rs`, `wrap_message.rs`, the forwarder parts of `external_call.rs`, `events.rs`, `constants.rs`, `program_ids.rs`, `idl/spl_token_forwarder.json`) |
| `ts/` | The same surface as an npm package | anoma-pa-solana-client's `ts/` forwarder modules |
| `crates/integration-test/` | The tests, in Rust, on the adapter's harness | the adapter's TypeScript forwarder specs (listed below) |
| `scripts/` | `dev.sh` and `ops.sh` for this program: build, lint, unit tests, deterministic build, deploy, upgrade, `forwarder init`, `reinitialize`, `emergency-withdraw`, IDL publishing, and the forwarder's keys for the settlement lookup table | the adapter's `scripts/` (`forwarder.ts`, the forwarder rows of `ops.sh`, `validator-deploy.sh`, `lookup-table.ts`) |
| `env/` | The forwarder's address per cluster, and the address of the adapter it is built against | the adapter's `env/` |
| `docs/` | `OPERATIONS.md` (the adapter runbook's forwarder section) and the devnet deployment record of the forwarder | the adapter's `docs/OPERATIONS.md` and `docs/DEVNET_DEPLOYMENT.md` |
| `flake.nix` | The adapter flake's toolchain pins (Rust, Solana, Anchor) | the adapter's `flake.nix` |

The first commit moves the code as it is on `anthony/arm-v2-port`, naming the source commit, as the resource circuits moved out of anomapay-backend. The devnet deployment does not change: the forwarder stays at `BsfuXpxw8oCmZXnYijyQkUYNcCnuskFZbYizmWLnpSU7`, and its record moves here.

## How the forwarder depends on the adapter

The forwarder keeps depending on the adapter's program crate (`protocol-adapter`, feature `no-entrypoint`), now by git revision of solana-protocol-adapter, as the ERC20 forwarder depends on pa-evm. It uses three things from it: the `PAStateAccount` layout, to read whether the adapter is paused before an emergency call; the upgrade-authority seed; and the upgrade helpers the adapter's `upgrade.rs` shares with it. Since #126 the adapter crate reads its own address at build time (`PROTOCOL_ADAPTER_PROGRAM_ID`), so this repository's `env/<cluster>.env` names the adapter each forwarder build pairs with, as each environment's ERC20 forwarder is initialized with the adapter proxy of the same environment.

## The forwarder's client moves here

The forwarder's bindings leave anoma-pa-solana-client, which keeps the adapter's. Their live consumer, risc0-kind-tables (the forwarder's address in its Solana deployment record, its config PDA in its integration tests), depends on this repository's crate instead. anomapay-backend and pay-interface-app are frozen prototypes and stay on the client release they pin. anoma-pa-solana-client's settlement planning stays forwarder-agnostic: it takes each external call's account segment from its caller, as it does now.

## The harness passes a forwarder its accounts

A Solana settlement carries, for each external call, the accounts of the forwarder's CPI and sometimes instructions before the settlement: a wrap needs the user's and the escrow's token accounts, the nonce bitmap, and an ed25519 instruction carrying the user's signature. The proof commits each call's program, instruction data, expected output and account count (`SolanaExternalCall`, in each resource's external payload), but not the accounts. So the harness's `ProtocolAdapter` gets a registry of forwarders: a consumer's setup registers, for its forwarder's program, a function from a call to that call's account segment and preceding instructions. `execute` decodes the transaction's external calls in the order the adapter runs them, asks each call's forwarder for its accounts, puts the preceding instructions first in the settlement transaction, and passes the segments to `plan_settlement`. A call to an unregistered program fails before anything is sent, naming it. The proof fixes where the wrap's ed25519 instruction sits (the wrap input names its index), so the preceding instructions keep the order the forwarder returns.

## The tests, in Rust

`crates/integration-test` uses the adapter harness's `local` and `e2e` environments and adds the forwarder in the setup closure, as the ERC20 forwarder's tests add theirs to pa-evm's. Setup deploys the forwarder (in `local`, the deterministic build this repository ships, checked like the harness's binaries), initializes it, creates a mint and funds the user, and registers the forwarder's account function with the harness. In `e2e`, the forwarder is devnet's. Wrap and unwrap actions are built with anomapay-solana-resource's `transfer_library::action` builders, whose transfer logic pa-testkit's `LogicWitness` wraps as the ERC20 forwarder's tests wrap theirs.

The TypeScript specs this replaces, and what each covers:

- `fresh/4-spl-token-wrap-unwrap.ts`: wrap and unwrap with their events; replay; every account, mint, owner and ed25519 rejection; unwrap to the escrow; the relay through the test forwarder; the devnet kind table; the committee's instructions refused while the adapter runs.
- `fresh/2-forwarder-initialize.ts`: initialization's authority and argument checks, and the stored config.
- `forwarder-config.ts`: `reinitialize`, a direct `forward_call`, the emergency caller's committee check.
- `forwarder-ownership.ts`: ownership transfer and self-upgrade.
- `terminal/3-forwarder-renounce.ts`, `terminal/4-forwarder-emergency.ts`, `terminal/5-forwarder-teardown.ts`: renouncing, the emergency withdrawal, closing the escrow, bitmaps and config.
- The forwarder's halves of `version.ts`, `cluster-test-guard.ts` and `local-only.ts`.

The terminal specs pause the adapter; on the harness each test has its own runtime, so they no longer need to run last.

## What the adapter repository keeps, and its new tests

solana-protocol-adapter loses the SPL forwarder's program, specs, fixtures (`spl_token_*`), fixture-gen subcommands (`spl-token-wrap`, `spl-token-unwrap`), operator script, client builders, lookup-table keys and documentation. It keeps the block-time forwarder (the example forwarder of its README tutorial) and the test forwarder, and the adapter's own test of every external-call path moves onto them. Eight adapter behaviours were exercised only by the SPL forwarder's specs; each gets an adapter test with the test forwarder:

1. A forwarder CPI receives the segment's writable accounts as writable (a new test-forwarder mode that writes an account).
2. A settlement succeeds after other instructions in its transaction.
3. The adapter's events survive a truncated log (the test forwarder's log mode).
4. The settled event carries the actions' logic refs.
5. A settlement under an installed kind-table commitment (`set_kind_table_commitment`).
6. The largest settlement the suite sends fits one v0 transaction, and every static key the lookup table can hold is in it.
7. A segment of more than one program, with the forwarder calling a second program (the relay mode, relaying to the test forwarder itself).
8. A forwarder's return data checked against the expected output, from a forwarder that changes state.

## Order of the work

1. The harness's forwarder registry (solana-protocol-adapter, on the harness branch).
2. This repository: the moved program, client and scripts, then the Rust tests on the harness.
3. solana-protocol-adapter: the eight adapter tests, then the removal.
4. anoma-pa-solana-client: the forwarder's bindings removed, with its consumers pointed here.
