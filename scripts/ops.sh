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
  integration-test [--e2e]
                 The integration tests on the adapter's harness, locally; with
                 --e2e, their e2e cases on a fork of devnet (DEVNET_RPC_URL,
                 QUEUE_BASE_URL, QUEUE_AUTH_TOKEN, read by the harness)
  test-program [--check]
                 Write the deterministic build to the integration tests'
                 programs/, the binary they load; with --check, fail when the
                 committed binary is not, byte for byte, the fresh build
USAGE
  exit 1
}

# The binary the integration tests load, as the adapter's harness ships its
# programs: the deterministic build at the local addresses.
TEST_PROGRAM="${PROJECT_DIR}/crates/integration-test/programs/${PROGRAM_NAME}.so"

cd "$PROJECT_DIR"
load_program_ids localnet

case "${1:-}" in
  fmt)
    require_cmd cargo
    cargo fmt --all -- --check
    (cd crates/integration-test && cargo fmt -- --check)
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
    (cd crates/integration-test && cargo clippy --all-targets --features e2e -- -D warnings)
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
  integration-test)
    require_cmd cargo
    case "${2:-}" in
      "") (cd crates/integration-test && cargo test) ;;
      --e2e) (cd crates/integration-test && RUST_TEST_THREADS=1 cargo test --features e2e e2e_test) ;;
      *) usage ;;
    esac
    ;;
  test-program)
    deterministic_build
    case "${2:-}" in
      "")
        mkdir -p "$(dirname "$TEST_PROGRAM")"
        cp "$PROGRAM_SO" "$TEST_PROGRAM"
        echo "Wrote ${TEST_PROGRAM} ($(solana-verify get-executable-hash "$TEST_PROGRAM"))"
        ;;
      --check)
        if [[ ! -f "$TEST_PROGRAM" ]]; then
          echo "❌ ${TEST_PROGRAM} is missing; write it with ./scripts/dev.sh test-program." >&2
          exit 1
        elif cmp -s "$PROGRAM_SO" "$TEST_PROGRAM"; then
          echo "✅ the committed test program is the deterministic build"
        else
          echo "❌ the committed test program is not the deterministic build; rewrite it with ./scripts/dev.sh test-program." >&2
          exit 1
        fi
        ;;
      *)
        usage
        ;;
    esac
    ;;
  *)
    usage
    ;;
esac
