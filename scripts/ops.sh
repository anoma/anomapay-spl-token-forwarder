#!/usr/bin/env bash
# The forwarder's commands, run in the current environment: inside the Nix dev
# shell (./scripts/dev.sh <command> enters it), or in CI, which installs the
# same pinned tools.
set -euo pipefail

# shellcheck source=lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<USAGE
Usage: ./scripts/ops.sh <command> [argument] [flags]

Development:
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
  typecheck      Type-check and format-check the operator scripts against the
                 production IDL's types (the program is not compiled), and
                 check the client's copy of the IDL is that IDL
  script-test    Run the operator scripts against the harness's local runtime
                 (crates/integration-test/tests/operator_scripts.rs), after
                 installing their dependencies and generating the types they
                 import
  ts-test        Check the Rust and TypeScript bindings carry one version,
                 install the TypeScript bindings' (ts/) locked dependencies,
                 type-check them and run their tests

Cluster operations (--cluster required):
  deploy         First-time deploy of the production build. On localnet the
                 forwarder is initialized at once; on devnet/mainnet its IDL
                 is published, and forwarder init is left for after the
                 metadata account is given to the owner.
  upgrade        Rebuild and upgrade in place: through the forwarder's own
                 upgrade (owner wallet) once its upgrade authority is its PDA,
                 else through the loader
  forwarder <cmd>
                 init, reinitialize, emergency-withdraw. Parameters are STF_*
                 environment variables; see scripts/forwarder.ts.
  lookup-table   Add the forwarder's accounts, and STF_TOKEN_MINTS' escrow
                 accounts, to the deployment's settlement lookup table
                 PA_LOOKUP_TABLE (its authority's wallet). See
                 scripts/lookup-table.ts.
  idl-publish    Publish the production IDL on chain (the program's canonical
                 Program Metadata IDL account; signer must be the upgrade
                 authority to create it, the upgrade authority or its
                 authority to update it)
  status         Show the wallet's address and balance, and whether the
                 forwarder is deployed and initialized

Flags:
  --cluster <c>    localnet, devnet or mainnet. The addresses come from
                   env/<cluster>.env; a first deploy reads the forwarder's
                   keypair path from the uncommitted env/<cluster>.keys.env
                   (FORWARDER_PROGRAM_KEYPAIR=<path>).
  --wallet <path>  Wallet keypair. Defaults: devnet → scripts/devnet-wallet.json,
                   localnet → ~/.config/solana/id.json, mainnet → none (required).
                   The wallet must exist; nothing is auto-generated.
  --url <rpc>      The cluster's RPC endpoint. devnet and mainnet have no
                   default: pass --url or set DEVNET_RPC_URL / MAINNET_RPC_URL
                   (the operator's RPC provider, never the public endpoint)
                   The scripts confirm over its websocket at the RPC port plus
                   one; set ANCHOR_WS_URL when the endpoint's is elsewhere.
  --prebuilt       deploy/upgrade: ship the existing target/deploy artifact
                   (verify-build's) without rebuilding
  --e2e            integration-test: the e2e cases
  --check          test-program: compare instead of writing

Forwarder initialization parameters (required by forwarder init and a
localnet deploy, which initializes):
  STF_LOGIC_REF        32-byte hex logic ref the forwarder serves
  STF_EMERGENCY_COMMITTEE
                       base58 pubkey of the emergency committee
  STF_OWNER            base58 pubkey of the forwarder's initial owner, who
                       alone upgrades it and rotates its logic ref
  STF_TOKEN_MINT       optional: base58 mint whose escrow ATA to create
USAGE
  exit 1
}

# ---------- argument parsing ----------

COMMAND=""
ARGUMENT=""
CLUSTER=""
WALLET_OVERRIDE=""
RPC_OVERRIDE=""
PREBUILT=false
E2E=false
CHECK=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cluster)
      [[ $# -ge 2 ]] || { echo "❌ --cluster requires a value" >&2; exit 1; }
      CLUSTER="$2"
      shift 2
      ;;
    --wallet)
      [[ $# -ge 2 ]] || { echo "❌ --wallet requires a value" >&2; exit 1; }
      WALLET_OVERRIDE="$2"
      shift 2
      ;;
    --url)
      [[ $# -ge 2 ]] || { echo "❌ --url requires a value" >&2; exit 1; }
      RPC_OVERRIDE="$2"
      shift 2
      ;;
    --prebuilt)
      PREBUILT=true
      shift
      ;;
    --e2e)
      E2E=true
      shift
      ;;
    --check)
      CHECK=true
      shift
      ;;
    --*)
      echo "❌ Unknown flag: $1" >&2
      usage
      ;;
    *)
      if [[ -z "$COMMAND" ]]; then
        COMMAND="$1"
      elif [[ -z "$ARGUMENT" ]]; then
        ARGUMENT="$1"
      else
        echo "❌ Unexpected argument: $1" >&2
        usage
      fi
      shift
      ;;
  esac
done

[[ -n "$COMMAND" ]] || usage

# The binaries the integration tests load, as the adapter's harness ships its
# programs: the deterministic builds at the local addresses, production and
# development (the dev features' instructions, which some tests use).
TEST_PROGRAM="${PROJECT_DIR}/crates/integration-test/programs/${PROGRAM_NAME}.so"
TEST_PROGRAM_DEV="${PROJECT_DIR}/crates/integration-test/programs/${PROGRAM_NAME}_dev.so"

# Write the fresh build in target/deploy to the test program $1; with
# --check, fail instead when $1 is not, byte for byte, the fresh build.
write_test_program() {
  local committed="$1"
  if [[ "$CHECK" != "true" ]]; then
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

# Yarn's install of the operator scripts' dependencies, exactly as yarn.lock
# pins them.
ensure_node_modules() {
  yarn install --frozen-lockfile
}

# ---------- cluster resolution ----------

RPC_URL=""
EXPLORER_QS=""
WALLET=""

resolve_cluster() {
  local default_wallet=""
  case "$CLUSTER" in
    localnet)
      # The local test validator's defaults.
      RPC_URL="http://127.0.0.1:8899"
      default_wallet="${HOME}/.config/solana/id.json"
      ;;
    devnet)
      RPC_URL="${DEVNET_RPC_URL:-}"
      EXPLORER_QS="?cluster=devnet"
      default_wallet="${PROJECT_DIR}/scripts/devnet-wallet.json"
      ;;
    mainnet)
      RPC_URL="${MAINNET_RPC_URL:-}"
      ;;
    "")
      echo "❌ Missing --cluster <localnet|devnet|mainnet>" >&2
      exit 1
      ;;
    *)
      echo "❌ Unknown cluster: ${CLUSTER} (expected localnet, devnet, or mainnet)" >&2
      exit 1
      ;;
  esac

  if [[ -n "$RPC_OVERRIDE" ]]; then
    RPC_URL="$RPC_OVERRIDE"
  fi
  # A cluster operation goes through the operator's RPC provider, never the
  # rate-limited public endpoint.
  if [[ -z "$RPC_URL" ]]; then
    echo "❌ No RPC endpoint for ${CLUSTER}: pass --url <rpc> or set ${CLUSTER^^}_RPC_URL." >&2
    exit 1
  fi

  WALLET="${WALLET_OVERRIDE:-$default_wallet}"
  if [[ -z "$WALLET" ]]; then
    echo "❌ No default wallet for cluster '${CLUSTER}' — pass --wallet <path>" >&2
    exit 1
  fi
  if [[ ! -f "$WALLET" ]]; then
    echo "❌ Wallet not found: ${WALLET}" >&2
    echo "   Nothing is auto-generated. Provide an existing, funded keypair" >&2
    echo "   (or pass a different one with --wallet)." >&2
    exit 1
  fi
}

# ---------- cluster helpers ----------

# The SOL a first deploy of the forwarder needs: its program data's rent and
# the fees.
DEPLOY_SOL=3

get_wallet_pubkey() {
  solana-keygen pubkey "$WALLET"
}

get_balance() {
  # Returns numeric SOL balance (e.g. "3.5")
  solana balance --keypair "$WALLET" --url "$RPC_URL" | awk '{print $1}'
}

ensure_balance() {
  local min_sol="$1"
  local balance
  balance="$(get_balance)"

  if awk "BEGIN{exit ($balance >= $min_sol) ? 0 : 1}"; then
    echo "Balance: ${balance} SOL (need ${min_sol})"
    return 0
  fi

  echo "❌ Insufficient balance: ${balance} SOL (need ${min_sol})"
  echo "Wallet: $(get_wallet_pubkey)"
  echo "Transfer SOL to this wallet before proceeding."
  exit 1
}

# True when <program_id> is a program on the cluster, false when no account
# exists there; any other failure (an unreachable RPC) exits.
is_deployed() {
  local program_id="$1" out
  if out="$(solana program show "$program_id" --url "$RPC_URL" 2>&1)"; then
    return 0
  fi
  if [[ "$out" == "Error: Unable to find the account ${program_id}" ]]; then
    return 1
  fi
  echo "❌ solana program show ${program_id} failed: ${out}" >&2
  exit 1
}

# Exit unless the forwarder is deployed on the cluster.
require_forwarder_deployed() {
  if ! is_deployed "$FORWARDER_PROGRAM_ID"; then
    echo "❌ The forwarder (${FORWARDER_PROGRAM_ID}) is not deployed on ${CLUSTER}"
    echo "Run: ./scripts/dev.sh deploy --cluster ${CLUSTER}"
    exit 1
  fi
}

# The explorer link of <address> on devnet and mainnet; the explorer does not
# see localnet.
print_explorer_link() {
  local address="$1"
  if [[ "$CLUSTER" != "localnet" ]]; then
    echo "  https://explorer.solana.com/address/${address}${EXPLORER_QS}"
  fi
}

# An upgrade writes the new binary over the existing program-data account,
# which must be at least as large. `solana program deploy` extends it by the
# exact shortfall, but the upgradeable loader rejects ExtendProgram below
# 10240 bytes ("ExtendProgram requires a minimum of 10240 additional bytes or
# to extend to maximum size"), so a small growth fails the deploy. Extend by
# the shortfall, raised to that minimum, before deploying.
extend_program_data_if_needed() {
  local program_id="$FORWARDER_PROGRAM_ID"
  is_deployed "$program_id" || return 0
  local current_len new_len shortfall
  current_len="$(solana program show "$program_id" --url "$RPC_URL" | awk '/^Data Length:/ {print $3}')"
  [[ "$current_len" =~ ^[0-9]+$ ]] || { echo "❌ Could not read the data length of ${program_id}" >&2; exit 1; }
  new_len="$(stat -c %s "$PROGRAM_SO")"
  (( new_len > current_len )) || return 0
  shortfall=$(( new_len - current_len ))
  local min_extend=10240
  (( shortfall < min_extend )) && shortfall=$min_extend
  echo "Extending the forwarder's program data by ${shortfall} bytes (${current_len} on chain, ${new_len} needed)..."
  solana program extend "$program_id" "$shortfall" --keypair "$WALLET" --url "$RPC_URL"
}

# The path of the forwarder's keypair, from the uncommitted
# env/<cluster>.keys.env (FORWARDER_PROGRAM_KEYPAIR=<path>). It must
# be the keypair of the address env/<cluster>.env gives the forwarder: the
# binary has that address compiled in, and every PDA derivation depends on it.
program_keypair() {
  local keys path actual
  keys="${PROJECT_DIR}/env/${CLUSTER}.keys.env"
  if [[ -f "$keys" ]]; then
    path="$(set -a; source "$keys"; echo "${FORWARDER_PROGRAM_KEYPAIR:-}")"
  fi
  if [[ -z "${path:-}" ]]; then
    echo "❌ The forwarder is not deployed at ${FORWARDER_PROGRAM_ID}; creating it takes its keypair." >&2
    echo "   Name the keypair's path in ${keys}: FORWARDER_PROGRAM_KEYPAIR=<path>" >&2
    exit 1
  fi
  if [[ ! -f "$path" ]]; then
    echo "❌ FORWARDER_PROGRAM_KEYPAIR names ${path}, which does not exist." >&2
    exit 1
  fi
  actual="$(solana-keygen pubkey "$path")"
  if [[ "$actual" != "$FORWARDER_PROGRAM_ID" ]]; then
    echo "❌ ${path} is the keypair of ${actual}, but env/${CLUSTER}.env gives the forwarder the address ${FORWARDER_PROGRAM_ID}." >&2
    exit 1
  fi
  echo "$path"
}

deploy_program() {
  local program_id="$FORWARDER_PROGRAM_ID" program_id_arg
  # Creating a program at its address takes the address's keypair; a program
  # already there is redeployed by address.
  program_id_arg="$program_id"
  if ! is_deployed "$program_id"; then
    program_id_arg="$(program_keypair)"
  fi

  extend_program_data_if_needed
  echo "Deploying the forwarder (${program_id})..."
  if ! solana program deploy \
    "$PROGRAM_SO" \
    --keypair "$WALLET" \
    --program-id "$program_id_arg" \
    --url "$RPC_URL" 2>&1; then
    echo ""
    echo "❌ Deploy failed."
    echo "If the program was previously closed, the address is permanently burned:"
    echo "generate a new keypair outside the repository, set its address in"
    echo "env/${CLUSTER}.env and its path in env/${CLUSTER}.keys.env, then deploy again."
    exit 1
  fi
  echo "  ✅ Forwarder deployed: ${program_id}"
  print_explorer_link "$program_id"
}

# The owner writes the new code to a loader buffer and calls the program's
# `upgrade` with it, which hands the buffer to the program's upgrade
# authority PDA and upgrades.
upgrade_through_program() {
  local program_id="$FORWARDER_PROGRAM_ID" output buffer
  extend_program_data_if_needed
  echo "Writing the forwarder's code to a buffer..."
  output="$(solana program write-buffer "$PROGRAM_SO" --keypair "$WALLET" --url "$RPC_URL")"
  echo "$output"
  buffer="$(awk '/^Buffer:/ {print $2}' <<<"$output")"
  if [[ -z "$buffer" ]]; then
    echo "❌ solana program write-buffer printed no buffer address" >&2
    exit 1
  fi
  if ! run_ts scripts/upgrade-program.ts upgrade "$buffer"; then
    echo "❌ The upgrade failed. Buffer ${buffer} still holds its rent, under the wallet's authority:" >&2
    echo "   reclaim it with: solana program close ${buffer} --keypair ${WALLET} --url <rpc>" >&2
    exit 1
  fi
  echo "  ✅ Forwarder upgraded: ${program_id}"
  print_explorer_link "$program_id"
}

# The artifact a deploy or upgrade ships: the existing one with --prebuilt
# (verify-build's deterministic build, which a rebuild here would clobber),
# else a fresh production build.
build_for_deploy() {
  if [[ "$PREBUILT" == "true" ]]; then
    if [[ ! -f "$PROGRAM_SO" ]]; then
      echo "❌ --prebuilt: ${PROGRAM_SO} does not exist. Build it first (verify-build)." >&2
      exit 1
    fi
    echo "    Deploying the prebuilt artifact ${PROGRAM_SO} (no build)."
    return 0
  fi
  build_release
}

run_ts() {
  local script="$1"
  shift
  ANCHOR_PROVIDER_URL="$RPC_URL" \
  ANCHOR_WALLET="$WALLET" \
    npx ts-node -P tsconfig.json "$script" "$@"
}

require_forwarder_init_params() {
  if [[ -z "${STF_LOGIC_REF:-}" || -z "${STF_EMERGENCY_COMMITTEE:-}" || -z "${STF_OWNER:-}" ]]; then
    echo "❌ Missing STF_LOGIC_REF, STF_EMERGENCY_COMMITTEE and/or STF_OWNER." >&2
    echo "   The forwarder config pins the logic ref it serves, the committee" >&2
    echo "   that can act in an emergency and the owner; there is no safe default." >&2
    exit 1
  fi
}

# The checks every operator script's command makes, then the production IDL
# and types the script imports, then scripts/<script> with <args>.
run_operator_script() {
  require_cmd yarn
  require_forwarder_deployed
  ensure_node_modules
  release_idl
  run_ts "scripts/$1" "${@:2}"
}

# ---------- cluster commands ----------

cmd_deploy() {
  require_cmd anchor
  require_cmd yarn
  # A localnet deploy initializes the forwarder (below): check its parameters
  # before deploying.
  if [[ "$CLUSTER" == "localnet" ]]; then
    require_forwarder_init_params
  fi
  if ! is_deployed "$FORWARDER_PROGRAM_ID"; then
    program_keypair >/dev/null
  fi
  ensure_balance "$DEPLOY_SOL"
  ensure_node_modules
  build_for_deploy
  deploy_program

  # `initialize` hands the program's upgrade authority to the program, and
  # only the upgrade authority creates the program's canonical metadata
  # account. On devnet and mainnet the IDL is published first, and
  # initialization waits until the deployer has given that account to the
  # owner, by hand (docs/OPERATIONS.md).
  if [[ "$CLUSTER" == "localnet" ]]; then
    echo "Initializing SPL token forwarder (idempotent)..."
    run_ts scripts/forwarder.ts init
  else
    cmd_idl_publish
    echo "Next, by hand: give the forwarder's canonical metadata account to its owner, then run" \
      "forwarder init (docs/OPERATIONS.md)."
  fi

  echo ""
  echo "✅ Deploy complete (${CLUSTER}): ${FORWARDER_PROGRAM_ID}"
}

# Upgrade in place along the path the forwarder's upgrade authority leaves:
# through the program when the authority is the program's own PDA, through
# the loader while the wallet still holds it (before `initialize` hands it
# over).
cmd_upgrade() {
  require_cmd anchor
  require_cmd yarn
  require_forwarder_deployed
  ensure_node_modules
  build_for_deploy
  local path
  path="$(run_ts scripts/upgrade-program.ts path)"
  case "$path" in
    loader) deploy_program ;;
    program) upgrade_through_program ;;
    *)
      echo "❌ upgrade-program.ts printed '${path}' as the forwarder's upgrade path" >&2
      exit 1
      ;;
  esac
  echo ""
  echo "✅ Upgrade complete (${CLUSTER}): ${FORWARDER_PROGRAM_ID}"
}

cmd_forwarder() {
  run_operator_script forwarder.ts "$ARGUMENT"
}

cmd_lookup_table() {
  run_operator_script lookup-table.ts
}

# Publish the production IDL on chain as the program's canonical Program
# Metadata "idl" account (derived from the program ID), so explorers and
# generic Anchor clients decode the program's instructions, accounts, and
# events straight from the cluster. The production build self-checks that the
# dev-only instructions are absent, so a development IDL cannot be published
# by accident. The program's upgrade authority creates the account; it or the
# account's explicit authority updates it (client/programMetadata.ts).
cmd_idl_publish() {
  run_operator_script publish-idl.ts "$PROGRAM_IDL"
}

cmd_status() {
  echo "=== Status (${CLUSTER}) ==="
  echo ""

  local pubkey balance
  pubkey="$(get_wallet_pubkey)"
  balance="$(get_balance)"
  echo "Wallet: ${pubkey}"
  echo "Balance: ${balance} SOL"
  print_explorer_link "$pubkey"
  echo ""

  if is_deployed "$FORWARDER_PROGRAM_ID"; then
    echo "Forwarder: ✅ deployed — ${FORWARDER_PROGRAM_ID}"
  else
    echo "Forwarder: not deployed — ${FORWARDER_PROGRAM_ID}"
  fi
  print_explorer_link "$FORWARDER_PROGRAM_ID"

  local config out
  config="$(solana find-program-derived-address "$FORWARDER_PROGRAM_ID" string:config --url "$RPC_URL" | awk 'NR == 1 { print $1 }')"
  if out="$(solana account "$config" --url "$RPC_URL" 2>&1)"; then
    echo "Config: ✅ initialized — ${config}"
  elif [[ "$out" == "Error: AccountNotFound: pubkey=${config}" ]]; then
    echo "Config: not initialized — ${config}"
  else
    echo "❌ solana account ${config} failed: ${out}" >&2
    exit 1
  fi
}

# ---------- dispatch ----------

cd "$PROJECT_DIR"
load_program_ids "${CLUSTER:-localnet}"


case "$COMMAND" in
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
    (cd crates/integration-test && cargo clippy --all-targets --features e2e,scripts -- -D warnings)
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
    if [[ "$E2E" == "true" ]]; then
      (cd crates/integration-test && RUST_TEST_THREADS=1 cargo test --features e2e e2e_test)
    else
      (cd crates/integration-test && cargo test)
    fi
    ;;
  test-program)
    failed=0
    deterministic_build
    write_test_program "$TEST_PROGRAM" || failed=1
    deterministic_build --features "$DEV_FEATURES"
    write_test_program "$TEST_PROGRAM_DEV" || failed=1
    exit "$failed"
    ;;
  script-test)
    require_cmd cargo
    require_cmd yarn
    ensure_node_modules
    release_idl
    (cd crates/integration-test && cargo test --features scripts --test operator_scripts)
    ;;
  typecheck)
    require_cmd yarn
    ensure_node_modules
    release_idl
    # The client bindings' copy of the IDL, which their decoders are tested
    # against, is the program's.
    if ! cmp "$PROGRAM_IDL" crates/client/idl/spl_token_forwarder.json; then
      echo "❌ crates/client/idl/spl_token_forwarder.json is not the program's IDL; copy $PROGRAM_IDL over it." >&2
      exit 1
    fi
    yarn run typecheck
    yarn run lint
    ;;
  ts-test)
    require_cmd npm
    # The Rust and TypeScript client bindings release together, at one version.
    crate=$(sed -n 's/^version = "\(.*\)"$/\1/p' crates/client/Cargo.toml)
    package=$(node -p "require('./ts/package.json').version")
    if [[ "$crate" != "$package" ]]; then
      echo "❌ The client crate is at $crate and the TS package at $package; give both the same version." >&2
      exit 1
    fi
    (cd ts && npm ci && npm run tsc && npm test)
    ;;
  deploy | upgrade | forwarder | lookup-table | idl-publish | status)
    require_cmd solana
    require_cmd solana-keygen
    resolve_cluster
    "cmd_${COMMAND//-/_}"
    ;;
  *)
    usage
    ;;
esac
