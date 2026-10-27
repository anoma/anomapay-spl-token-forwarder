[![CI](https://github.com/anoma/anomapay-spl-token-forwarder/actions/workflows/ci.yml/badge.svg)](https://github.com/anoma/anomapay-spl-token-forwarder/actions/workflows/ci.yml) [![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

# AnomaPay SPL Token Forwarder Client

This crate provides [Rust](https://www.rust-lang.org/) bindings to the AnomaPay SPL token forwarder, which is the Solana program in this repository. The crate contains:

- builders for the instructions that the program accepts;
- the accounts that the submitter of a settlement supplies for each call to the forwarder;
- the instructions that a settlement transaction must carry before the settlement itself;
- encoders and decoders for the data that the program receives: the message that authorizes a wrap, and the inputs of a wrap, an unwrap and a migration;
- decoders for the program's accounts and events, the events being the types that Anchor's `declare_program!` generates from the program's IDLs in `idls/` (module `generated`);
- the program's id.

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

- `solana` (on by default) adds everything that needs the Solana SDK: the instruction builders (including those of the emergency-migrating build), the accounts and the preceding instructions of a settlement, the addresses that are derived from the program id, and the program id itself.
- `rpc` adds `SplTokenForwarder`, which implements the adapter client's `Forwarder` trait. The adapter client calls it to get the accounts for each call to this forwarder during a settlement. `rpc` also turns on `solana`.

With `--no-default-features`, the crate keeps the encoders and decoders for the data that the program receives, the decoders for its accounts, and the generated bindings with the event decoder, which need only `anchor-lang`.

### Test

To test the crate, run

```sh
./scripts/dev.sh run cargo test -p anomapay-spl-token-forwarder-client --all-features
```

The unit tests check the instruction builders, the account and event decoders, the constants and the program id against the program's IDLs in `idls/`. They also check the byte layout of each input and of the accounts that each call receives. The test in `tests/cross_package_fixture.rs` checks that the crate encodes the message that authorizes a wrap to the bytes recorded in the repository's `fixtures/wrap_message_fixture.json`. The TypeScript package checks the same file, so the two packages encode the message alike.
