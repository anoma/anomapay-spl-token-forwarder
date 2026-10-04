#!/usr/bin/env bash
# The forwarder's commands, run in the current environment: inside the Nix dev
# shell (./scripts/dev.sh <command> enters it), or in CI, which installs the
# same pinned tools.
set -euo pipefail

# shellcheck source=lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<USAGE
Usage: ./scripts/ops.sh <command>

  fmt            Check the formatting of every Rust workspace
  clippy         Lint the program (production, with its dev features, and as a
                 CPI dependency) and the client crate, with no lint allowances
  unit-test      The Rust unit tests at the local addresses
  build-dev      The development build (dev features on), with IDL and types
  build-release  The production build; checks its IDL is the development IDL
                 minus the dev-only instructions
  verify-build   The deterministic solana-verify build at the local addresses
USAGE
  exit 1
}

cd "$PROJECT_DIR"
load_program_ids localnet

case "${1:-}" in
  fmt)
    require_cmd cargo
    cargo fmt --all -- --check
    ;;
  clippy)
    require_cmd cargo
    check_program_id_not_in_tree
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy -p spl-token-forwarder --features "$DEV_FEATURES" --all-targets -- -D warnings
    # `cpi` is how another program depends on this one to call it.
    cargo clippy -p spl-token-forwarder --features cpi --all-targets -- -D warnings
    # The client's wire formats and event decoders without the Solana SDK.
    cargo clippy -p anomapay-spl-token-forwarder-client --no-default-features --all-targets -- -D warnings
    ;;
  unit-test)
    require_cmd cargo
    cargo test --workspace
    ;;
  build-dev)
    require_cmd anchor
    build_dev
    ;;
  build-release)
    require_cmd anchor
    require_cmd node
    build_release
    ;;
  verify-build)
    deterministic_build
    echo "Built executable hash: $(solana-verify get-executable-hash "$PROGRAM_SO")"
    ;;
  *)
    usage
    ;;
esac
