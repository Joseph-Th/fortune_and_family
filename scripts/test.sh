#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

mode=${1:-fast}
test_jobs=${CIVIC_DYNASTY_JOBS:-}
if [[ -n "$test_jobs" ]] && ! [[ "$test_jobs" =~ ^[1-9][0-9]*$ ]]; then
  printf 'CIVIC_DYNASTY_JOBS must be a positive integer, got %q\n' "$test_jobs" >&2
  exit 2
fi
job_args=()
if [[ -n "$test_jobs" ]]; then
  job_args=(--jobs "$test_jobs")
fi

usage() {
  cat >&2 <<EOF
usage:
  $0 fast [filter]       run non-ignored library tests (default loop)
  $0 standard            pre-commit loop: syntax, library, docs, core CLI smoke
  $0 exact <test-name>   run one fully qualified library test
  $0 debug <test-name>   run one exact test with captured output enabled
  $0 list [filter]       list matching library tests
  $0 soak                run soak tests in release mode
  $0 docs                run documentation consistency and doctests
  $0 cli                 run core campaign/projection/dashboard CLI smoke tests
  $0 art-cli             run procedural-art CLI smoke tests
  $0 gameplay-cli        run gameplay-harness CLI smoke tests
  $0 adapters            run all CLI smoke groups
  $0 gameplay            run release gameplay and generation-length quality gates
  $0 gameplay-audit      run mature multi-seed, generation, and credit-stress audits
  $0 ci-verify           fast CI lane: format, clippy, library, docs
  $0 ci-gates            deep CI lane: release tests, soaks, adapters, gameplay, audit
  $0 all                 everything: standard + soak + adapters + gameplay
  $0 slow                heavy release gates (ci-gates minus audit)
  $0 deep                deepest design gates (slow + gameplay-audit)

environment:
  CIVIC_DYNASTY_JOBS      pass --jobs N to cargo test/build commands
  CIVIC_DYNASTY_PROFILE   debug (default) or release for adapter smoke builds
  CIVIC_DYNASTY_BINARY    reuse a prebuilt CLI binary for adapter smoke groups
  CIVIC_DYNASTY_PYTHON    select an explicit Python interpreter
EOF
  exit 2
}

resolve_python() {
  if [[ -n "${CIVIC_DYNASTY_PYTHON:-}" ]]; then
    if command -v "$CIVIC_DYNASTY_PYTHON" >/dev/null 2>&1; then
      printf '%s' "$CIVIC_DYNASTY_PYTHON"
      return 0
    fi
    printf 'CIVIC_DYNASTY_PYTHON is not executable: %s\n' "$CIVIC_DYNASTY_PYTHON" >&2
    return 1
  fi
  if python3 --version >/dev/null 2>&1; then
    printf 'python3'
  elif python --version >/dev/null 2>&1; then
    printf 'python'
  elif py --version >/dev/null 2>&1; then
    printf 'py'
  else
    printf 'Python is required for repository validation; install Python or set CIVIC_DYNASTY_PYTHON\n' >&2
    return 1
  fi
}

ensure_cli_binary() {
  local requested_profile=${1:-debug}
  if [[ -n "${CIVIC_DYNASTY_BINARY_OVERRIDE:-}" ]]; then
    # An explicit CIVIC_DYNASTY_BINARY_OVERRIDE wins over every profile choice,
    # including the release contract that gameplay gates rely on.
    export CIVIC_DYNASTY_BINARY="$CIVIC_DYNASTY_BINARY_OVERRIDE"
    if [[ ! -x "$CIVIC_DYNASTY_BINARY" && -x "${CIVIC_DYNASTY_BINARY}.exe" ]]; then
      export CIVIC_DYNASTY_BINARY="${CIVIC_DYNASTY_BINARY}.exe"
    fi
    if [[ ! -x "$CIVIC_DYNASTY_BINARY" ]]; then
      printf 'CIVIC_DYNASTY_BINARY_OVERRIDE is not executable: %q\n' "$CIVIC_DYNASTY_BINARY" >&2
      return 1
    fi
    return 0
  fi

  if [[ -n "${CIVIC_DYNASTY_BINARY:-}" ]]; then
    # A caller-provided CIVIC_DYNASTY_BINARY is honored, but the gameplay gates
    # must never inherit a debug binary: simulation throughput drops an order of
    # magnitude, so they rebuild the optimized CLI instead.
    if [[ "$requested_profile" != release || "$CIVIC_DYNASTY_BINARY" == *"/release/civic-dynasty"* || "$CIVIC_DYNASTY_BINARY" == *"/release/civic-dynasty.exe"* ]]; then
      if [[ ! -x "$CIVIC_DYNASTY_BINARY" && -x "${CIVIC_DYNASTY_BINARY}.exe" ]]; then
        export CIVIC_DYNASTY_BINARY="${CIVIC_DYNASTY_BINARY}.exe"
      fi
      if [[ ! -x "$CIVIC_DYNASTY_BINARY" ]]; then
        printf 'CIVIC_DYNASTY_BINARY is not executable: %q\n' "$CIVIC_DYNASTY_BINARY" >&2
        return 1
      fi
      return 0
    fi
    printf 'CIVIC_DYNASTY_BINARY %q is not a release binary; rebuilding the release CLI for gameplay gates\n' \
      "$CIVIC_DYNASTY_BINARY" >&2
  fi

  local profile_args=()
  case "$requested_profile" in
    debug)
      ;;
    release)
      profile_args=(--release)
      ;;
    *)
      printf 'unsupported CLI profile %q (expected debug or release)\n' "$requested_profile" >&2
      return 2
      ;;
  esac

  run_step "Build $requested_profile CLI once" \
    cargo build --quiet --locked "${job_args[@]}" "${profile_args[@]}" --bin civic-dynasty

  local binary="target/$requested_profile/civic-dynasty"
  if [[ ! -x "$binary" && -x "${binary}.exe" ]]; then
    binary="${binary}.exe"
  fi
  if [[ ! -x "$binary" ]]; then
    printf 'CLI binary %q was not produced by the %s build\n' "$binary" "$requested_profile" >&2
    return 1
  fi
  export CIVIC_DYNASTY_BINARY="$binary"
}

run_shell_syntax() {
  run_step 'Shell syntax checks' bash -n scripts/test.sh scripts/verify_cli.sh
}

run_cli_core() {
  run_step 'Core CLI smoke tests' bash scripts/verify_cli.sh core
}

run_cli_art() {
  run_step 'Art CLI smoke tests' bash scripts/verify_cli.sh art
}

run_cli_gameplay() {
  run_step 'Gameplay CLI smoke tests' bash scripts/verify_cli.sh gameplay
}

run_playtest() {
  ensure_cli_binary release
  "$CIVIC_DYNASTY_BINARY" playtest "$@"
}

run_standard() {
  run_shell_syntax
  run_fast
  run_docs
  ensure_cli_binary "${CIVIC_DYNASTY_PROFILE:-debug}"
  run_cli_core
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
    local ignored_count
    ignored_count=$(grep -Eo '[0-9]+ ignored' "$output_file" | head -n1 | grep -Eo '^[0-9]+' || printf '0')
    rm -f "$output_file"
    if [[ "$ignored_count" =~ ^[1-9][0-9]*$ ]]; then
      printf 'filter %q matched only %s ignored test(s); use `soak`, `exact`, or `debug` to include them\n' \
        "$match_description" "$ignored_count" >&2
    else
      printf 'no executable library tests matched %q\n' "$match_description" >&2
    fi
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
  output=$(cargo test --quiet --locked "${job_args[@]}" --lib "$filter" -- --list "$@") || return
  printf '%s\n' "$output" | grep ': test$' || true
}

run_fast() {
  local filter=${1:-}
  local command=(cargo test --quiet --locked --lib "${job_args[@]}")
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
  local profile_args=()
  if [[ "$test_name" == *::soak::* ]]; then
    profile_args=(--release)
  fi
  run_test_step "Library test '$test_name'" "$test_name" \
    cargo test --quiet --locked "${job_args[@]}" "${profile_args[@]}" --lib "$test_name" \
    -- --exact --include-ignored
}

run_debug() {
  local test_name=$1
  local profile_args=()
  if [[ "$test_name" == *::soak::* ]]; then
    profile_args=(--release)
  fi
  run_step "Debug library test '$test_name'" \
    cargo test --locked "${job_args[@]}" "${profile_args[@]}" --lib "$test_name" \
    -- --exact --include-ignored --nocapture
}

list_tests() {
  local filter=${1:-}
  local output
  local matches
  if [[ -n "$filter" ]]; then
    matches=$(matching_tests "$filter") || return
  else
    output=$(cargo test --quiet --locked "${job_args[@]}" --lib -- --list) || return
    matches=$(printf '%s\n' "$output" | grep ': test$' || true)
  fi
  if [[ -z "$matches" ]]; then
    printf 'no library tests matched %q\n' "$filter" >&2
    return 2
  fi
  printf '%s\n' "$matches"
}

run_soak() {
  # Soak tests simulate thousands of days; release mode runs them ~100x faster
  # than the debug test profile with the same deterministic assertions.
  run_step 'Deterministic soak tests (release)' \
    cargo test --release --quiet --locked "${job_args[@]}" --lib '::soak::' -- --ignored
}

run_docs() {
  local python_command
  python_command=$(resolve_python) || return
  run_step 'Documentation consistency' "$python_command" scripts/check_docs.py
  run_step 'Documentation tests' cargo test --quiet --locked "${job_args[@]}" --doc
}

run_gameplay() {
  run_step 'Gameplay quality gate' \
    run_playtest \
      --minimum-overall 75 \
      --fail-on-critical \
      --json \
      --output target/gameplay-quality-gate.json
}

run_generation_gameplay() {
  local python_command
  python_command=$(resolve_python) || return
  run_step 'Generation-length gameplay gate' \
    run_playtest \
      --days 7200 \
      --persona steward \
      --background baker \
      --trace-limit 20 \
      --minimum-overall 75 \
      --fail-on-critical \
      --json \
      --output target/gameplay-generation-gate.json
  run_step 'Generation-length fantasy validation' "$python_command" scripts/check_gameplay.py generation \
    target/gameplay-generation-gate.json
}

run_gameplay_audit() {
  local python_command
  python_command=$(resolve_python) || return
  run_step 'Mature multi-seed gameplay audit' \
    run_playtest \
      --days 3600 \
      --start-seed 1 \
      --seeds 2 \
      --trace-limit 30 \
      --minimum-overall 75 \
      --fail-on-critical \
      --json \
      --output target/gameplay-deep-audit.json
  run_step 'Generation-length persona audit' \
    run_playtest \
      --days 7200 \
      --persona steward \
      --persona entrepreneur \
      --persona power-broker \
      --persona opportunist \
      --background baker \
      --trace-limit 30 \
      --minimum-overall 75 \
      --fail-on-critical \
      --json \
      --output target/gameplay-generation-matrix.json
  run_step 'Opportunist credit stress audit' \
    run_playtest \
      --days 7200 \
      --start-seed 1 \
      --seeds 2 \
      --persona opportunist \
      --trace-limit 20 \
      --minimum-overall 75 \
      --fail-on-critical \
      --json \
      --output target/gameplay-credit-stress.json
  run_step 'Credit stress validation' "$python_command" scripts/check_gameplay.py credit-stress \
    target/gameplay-credit-stress.json
  run_step 'Deep gameplay fantasy validation' "$python_command" scripts/check_gameplay.py generation-matrix \
    target/gameplay-generation-matrix.json
}

run_ci_verify() {
  run_shell_syntax
  run_step 'Format' cargo fmt --all -- --check
  run_step 'Compile and lint' cargo clippy --all-targets --all-features --locked -- -D warnings
  run_fast
  run_docs
  run_step 'Documentation warnings' env RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked
  run_step 'Whitespace errors' git diff-tree --check --no-commit-id --root -r -m HEAD
}

run_ci_gates() {
  run_soak
  run_step 'Release library tests' cargo test --release --quiet --locked "${job_args[@]}" --lib
  ensure_cli_binary release
  run_cli_core
  run_cli_art
  run_cli_gameplay
  run_gameplay
  run_step 'Dependency audit' cargo audit
}

run_slow_gates() {
  run_soak
  run_step 'Release library tests' cargo test --release --quiet --locked "${job_args[@]}" --lib
  ensure_cli_binary release
  run_cli_core
  run_cli_art
  run_cli_gameplay
  run_gameplay
  run_generation_gameplay
}

run_all() {
  run_standard
  run_soak
  ensure_cli_binary "${CIVIC_DYNASTY_PROFILE:-debug}"
  run_cli_art
  run_cli_gameplay
  run_gameplay
  run_generation_gameplay
}

case "$mode" in
  fast)
    [[ $# -le 2 ]] || usage
    run_fast "${2:-}"
    ;;
  standard)
    [[ $# -eq 1 ]] || usage
    run_standard
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
    run_cli_core
    ;;
  art-cli)
    [[ $# -eq 1 ]] || usage
    run_cli_art
    ;;
  gameplay-cli)
    [[ $# -eq 1 ]] || usage
    run_cli_gameplay
    ;;
  adapters)
    [[ $# -eq 1 ]] || usage
    ensure_cli_binary "${CIVIC_DYNASTY_PROFILE:-debug}"
    run_cli_core
    run_cli_art
    run_cli_gameplay
    ;;
  gameplay)
    [[ $# -eq 1 ]] || usage
    run_gameplay
    run_generation_gameplay
    ;;
  gameplay-audit)
    [[ $# -eq 1 ]] || usage
    run_gameplay_audit
    ;;
  ci-verify)
    [[ $# -eq 1 ]] || usage
    run_ci_verify
    ;;
  ci-gates)
    [[ $# -eq 1 ]] || usage
    run_ci_gates
    ;;
  slow)
    [[ $# -eq 1 ]] || usage
    run_slow_gates
    ;;
  deep)
    [[ $# -eq 1 ]] || usage
    run_slow_gates
    run_gameplay_audit
    ;;
  all)
    [[ $# -eq 1 ]] || usage
    run_all
    ;;
  *)
    usage
    ;;
esac
