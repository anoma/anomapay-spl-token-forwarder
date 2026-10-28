# AnomaPay SPL token forwarder

The Solana program that moves SPL tokens in and out of AnomaPay for the Anoma protocol adapter ([solana-protocol-adapter](https://github.com/anoma/solana-protocol-adapter)): a wrap escrows a user's tokens for a resource the same transaction creates, and an unwrap releases them for a resource it consumes. It is the Solana counterpart of [anomapay-erc20-forwarder](https://github.com/anoma/anomapay-erc20-forwarder/tree/next).

The adapter calls the forwarder through the external calls a settled transaction's resources commit to; the forwarder accepts a call only from the adapter it was initialized with, and only for the logic ref of the AnomaPay transfer resource it was initialized with.

## Layout

| Path | Contents |
|---|---|
| `programs/spl-token-forwarder/` | The Anchor program and its Rust unit tests |
| `crates/client/` | `anomapay-spl-token-forwarder-client`: the bindings generated from both builds' IDLs (its id, accounts, events, constants, instruction accounts and arguments), builders for the instructions it takes, the CPI segments a settlement passes it, its wire formats (the wrap authorization message, the wrap, unwrap and migrate inputs), the deployment record (`deployments.json`), the program at an address with its reads (`instance`), and the IDLs |
| `ts/` | `@anomaorg/anomapay-spl-token-forwarder`: the client crate's counterpart in TypeScript (the IDLs' coders, the deployment record, the instances with their reads and instruction builders, the CPI segments, the wrap authorization message, the event decoder), which carries copies of the IDLs and the record it builds from `crates/client/` |
| `crates/integration-test/` | The forwarder's integration tests on the adapter's harness, and the test programs they load |
| `env/` | The forwarder's address per cluster, and the adapter's it is built against ([env/README.md](env/README.md)) |
| `fixtures/` | The wrap authorization message every client must serialize alike |
| `scripts/` | `dev.sh` (enters the Nix shell), `ops.sh` (the commands) and the operator scripts (TypeScript) |
| `client/` | The operator scripts' TypeScript instruction builders, PDAs and cluster helpers |
| `docs/` | [OPERATIONS.md](docs/OPERATIONS.md) (deploying and operating the forwarder), [INTEGRATION.md](docs/INTEGRATION.md) (its events and call segments), [EMERGENCY_MIGRATION.md](docs/EMERGENCY_MIGRATION.md) (the emergency-migrating build, for a flaw in a circuit version) and [DEVNET_DEPLOYMENT.md](docs/DEVNET_DEPLOYMENT.md) (the devnet record) |

The program builds against the adapter's program crate, pinned by revision in `Cargo.toml`: it reads the adapter's state account to check that the adapter is paused before the owner closes the forwarder's accounts, and uses the upgrade helpers the adapter shares with its forwarders.

## Development

Everything runs in the repository's Nix dev shell, which pins the Rust, Solana and Anchor versions the adapter builds with; `./scripts/dev.sh` enters it.

| Command | What it does |
|---|---|
| `./scripts/dev.sh shell` / `run <cmd>` | Interactive Nix shell / one command in it, the local addresses exported |
| `./scripts/dev.sh fmt` / `clippy` | Format check / lints with CI's flags |
| `./scripts/dev.sh unit-test` | The Rust unit tests, the client's with all its features |
| `./scripts/dev.sh build-dev` | Development build (the `dev-config-version` instruction enabled) |
| `./scripts/dev.sh build-release` | Production build; checks its IDL is the development IDL minus the dev-only instruction |
| `./scripts/dev.sh verify-build` | The deterministic solana-verify build |
| `./scripts/dev.sh integration-test [--e2e]` | The integration tests on a local runtime; with `--e2e`, their cases with real proofs, on a local runtime (`E2E_RUNTIME=devnet`: on a fork of devnet) |
| `./scripts/dev.sh test-program [--check]` | Write (or check) the deterministic production and development builds the integration tests load |
| `./scripts/dev.sh typecheck` | Type-check and format-check the operator scripts against the production IDL's types |
| `./scripts/dev.sh script-test` | Run the operator scripts against a local runtime |
| `./scripts/dev.sh ts-test` | Type-check and test the TypeScript bindings (`ts/`); at each deployment and upgrade, [docs/OPERATIONS.md](docs/OPERATIONS.md) checks the recorded programs and their reads on devnet |

## Operations

Deploying, initializing, upgrading and operating the forwarder on a cluster go through `./scripts/dev.sh <command> --cluster <localnet\|devnet\|mainnet>`: `deploy`, `upgrade`, `forwarder <init\|reinitialize>`, `idl-publish`, `status`. [docs/OPERATIONS.md](docs/OPERATIONS.md) is the procedure set.
