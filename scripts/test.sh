#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

mode=${1:-fast}

usage() {
  printf 'usage: %s [fast [filter]|soak|all]\n' "$0" >&2
  exit 2
}

run_step() {
  local label=$1
  shift
  local started=$SECONDS
  local elapsed
  local duration
  printf '\n==> %s\n' "$label"
  "$@"
  elapsed=$((SECONDS - started))
  if ((elapsed == 0)); then
    duration='<1s'
  else
    duration="${elapsed}s"
  fi
  printf '<== %s passed in %s\n' "$label" "$duration"
}

run_fast() {
  local filter=${1:-}
  local command=(cargo test --quiet --locked -j 2)
  local label='Fast Cargo tests'
  if [[ -n "$filter" ]]; then
    command+=("$filter")
    label="Fast tests matching '$filter'"
  fi
  run_step "$label" "${command[@]}"
}

run_soak() {
  run_step 'Deterministic soak tests' \
    cargo test --quiet --locked -j 2 test_deterministic_ -- --ignored --test-threads=1
}

case "$mode" in
  fast)
    [[ $# -le 2 ]] || usage
    run_fast "${2:-}"
    ;;
  soak)
    [[ $# -eq 1 ]] || usage
    run_soak
    ;;
  all)
    [[ $# -eq 1 ]] || usage
    run_step 'Shell syntax checks' bash -n scripts/test.sh scripts/verify_cli.sh
    run_fast
    run_soak
    run_step 'CLI smoke tests' bash scripts/verify_cli.sh
    ;;
  *)
    usage
    ;;
esac
