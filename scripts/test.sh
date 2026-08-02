#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

mode=${1:-fast}

usage() {
  cat >&2 <<EOF
usage:
  $0 fast [filter]       run non-ignored library tests
  $0 exact <test-name>   run one fully qualified library test
  $0 list [filter]       list matching library tests
  $0 soak                run ignored deterministic soak tests
  $0 cli                 run CLI smoke tests
  $0 all                 run syntax, library, doc, soak, and CLI tests
EOF
  exit 2
}

format_duration() {
  local elapsed=$1
  if ((elapsed == 0)); then
    printf '<1s'
  else
    printf '%ss' "$elapsed"
  fi
}

run_step() {
  local label=$1
  shift
  local started=$SECONDS
  local status

  printf '\n==> %s\n' "$label"
  if "$@"; then
    printf '<== %s passed in %s\n' "$label" "$(format_duration "$((SECONDS - started))")"
    return 0
  else
    status=$?
  fi

  printf '<== %s FAILED in %s\n' "$label" "$(format_duration "$((SECONDS - started))")" >&2
  return "$status"
}

matching_tests() {
  local filter=$1
  shift
  local output
  output=$(cargo test --quiet --locked --lib "$filter" -- --list "$@") || return
  printf '%s\n' "$output" | grep ': test$' || true
}

require_matching_test() {
  local filter=$1
  shift
  local matches
  matches=$(matching_tests "$filter" "$@") || return
  if [[ -z "$matches" ]]; then
    printf 'no library tests matched %q\n' "$filter" >&2
    return 2
  fi
}

require_matching_fast_test() {
  local filter=$1
  local listed
  local matches
  listed=$(matching_tests "$filter") || return
  matches=$(printf '%s\n' "$listed" | grep -v '::soak::' || true)
  if [[ -z "$matches" ]]; then
    printf 'no non-ignored library tests matched %q\n' "$filter" >&2
    return 2
  fi
}

run_fast() {
  local filter=${1:-}
  local command=(cargo test --quiet --locked --lib)
  local label='Library tests'
  if [[ -n "$filter" ]]; then
    require_matching_fast_test "$filter"
    command+=("$filter")
    label="Library tests matching '$filter'"
  fi
  run_step "$label" "${command[@]}"
}

run_exact() {
  local test_name=$1
  require_matching_test "$test_name" --exact
  run_step "Library test '$test_name'" \
    cargo test --quiet --locked --lib "$test_name" -- --exact --include-ignored
}

list_tests() {
  local filter=${1:-}
  local output
  local matches
  if [[ -n "$filter" ]]; then
    matches=$(matching_tests "$filter") || return
  else
    output=$(cargo test --quiet --locked --lib -- --list) || return
    matches=$(printf '%s\n' "$output" | grep ': test$' || true)
  fi
  if [[ -z "$matches" ]]; then
    printf 'no library tests matched %q\n' "$filter" >&2
    return 2
  fi
  printf '%s\n' "$matches"
}

run_soak() {
  run_step 'Deterministic soak tests' \
    cargo test --quiet --locked --lib '::soak::' -- --ignored --test-threads=1
}

case "$mode" in
  fast)
    [[ $# -le 2 ]] || usage
    run_fast "${2:-}"
    ;;
  exact)
    [[ $# -eq 2 ]] || usage
    run_exact "$2"
    ;;
  list)
    [[ $# -le 2 ]] || usage
    list_tests "${2:-}"
    ;;
  soak)
    [[ $# -eq 1 ]] || usage
    run_soak
    ;;
  cli)
    [[ $# -eq 1 ]] || usage
    run_step 'CLI smoke tests' bash scripts/verify_cli.sh
    ;;
  all)
    [[ $# -eq 1 ]] || usage
    run_step 'Shell syntax checks' bash -n scripts/test.sh scripts/verify_cli.sh
    run_fast
    run_step 'Documentation tests' cargo test --quiet --locked --doc
    run_soak
    run_step 'CLI smoke tests' bash scripts/verify_cli.sh
    ;;
  *)
    usage
    ;;
esac
