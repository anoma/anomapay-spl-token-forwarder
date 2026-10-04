#!/usr/bin/env bash
# Runs ops.sh's commands in the repository's Nix dev shell, entering it when
# not already in it.
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

run_in_project() {
  local cmd="$1"
  if [[ -n "${IN_NIX_SHELL:-}" ]]; then
    (cd "$PROJECT_DIR" && bash --noprofile --norc -c "$cmd")
  else
    nix --extra-experimental-features 'nix-command flakes' develop "$PROJECT_DIR" \
      --command bash --noprofile --norc -c "cd '$PROJECT_DIR' && $cmd"
  fi
}

case "${1:-}" in
  shell)
    cd "$PROJECT_DIR"
    if [[ -n "${IN_NIX_SHELL:-}" ]]; then
      exec bash
    fi
    exec nix --extra-experimental-features 'nix-command flakes' develop
    ;;
  run)
    # A command in the Nix shell, with the local addresses exported.
    shift
    run_in_project "set -a && . env/localnet.env && set +a && $(printf '%q ' "$@")"
    ;;
  "")
    echo "Usage: ./scripts/dev.sh shell | run <cmd> | <ops.sh command>" >&2
    run_in_project "./scripts/ops.sh"
    ;;
  *)
    run_in_project "./scripts/ops.sh $(printf '%q ' "$@")"
    ;;
esac
