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
  $0 debug <test-name>   run one exact test with captured output disabled
  $0 list [filter]       list matching library tests
  $0 soak                run ignored deterministic soak tests
  $0 docs                run documentation consistency and doctests
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

run_test_step() {
  local label=$1
  local match_description=$2
  shift 2
  local started=$SECONDS
  local output_file
  local status

  output_file=$(mktemp)
  printf '\n==> %s\n' "$label"

  set +e
  "$@" >"$output_file" 2>&1
  status=$?
  set -e

  if ((status != 0)); then
    cat "$output_file" >&2
    rm -f "$output_file"
    printf '<== %s FAILED in %s\n' "$label" "$(format_duration "$((SECONDS - started))")" >&2
    return "$status"
  fi

  if [[ -n "$match_description" ]] \
    && grep -Eq 'test result: ok\. 0 passed; 0 failed;' "$output_file"; then
    rm -f "$output_file"
    printf 'no executable library tests matched %q\n' "$match_description" >&2
    printf '<== %s FAILED in %s\n' "$label" "$(format_duration "$((SECONDS - started))")" >&2
    return 2
  fi

  local summaries
  summaries=$(grep -E '^(running [0-9]+ tests?|test result:)' "$output_file" || true)
  if [[ -n "$summaries" ]]; then
    printf '%s\n' "$summaries" | sed 's/^/    /'
  fi
  rm -f "$output_file"
  printf '<== %s passed in %s\n' "$label" "$(format_duration "$((SECONDS - started))")"
}

matching_tests() {
  local filter=$1
  shift
  local output
  output=$(cargo test --quiet --locked --lib "$filter" -- --list "$@") || return
  printf '%s\n' "$output" | grep ': test$' || true
}

run_fast() {
  local filter=${1:-}
  local command=(cargo test --quiet --locked --lib)
  local label='Library tests'
  local match_description=''
  if [[ -n "$filter" ]]; then
    command+=("$filter")
    label="Library tests matching '$filter'"
    match_description=$filter
  fi
  run_test_step "$label" "$match_description" "${command[@]}"
}

run_exact() {
  local test_name=$1
  run_test_step "Library test '$test_name'" "$test_name" \
    cargo test --quiet --locked --lib "$test_name" -- --exact --include-ignored
}

run_debug() {
  local test_name=$1
  run_step "Debug library test '$test_name'" \
    cargo test --locked --lib "$test_name" -- --exact --include-ignored --nocapture
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

run_docs() {
  local python_command
  if python3 --version >/dev/null 2>&1; then
    python_command=python3
  elif python --version >/dev/null 2>&1; then
    python_command=python
  else
    printf 'Python is required for documentation consistency checks\n' >&2
    return 1
  fi
  run_step 'Documentation consistency' "$python_command" scripts/check_docs.py
  run_step 'Documentation tests' cargo test --quiet --locked --doc
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
  debug)
    [[ $# -eq 2 ]] || usage
    run_debug "$2"
    ;;
  list)
    [[ $# -le 2 ]] || usage
    list_tests "${2:-}"
    ;;
  soak)
    [[ $# -eq 1 ]] || usage
    run_soak
    ;;
  docs)
    [[ $# -eq 1 ]] || usage
    run_docs
    ;;
  cli)
    [[ $# -eq 1 ]] || usage
    run_step 'CLI smoke tests' bash scripts/verify_cli.sh
    ;;
  all)
    [[ $# -eq 1 ]] || usage
    run_step 'Shell syntax checks' bash -n scripts/test.sh scripts/verify_cli.sh
    run_fast
    run_docs
    run_soak
    run_step 'CLI smoke tests' bash scripts/verify_cli.sh
    ;;
  *)
    usage
    ;;
esac
