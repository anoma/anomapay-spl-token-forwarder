# AnomaPay SPL token forwarder

The Solana program that moves SPL tokens in and out of AnomaPay for the Anoma protocol adapter ([solana-protocol-adapter](https://github.com/anoma/solana-protocol-adapter)): a wrap escrows a user's tokens for a resource the same transaction creates, and an unwrap releases them for a resource it consumes. It is the Solana counterpart of [anomapay-erc20-forwarder](https://github.com/anoma/anomapay-erc20-forwarder/tree/next).

The adapter calls the forwarder through the external calls a settled transaction's resources commit to; the forwarder accepts a call only from the adapter it was initialized with, and only for the logic ref of the AnomaPay transfer resource it was initialized with.

## Layout

| Path | Contents |
|---|---|
| `programs/spl-token-forwarder/` | The Anchor program and its Rust unit tests |
| `crates/client/` | `anomapay-spl-token-forwarder-client`: the instructions it takes, the CPI segments a settlement passes it, its wire formats (the wrap authorization message, the wrap and unwrap inputs), its events, its IDL |
| `env/` | The forwarder's address per cluster, and the adapter's it is built against ([env/README.md](env/README.md)) |
| `fixtures/` | The wrap authorization message every client must serialize alike |
| `scripts/` | `dev.sh` (enters the Nix shell) and `ops.sh` (the commands) |

The program builds against the adapter's program crate, pinned by revision in `Cargo.toml`: it reads the adapter's state account to check that the adapter is paused before an emergency call, and uses the upgrade helpers the adapter shares with its forwarders.

## Development

Everything runs in the repository's Nix dev shell, which pins the Rust, Solana and Anchor versions the adapter builds with; `./scripts/dev.sh` enters it.

| Command | What it does |
|---|---|
| `./scripts/dev.sh shell` / `run <cmd>` | Interactive Nix shell / one command in it, the local addresses exported |
| `./scripts/dev.sh fmt` / `clippy` | Format check / lints with CI's flags |
| `./scripts/dev.sh unit-test` | The Rust unit tests |
| `./scripts/dev.sh build-dev` | Development build (the `dev-config-version` instruction enabled) |
| `./scripts/dev.sh build-release` | Production build; checks its IDL is the development IDL minus the dev-only instruction |
| `./scripts/dev.sh verify-build` | The deterministic solana-verify build |
