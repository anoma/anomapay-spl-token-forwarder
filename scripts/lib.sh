#!/usr/bin/env bash
# Shared by dev.sh and ops.sh: the program's addresses and builds. Sourcing
# has no side effects beyond variable defaults, so it is safe outside the Nix
# shell.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

PROGRAM_NAME="spl_token_forwarder"
PROGRAM_SO="target/deploy/${PROGRAM_NAME}.so"
PROGRAM_IDL="target/idl/${PROGRAM_NAME}.json"
# The development build's Cargo features, and the instructions they add,
# which the production build's IDL must lack (build_release checks it).
DEV_FEATURES="dev-config-version"
DEV_ONLY_IX="dev_set_config_version"

# The SBPF version every build targets: local and CI builds, and the
# solana-verify deterministic build, whose `cargo build-sbf` would otherwise
# default to v0. The adapter's builds target the same.
SBPF_ARCH="v3"

# solana-verify maps [workspace.metadata.cli] solana (Cargo.toml) to a build
# image through a table compiled into each release; 0.5.2 is the release whose
# table has the Solana version pinned there.
SOLANA_VERIFY_VERSION="0.5.2"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "❌ Missing required command: $1. Run this inside the Nix dev shell (./scripts/dev.sh shell)." >&2
    exit 1
  fi
}

# Export the forwarder's and the adapter's address for <cluster>: the program
# and the adapter crate it builds against read them at compile time
# (env/README.md).
load_program_ids() {
  local cluster="$1" file="${PROJECT_DIR}/env/$1.env" var
  if [[ ! -f "$file" ]]; then
    echo "❌ ${file} is missing; it names the forwarder's and the adapter's addresses on ${cluster}." >&2
    exit 1
  fi
  set -a
  source "$file"
  set +a
  for var in FORWARDER_PROGRAM_ID PROTOCOL_ADAPTER_PROGRAM_ID; do
    if [[ -z "${!var:-}" ]]; then
      echo "❌ ${file} names no ${var}." >&2
      exit 1
    fi
  done
}

# The program's address is configuration (env/<cluster>.env) and its keypair
# is a secret (env/<cluster>.keys.env, never committed): fail if a keypair
# file is tracked or the program declares its address as a literal.
check_program_id_not_in_tree() {
  local tracked literal failed=0
  tracked="$(git -C "$PROJECT_DIR" ls-files -- '*-keypair.json' 'keypairs/*.json')"
  if [[ -n "$tracked" ]]; then
    echo "❌ Program keypairs are tracked; they belong outside the repository, named in env/<cluster>.keys.env:" >&2
    echo "$tracked" >&2
    failed=1
  fi
  if literal="$(grep -rn 'declare_id!("' "${PROJECT_DIR}/programs")"; then
    echo "❌ The program declares its address as a literal; it comes from env/<cluster>.env:" >&2
    echo "$literal" >&2
    failed=1
  fi
  return "$failed"
}

# Run an SBF build command and fail if it failed or reported a stack-frame
# overflow. The SBF backend reports a function whose frame exceeds the 4 KiB
# limit as an "Error: Function ... overflows the maximum allowed frame space"
# line yet still exits 0; such a function corrupts memory when it runs on
# chain.
checked_sbf_build() {
  local build_log build_status=0 grep_status=0
  build_log="$(mktemp)"
  "$@" 2>&1 | tee "$build_log" || build_status=$?
  grep -E "overflows the maximum allowed frame space|Stack offset of .* exceeded max offset" "$build_log" || grep_status=$?
  rm "$build_log"
  if ((build_status != 0)); then
    echo "❌ $* exited with status ${build_status}." >&2
    exit "$build_status"
  fi
  case "$grep_status" in
    0)
      echo "❌ The SBF build reported stack-frame overflows (above); the program would crash on chain." >&2
      exit 1
      ;;
    1) ;; # grep found no overflow report
    *)
      echo "❌ Could not scan the build log for stack-frame overflows (grep status ${grep_status})." >&2
      exit 1
      ;;
  esac
}

# The development build: the dev features on, with its IDL and TypeScript
# types unless $1 is "noidl".
build_dev() {
  local idl_flag=()
  if [[ "${1:-}" == "noidl" ]]; then
    idl_flag=(--no-idl)
  fi
  echo "    Building the forwarder (development build)..."
  checked_sbf_build anchor build --arch "$SBPF_ARCH" -p "$PROGRAM_NAME" "${idl_flag[@]}" -- --features "$DEV_FEATURES"
}

# The production build: no dev features. Its IDL must be the development IDL
# minus the dev-only instructions, so this stays a self-checking command
# rather than a convention nothing enforces.
build_release() {
  echo "    Building the forwarder (production build)..."
  rm -f "$PROGRAM_IDL"
  checked_sbf_build anchor build --arch "$SBPF_ARCH" -p "$PROGRAM_NAME"
  if [[ ! -f "$PROGRAM_IDL" ]]; then
    echo "❌ release build: anchor build did not produce an IDL at ${PROGRAM_IDL}" >&2
    exit 1
  fi
  assert_release_idl_lacks_dev_only
}

# The production IDL must be exactly the development IDL minus DEV_ONLY_IX,
# and each of those instructions must be in the development IDL: the check
# fails when a dev-only instruction reaches the production build, and when the
# development build gains or loses anything DEV_ONLY_IX does not account for.
assert_release_idl_lacks_dev_only() {
  local dev_idl
  dev_idl="$(mktemp --suffix .json)"
  anchor idl build -p "$PROGRAM_NAME" -o "$dev_idl" -- --features "$DEV_FEATURES"
  if ! node -e '
    const fs = require("fs");
    const [devPath, releasePath, declaredList] = process.argv.slice(1);
    const dev = JSON.parse(fs.readFileSync(devPath, "utf-8"));
    const release = JSON.parse(fs.readFileSync(releasePath, "utf-8"));
    const declared = declaredList.split(",");
    const devNames = dev.instructions.map((ix) => ix.name);
    const releaseNames = release.instructions.map((ix) => ix.name);
    const problems = [];
    const report = (what, names) => names.length && problems.push(`${what}: ${names.join(", ")}`);
    report("declared dev-only but absent from the development IDL", declared.filter((n) => !devNames.includes(n)));
    report("dev-only instruction present in the production IDL", declared.filter((n) => releaseNames.includes(n)));
    report(
      "only in the development IDL but not declared dev-only",
      devNames.filter((n) => !releaseNames.includes(n) && !declared.includes(n)),
    );
    report("only in the production IDL", releaseNames.filter((n) => !devNames.includes(n)));
    const expected = { ...dev, instructions: dev.instructions.filter((ix) => !declared.includes(ix.name)) };
    if (problems.length === 0 && JSON.stringify(expected) !== JSON.stringify(release)) {
      problems.push("the IDLs differ beyond the declared dev-only instructions (types, accounts, events, or instruction signatures)");
    }
    if (problems.length > 0) {
      console.error(`❌ release build: the production IDL (${releasePath}) is not the development IDL minus the declared dev-only instructions (${declaredList}):`);
      for (const p of problems) console.error(`   ${p}`);
      process.exit(1);
    }
  ' "$dev_idl" "$PROGRAM_IDL" "$DEV_ONLY_IX"; then
    rm -f "$dev_idl"
    exit 1
  fi
  rm -f "$dev_idl"
}

# The deterministic build of the forwarder at the loaded addresses, into
# target/deploy, with any further `cargo build` arguments given (the
# development build's features). solana-verify builds in a container from the
# repository alone, passing its trailing arguments to `cargo build`; the
# addresses go in as cargo [env] configuration, which a remote verification
# repeats.
deterministic_build() {
  # A missing solana-verify fails the substitution ("command not found") and
  # the comparison both.
  if [[ "$(solana-verify --version)" != "solana-verify ${SOLANA_VERIFY_VERSION}" ]]; then
    echo "❌ The deterministic build needs solana-verify ${SOLANA_VERIFY_VERSION}. Install with:" >&2
    echo "   cargo install solana-verify --version ${SOLANA_VERIFY_VERSION} --locked" >&2
    exit 1
  fi
  checked_sbf_build solana-verify build --library-name "$PROGRAM_NAME" --arch "$SBPF_ARCH" -- \
    --config "env.FORWARDER_PROGRAM_ID=\"${FORWARDER_PROGRAM_ID}\"" \
    --config "env.PROTOCOL_ADAPTER_PROGRAM_ID=\"${PROTOCOL_ADAPTER_PROGRAM_ID}\"" "$@"
}
