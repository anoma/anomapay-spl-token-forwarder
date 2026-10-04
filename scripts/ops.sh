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
                 Write the deterministic production and development builds
                 to the integration tests' programs/, the binaries they load;
                 with --check, fail when a committed binary is not, byte for
                 byte, the fresh build
USAGE
  exit 1
}

# The binaries the integration tests load, as the adapter's harness ships its
# programs: the deterministic builds at the local addresses, production and
# development (the dev features' instructions, which some tests use).
TEST_PROGRAM="${PROJECT_DIR}/crates/integration-test/programs/${PROGRAM_NAME}.so"
TEST_PROGRAM_DEV="${PROJECT_DIR}/crates/integration-test/programs/${PROGRAM_NAME}_dev.so"

# Write the fresh build in target/deploy to the test program $1; with
# --check ($2), fail instead when $1 is not, byte for byte, the fresh build.
write_test_program() {
  local committed="$1"
  if [[ "${2:-}" != "--check" ]]; then
    mkdir -p "$(dirname "$committed")"
    cp "$PROGRAM_SO" "$committed"
    echo "Wrote ${committed} ($(solana-verify get-executable-hash "$committed"))"
  elif [[ ! -f "$committed" ]]; then
    echo "❌ ${committed} is missing; write it with ./scripts/dev.sh test-program." >&2
    return 1
  elif cmp -s "$PROGRAM_SO" "$committed"; then
    echo "✅ ${committed} is the deterministic build"
  else
    echo "❌ ${committed} is not the deterministic build; rewrite it with ./scripts/dev.sh test-program." >&2
    return 1
  fi
}

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
    case "${2:-}" in
      "" | --check) ;;
      *) usage ;;
    esac
    failed=0
    deterministic_build
    write_test_program "$TEST_PROGRAM" "${2:-}" || failed=1
    deterministic_build --features "$DEV_FEATURES"
    write_test_program "$TEST_PROGRAM_DEV" "${2:-}" || failed=1
    exit "$failed"
    ;;
  *)
    usage
    ;;
esac
