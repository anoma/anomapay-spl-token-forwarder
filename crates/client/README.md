[![CI](https://github.com/anoma/anomapay-spl-token-forwarder/actions/workflows/ci.yml/badge.svg)](https://github.com/anoma/anomapay-spl-token-forwarder/actions/workflows/ci.yml) [![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

# AnomaPay SPL Token Forwarder Client

This crate provides [Rust](https://www.rust-lang.org/) bindings to the AnomaPay SPL token forwarder, which is the Solana program in this repository. The crate contains:

- the bindings that Anchor's `declare_program!` generates from the program's IDLs in `idls/` (module `generated`, one module per build): the program's id, its accounts, events, constants and errors, and the accounts and arguments of each instruction;
- builders for the instructions that the program accepts, over the generated accounts and arguments, which derive the program's addresses from the seeds the IDLs declare;
- the accounts that the submitter of a settlement supplies for each call to the forwarder;
- the instructions that a settlement transaction must carry before the settlement itself;
- encoders and decoders for the data that the program receives: the message that authorizes a wrap, and the inputs of a wrap, an unwrap and a migration;
- the decoder of the emergency-migrating build's `VulnerableLogicRefs` account, which the IDL does not declare;
- the deployment record, `deployments.json` (module `deployments`): per environment (staging on devnet, production on mainnet-beta), the program on each cluster under the cluster's CAIP-2 chain id, with the slot of its first deployment;
- the program at an address (module `instance`), as the EVM bindings return the ERC20 forwarder: `spl_token_forwarder(rpc, environment)` and `emergency_migrating_spl_token_forwarder(rpc, environment)` find the recorded program of the cluster behind `rpc`, and `at(rpc, program_id)` takes any address. Each answers the EVM forwarder's reads in snake case (`get_logic_ref`, `get_protocol_adapter`, `owner`, and `version` by simulating the `version` instruction; the emergency-migrating build adds `is_logic_ref_vulnerable`, `is_nullifier_migrated` and `legacy_forwarder`) and builds its build's instructions at its address.

The program's events decode with the adapter client's `decode_cpi_event` as the generated event types.

The bindings for the adapter itself are in [anoma-pa-solana-client](https://github.com/anoma/anoma-pa-solana-client).

## Prerequisites

The repository's Nix shell provides the Rust version that CI uses, so [Nix](https://nixos.org/) is the only thing to install:

```sh
curl --proto '=https' --tlsv1.2 -L https://nixos.org/nix/install | sh -s -- --daemon
```

On macOS, leave out `-s -- --daemon`. From a checkout of this repository, the commands below run in the Nix shell through `./scripts/dev.sh`, which turns on the flakes support that the shell needs.

## Usage

### Build

Run

```sh
./scripts/dev.sh run cargo build -p anomapay-spl-token-forwarder-client --all-features
```

### Features

The crate has two features:

- `solana` (on by default) adds everything that needs the Solana SDK: the instruction builders (including those of the emergency-migrating build), the accounts and the preceding instructions of a settlement, the addresses that are derived from the program id, and the deployment record.
- `rpc` adds `submitter::SplTokenForwarder`, which implements the adapter client's `Forwarder` trait (the adapter client calls it to get the accounts for each call to this forwarder during a settlement), and the instances in `instance`. `rpc` also turns on `solana`.

With `--no-default-features`, the crate keeps the encoders and decoders for the data that the program receives, the `VulnerableLogicRefs` decoder, and the generated bindings, which need only `anchor-lang`.

### Test

To test the crate, run

```sh
./scripts/dev.sh run cargo test -p anomapay-spl-token-forwarder-client --all-features
```

The unit tests check that each instruction builder writes the expected bytes and accounts and matches the program's IDLs in `idls/`, and that the derived addresses are the ones the IDLs declare. They also check the byte layout of each input, of the `VulnerableLogicRefs` account and of the accounts that each call receives. The test in `tests/cross_package_fixture.rs` checks that the crate encodes the message that authorizes a wrap to the bytes recorded in the repository's `fixtures/wrap_message_fixture.json`. The TypeScript package checks the same file, so the two packages encode the message alike. The tests in `tests/deployments.rs` check that every entry of `deployments.json` names a known cluster once with a program id that parses, and that staging's devnet program is the IDLs' address and `env/devnet.env`'s `FORWARDER_PROGRAM_ID`.

The instances' reads are tested against a deployed forwarder by the repository's integration tests (`crates/integration-test/tests/instance_reads.rs`, and `emergency_migrating.rs` for the emergency-migrating build). At each deployment and upgrade, `tests/deployed.rs` checks that every recorded program exists and is executable on its cluster:

```sh
HELIUS_DEVNET_URL=<the Helius devnet endpoint> ./scripts/dev.sh run cargo test -p anomapay-spl-token-forwarder-client --all-features --test deployed -- --ignored
```
