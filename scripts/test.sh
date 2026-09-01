#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

lane_start=$SECONDS
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
  $0 check [filter]      fastest syntax check (cargo check, ~0.3s warm, no tests)
  $0 fast [filter]       library tests — default loop (~2s full, <1s filtered, incremental)
  $0 quick [filter]      alias for fast (never triggers docs/CLI)
  $0 changed             auto-targeted loop: only domains touched by current diff (<1s)
  $0 standard            pre-commit loop: syntax + lib + docs + core CLI (~4s warm)
  $0 exact <test-name>   one fully-qualified test (with --exact --include-ignored)
  $0 debug <test-name>   one test with --nocapture output
  $0 list [filter]       list matching library tests
  $0 soak                long-horizon deterministic soaks (release, ~1s warm)
  $0 docs                doc consistency + doctests (~1s warm)
  $0 cli                 core CLI smoke (campaign/projection/dashboard)
  $0 art-cli             art CLI smoke   |  $0 gameplay-cli   harness CLI smoke
  $0 adapters            all CLI groups (one CLI build, ~2s warm)
  $0 playtest [args...]  focused harness: no args = 60-day debug check (<1s warm)
  $0 gameplay            release quality gates: 36+3 campaigns, 60k days (~16s warm)
  $0 gameplay-audit      deep design audit: multi-seed / generation / credit stress (~30s)
  $0 ci-verify|ci        fast CI: format + clippy + lib + docs (~5s warm)
  $0 ci-gates            deep CI: release lib + soaks + adapters + gameplay + audit
  $0 all                 standard + soak + adapters + gameplay (~25s warm)
  $0 slow                release gates without audit  |  $0 deep  slow + audit (~1.2m)

inner loop — solo dev, every lane is incremental (no world rebuild):
  $0 fast simulation               # 82 tests, 0.12s exec, <1s total warm
  $0 fast strategic                # 197 tests, 0.30s exec
  $0 changed                       # auto-detects touched domain, <1s warm
  $0 check                         # syntax only, cargo check shares dev cache
  CIVIC_DYNASTY_SKIP_CLI_BUILD=1 $0 standard  # lib-only, skips debug CLI
  CIVIC_DYNASTY_SKIP_DOCS=1 $0 standard       # skip docs (~3s)
  $0 debug 'systems::simulation::tests::household_fallback'  # one test --nocapture
  $0 playtest                      # 60-day single-persona harness smoke, debug <1s
  $0 playtest --days 90 --persona steward --background baker
  CIVIC_DYNASTY_PROFILE=release $0 playtest --days 360 --persona entrepreneur  # gate fidelity
  $0 gameplay                      # full release gate, only when needed (one release build)

warm budgets (incremental, after one-time cold: clippy ~11s, release ~56s):
  check <1s | fast-filter <1s | fast ~2s (980 tests, 1.7s exec) | standard ~4s
  docs ~1s | adapters ~2s | playtest <1s (debug) | ci-verify ~5s | gameplay ~16s | all ~25s | deep ~1.2m

knobs (all optional):
  CIVIC_DYNASTY_JOBS=4             cap cargo --jobs + harness parallelism
  CIVIC_DYNASTY_PROFILE=release    force release CLI for adapters/playtest
  CIVIC_DYNASTY_BINARY=/path/to/civic-dynasty  reuse prebuilt CLI
  CIVIC_DYNASTY_SKIP_CLI_BUILD=1   skip CLI rebuild (lib-only iteration)
  CIVIC_DYNASTY_SKIP_DOCS=1        skip docs in standard
  CIVIC_DYNASTY_NEXTEST=1          run lib tests under cargo-nextest (isolated)
  CIVIC_DYNASTY_PYTHON=python3     select Python interpreter
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
  if [[ -n "${CIVIC_DYNASTY_SKIP_CLI_BUILD:-}" ]]; then
    return 0
  fi
  local requested_profile=${1:-debug}
  if [[ -n "${CIVIC_DYNASTY_BINARY_OVERRIDE:-}" ]]; then
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

  local build_log
  local build_started=$SECONDS
  build_log=$(mktemp)
  printf '\n==> Build %s CLI once\n' "$requested_profile"
  if ! cargo build --quiet --locked "${job_args[@]}" "${profile_args[@]}" --bin civic-dynasty >"$build_log" 2>&1; then
    cat "$build_log" >&2
    rm -f "$build_log"
    printf '<== Build %s CLI once FAILED in %s\n' "$requested_profile" "$(format_duration "$((SECONDS - build_started))")" >&2
    return 1
  fi
  rm -f "$build_log"
  printf '<== Build %s CLI once passed in %s\n' "$requested_profile" "$(format_duration "$((SECONDS - build_started))")"

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

# playtest is the focused harness entry for iteration. By default it uses the
# debug CLI so a warm edit→playtest roundtrip is <1s. Release is enforced only
# by explicit profile selection or by gate lanes (gameplay, ci-gates, etc.)
run_playtest() {
  local profile=${CIVIC_DYNASTY_PROFILE:-debug}
  # Allow explicit override: CIVIC_DYNASTY_PROFILE=release makes playtest
  # gate-faithful. Callers who set CIVIC_DYNASTY_BINARY directly already
  # control the binary via ensure_cli_binary's release guard.
  ensure_cli_binary "$profile"
  "$CIVIC_DYNASTY_BINARY" playtest "$@"
}

# release_playtest is used by gate lanes that must not inherit a debug CLI.
run_release_playtest() {
  ensure_cli_binary release
  "$CIVIC_DYNASTY_BINARY" playtest "$@"
}

run_standard() {
  run_shell_syntax
  run_fast
  if [[ -z "${CIVIC_DYNASTY_SKIP_DOCS:-}" ]]; then
    run_docs
  else
    printf '\n==> Documentation checks (skipped via CIVIC_DYNASTY_SKIP_DOCS)\n'
  fi
  if [[ -z "${CIVIC_DYNASTY_SKIP_CLI_BUILD:-}" ]]; then
    ensure_cli_binary "${CIVIC_DYNASTY_PROFILE:-debug}"
    run_cli_core
  else
    printf '\n==> Core CLI smoke tests (skipped via CIVIC_DYNASTY_SKIP_CLI_BUILD)\n'
  fi
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

has_nextest() {
  command -v cargo-nextest >/dev/null 2>&1 || cargo nextest --version >/dev/null 2>&1
}

run_check() {
  local filter=${1:-}
  local label='Syntax check (cargo check)'
  if [[ -n "$filter" ]]; then
    label="Syntax check matching '$filter'"
  fi
  # cargo check is the fastest syntax gate (warm incremental after the first
  # build). --all-targets would also typecheck benches/examples; the routine
  # gate only needs the real shipped artifacts (lib + bins) to stay fast, so
  # check them here while `cargo test`/`cargo clippy` separately verify tests.
  local command=(cargo check --quiet --locked "${job_args[@]}" --lib --bins)
  if [[ -n "$filter" ]]; then
    # cargo check does not filter by test name, but we can still validate that
    # the filter would match at least one test to give precise feedback.
    local matches
    matches=$(matching_tests "$filter") || true
    if [[ -z "$matches" ]]; then
      printf '\n==> %s\n' "$label"
      printf 'no library tests matched %q\n' "$filter" >&2
      printf '<== %s FAILED in <1s\n' "$label" >&2
      return 2
    fi
  fi
  run_step "$label" "${command[@]}"
}

run_fast() {
  local filter=${1:-}
  local label='Library tests'
  if [[ -n "$filter" ]]; then
    label="Library tests matching '$filter'"
  fi
  if has_nextest && [[ -n "${CIVIC_DYNASTY_NEXTEST:-}" ]]; then
    local command=(cargo nextest run --locked --lib "${job_args[@]}" --no-fail-fast)
    if [[ -n "$filter" ]]; then
      command+=(-E "test($filter)")
    fi
    run_step "$label (nextest)" "${command[@]}"
    return $?
  fi
  local command=(cargo test --quiet --locked --lib "${job_args[@]}")
  if [[ -n "$filter" ]]; then
    command+=("$filter")
  fi
  run_test_step "$label" "${filter:+$filter}" "${command[@]}"
}

run_changed() {
  # Solo-dev targeted loop: diff against HEAD and map changed files to the
  # narrowest `fast <filter>` that proves the change. Untracked non-ignored
  # files are included so a new module without a commit is still detected.
  # 1 domain  -> `fast <filter>` (<1s, ~80-200 tests)
  # 2-3 domains -> one cargo build with OR filters via test binary `--` (still <1s)
  # docs-only -> docs lane (~1s) instead of lib suite
  # cross-cutting/unknown -> full lib suite so coverage never narrows silently.
  local changed_files
  changed_files=$( (
    git diff --name-only HEAD 2>/dev/null || true
    git ls-files --others --exclude-standard 2>/dev/null || true
  ) | sort -u )
  if [[ -z "$changed_files" ]]; then
    printf '\n==> Changed domains: no tracked diff — running full library suite\n'
    run_fast
    return $?
  fi
  local filters=()
  local docs_only=1
  local seen=() # deduplicate while preserving first-seen order
  add_filter() {
    local candidate=$1
    for existing in "${seen[@]}"; do
      [[ "$existing" == "$candidate" ]] && return 0
    done
    seen+=("$candidate")
    filters+=("$candidate")
  }
  while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    case "$file" in
      src/systems/simulation/*) add_filter simulation; docs_only=0 ;;
      src/systems/strategic/*) add_filter strategic; docs_only=0 ;;
      src/systems/commands/*) add_filter commands; docs_only=0 ;;
      src/systems/bootstrap*|src/systems/invariants*|src/systems/legal*|src/systems/progression*|src/systems/transactions*|src/systems/mod.rs) add_filter bootstrap; docs_only=0 ;;
      src/core/*|src/ids.rs|src/money.rs|src/rng.rs) add_filter core; docs_only=0 ;;
      src/persistence*|src/projection*|src/registry/*) add_filter persistence; docs_only=0 ;;
      src/gameplay/*|src/gameplay_tests.rs) add_filter gameplay; docs_only=0 ;;
      src/art/*) add_filter art; docs_only=0 ;;
      src/main.rs|src/lib.rs) add_filter ""; docs_only=0 ;; # top-level -> full suite
      *.md|docs/*) : ;; # docs-only, handled via docs_only flag
      scripts/*) add_filter ""; docs_only=0 ;; # script changes could affect any lane
      *) : ;;
    esac
  done <<< "$changed_files"
  # Docs-only edits should not pay for the 980-test lib suite.
  if [[ ${#filters[@]} -eq 0 && $docs_only -eq 1 ]]; then
    printf '\n==> Changed files are docs-only — running documentation checks\n'
    run_docs
    return $?
  fi
  # Any empty filter means "unknown scope" -> full suite.
  for f in "${filters[@]}"; do
    if [[ -z "$f" ]]; then
      printf '\n==> Changed domains span multiple systems (%s) — running full library suite\n' "${changed_files//$'\n'/, }"
      run_fast
      return $?
    fi
  done
  if [[ ${#filters[@]} -eq 0 ]]; then
    printf '\n==> Changed files do not map to a library domain — running full library suite\n'
    run_fast
    return $?
  fi
  if [[ ${#filters[@]} -eq 1 ]]; then
    printf '\n==> Changed domain: %s — running filtered suite\n' "${filters[0]}"
    run_fast "${filters[0]}"
    return $?
  fi
  if [[ ${#filters[@]} -ge 2 && ${#filters[@]} -le 3 ]]; then
    printf '\n==> Changed domains: %s — running targeted multi-filter suite\n' "${filters[*]}"
    local label="Library tests matching '${filters[*]}'"
    if has_nextest && [[ -n "${CIVIC_DYNASTY_NEXTEST:-}" ]]; then
      local expr=""
      for f in "${filters[@]}"; do
        if [[ -z "$expr" ]]; then
          expr="test($f)"
        else
          expr="$expr | test($f)"
        fi
      done
      run_step "$label (nextest)" cargo nextest run --locked --lib "${job_args[@]}" --no-fail-fast -E "$expr"
      return $?
    fi
    run_test_step "$label" "${filters[*]}" cargo test --quiet --locked --lib "${job_args[@]}" -- "${filters[@]}"
    return $?
  fi
  printf '\n==> Changed domains: %s — running full library suite\n' "${filters[*]}"
  run_fast
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
  # than the debug test profile with the same deterministic assertions. Warm
  # incremental rebuild is ~1s; cold is ~50s (one-time). Filter matches the
  # canonical `::soak::` module so a stale `ignored` marker cannot hide drift.
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
  run_step 'Gameplay quality gate (36 campaigns)' \
    run_release_playtest \
      --minimum-overall 75 \
      --fail-on-critical \
      --json \
      --output target/gameplay-quality-gate.json
}

run_generation_gameplay() {
  local python_command
  python_command=$(resolve_python) || return
  run_step 'Generation-length gameplay gate (3 campaigns, 7200 days)' \
    run_release_playtest \
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
  run_step 'Mature multi-seed gameplay audit (2 seeds, 3600 days)' \
    run_release_playtest \
      --days 3600 \
      --start-seed 1 \
      --seeds 2 \
      --trace-limit 30 \
      --minimum-overall 75 \
      --fail-on-critical \
      --json \
      --output target/gameplay-deep-audit.json
  run_step 'Generation-length persona audit (4 personas, 7200 days)' \
    run_release_playtest \
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
  run_step 'Opportunist credit stress audit (2 seeds, 7200 days)' \
    run_release_playtest \
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
  # Primary gate first so a failing lib test reports in ~2s warm before
  # paying for clippy/doc rebuilds; warm clippy/doc are <1s each.
  run_fast
  run_docs
  run_step 'Compile and lint' cargo clippy --all-targets --all-features --locked -- -D warnings
  run_step 'Documentation warnings' env RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked
  # Whitespace check is syntax-only and fast; keep it last so it never
  # obscures a lib/docs failure.
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
  if command -v cargo-audit >/dev/null 2>&1 || cargo audit --version >/dev/null 2>&1; then
    run_step 'Dependency audit' cargo audit
  else
    printf '\n==> Dependency audit (skipped: cargo-audit not installed; run `cargo install cargo-audit`)\n'
  fi
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

# Full unfiltered routine lanes may produce a validation receipt.
receipt_lane=
case "$mode" in
  fast|quick)
    [[ $# -eq 1 ]] && receipt_lane=quick
    ;;
  standard|all)
    receipt_lane=standard
    ;;
esac
receipt_python=
receipt_start=
if [[ -n "$receipt_lane" ]]; then
  receipt_python=$(resolve_python)
  receipt_start=$("$receipt_python" scripts/validation_receipt.py fingerprint)
fi

case "$mode" in
  check)
    [[ $# -le 2 ]] || usage
    run_check "${2:-}"
    ;;
  fast)
    [[ $# -le 2 ]] || usage
    run_fast "${2:-}"
    ;;
  quick)
    [[ $# -le 2 ]] || usage
    run_fast "${2:-}"
    ;;
  changed)
    [[ $# -eq 1 ]] || usage
    run_changed
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
    ensure_cli_binary "${CIVIC_DYNASTY_PROFILE:-debug}"
    run_cli_core
    ;;
  art-cli)
    [[ $# -eq 1 ]] || usage
    ensure_cli_binary "${CIVIC_DYNASTY_PROFILE:-debug}"
    run_cli_art
    ;;
  gameplay-cli)
    [[ $# -eq 1 ]] || usage
    ensure_cli_binary "${CIVIC_DYNASTY_PROFILE:-debug}"
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
  playtest)
    shift
    if [[ $# -eq 0 ]]; then
      run_step 'Gameplay harness run (60-day debug smoke, <1s warm)' run_playtest --days 60 --seeds 1 --persona steward --background baker --trace-limit 8
    else
      run_step 'Gameplay harness run' run_playtest "$@"
    fi
    ;;
  gameplay-audit)
    [[ $# -eq 1 ]] || usage
    run_gameplay_audit
    ;;
  ci-verify)
    [[ $# -eq 1 ]] || usage
    run_ci_verify
    ;;
  ci)
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
  help|--help|-h)
    usage
    ;;
  *)
    usage
    ;;
esac

if [[ -n "$receipt_lane" ]]; then
  "$receipt_python" scripts/validation_receipt.py record "$receipt_lane" "$receipt_start"
fi

printf '\n==> Lane %q completed in %s\n' "$mode" "$(format_duration "$((SECONDS - lane_start))")"
