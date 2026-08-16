#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

mode=${1:-fast}

usage() {
  cat >&2 <<EOF
usage:
  $0 fast [filter]       run non-ignored library tests
  $0 standard            run the normal pre-commit loop: syntax, library, docs, and core CLI
  $0 exact <test-name>   run one fully qualified library test
  $0 debug <test-name>   run one exact test with captured output disabled
  $0 list [filter]       list matching library tests
  $0 soak                run ignored deterministic soak tests
  $0 docs                run documentation consistency and doctests
  $0 cli                 run core campaign/projection/dashboard CLI smoke tests
  $0 art-cli             run procedural-art CLI smoke tests
  $0 gameplay-cli        run gameplay-harness CLI smoke tests
  $0 adapters            run all CLI smoke groups
  $0 gameplay            run release gameplay and generation-length quality gates
  $0 gameplay-audit      run mature multi-seed, generation, and credit-stress design audits
  $0 all                 run syntax, library, doc, soak, CLI, and gameplay tests
EOF
  exit 2
}

resolve_python() {
  if python3 --version >/dev/null 2>&1; then
    printf 'python3'
  elif python --version >/dev/null 2>&1; then
    printf 'python'
  else
    printf 'Python is required for repository validation\n' >&2
    return 1
  fi
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
  if [[ -n "${CIVIC_DYNASTY_BINARY:-}" ]]; then
    "$CIVIC_DYNASTY_BINARY" playtest "$@"
  else
    cargo run --release --quiet --locked -- playtest "$@"
  fi
}

run_standard() {
  run_shell_syntax
  run_fast
  run_docs
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
    cargo test --quiet --locked --lib '::soak::' -- --ignored
}

run_docs() {
  local python_command
  python_command=$(resolve_python) || return
  run_step 'Documentation consistency' "$python_command" scripts/check_docs.py
  run_step 'Documentation tests' cargo test --quiet --locked --doc
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
  run_step 'Generation-length fantasy validation' "$python_command" -c '
import json
from pathlib import Path

report = json.loads(Path("target/gameplay-generation-gate.json").read_text(encoding="utf-8"))
campaigns = report["campaigns"]
if not campaigns or campaigns[0]["fantasy_arc"]["first_succession_day"] is None:
    raise SystemExit("generation-length gameplay gate did not reach succession")
phase = report["aggregate"]["phase_stats"].get("SuccessionLegacy", {})
if phase.get("decision_cycles", 0) == 0:
    raise SystemExit("generation-length gameplay gate did not observe succession-and-legacy decisions")
'
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
  run_step 'Credit stress validation' "$python_command" -c '
import json
from pathlib import Path

report = json.loads(Path("target/gameplay-credit-stress.json").read_text(encoding="utf-8"))
campaigns = report["campaigns"]
credit_actions = sum(
    campaign["commands"]["ExtendCredit"]["executed"] for campaign in campaigns
)
minimum_credit_sample = len(campaigns) * 2
if credit_actions < minimum_credit_sample:
    raise SystemExit(
        f"credit stress audit observed only {credit_actions} player loans; "
        f"requires at least {minimum_credit_sample}"
    )
distressed = [
    campaign
    for campaign in campaigns
    if campaign["maximum_player_delinquent_lending"] > 0
    or campaign["maximum_player_defaulted_lending"] > 0
]
if not distressed:
    raise SystemExit("credit stress audit observed no distress on player-issued loans")
enforcement_cases = sum(campaign["player_debt_enforcement_cases"] for campaign in campaigns)
if enforcement_cases == 0:
    raise SystemExit("credit stress audit observed player lending distress but no debt enforcement")
'
  run_step 'Deep gameplay fantasy validation' "$python_command" -c '
import json
from pathlib import Path

report = json.loads(Path("target/gameplay-generation-matrix.json").read_text(encoding="utf-8"))
campaigns = report["campaigns"]
expected_personas = {"Steward", "Entrepreneur", "PowerBroker", "Opportunist"}
observed_personas = {campaign["persona"] for campaign in campaigns}
if observed_personas != expected_personas:
    raise SystemExit(f"generation audit personas differ: {sorted(observed_personas)}")
missing = [campaign["persona"] for campaign in campaigns if campaign["fantasy_arc"]["first_succession_day"] is None]
if missing:
    raise SystemExit(f"generation audit did not reach succession for: {missing}")
missing_transitions = [
    campaign["persona"]
    for campaign in campaigns
    if campaign.get("succession_transition") is None
]
if missing_transitions:
    raise SystemExit(
        f"generation audit did not capture succession transitions for: {missing_transitions}"
    )
for campaign in campaigns:
    transition = campaign["succession_transition"]
    succession_day = campaign["fantasy_arc"]["first_succession_day"]
    if transition["day"] != succession_day:
        persona = campaign["persona"]
        transition_day = transition["day"]
        raise SystemExit(
            f"{persona} succession transition day {transition_day} "
            f"did not match fantasy milestone day {succession_day}"
        )
stranded = [
    finding
    for finding in report["findings"]
    if finding["title"] == "Political succession can strand institutional recovery"
]
if stranded:
    raise SystemExit(stranded[0]["evidence"])
phase = report["aggregate"]["phase_stats"].get("SuccessionLegacy", {})
if phase.get("decision_cycles", 0) == 0 or phase.get("substantive_actions", 0) == 0:
    raise SystemExit("generation audit did not observe substantive succession-and-legacy play")
'
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
  all)
    [[ $# -eq 1 ]] || usage
    run_standard
    run_soak
    run_cli_art
    run_cli_gameplay
    run_gameplay
    run_generation_gameplay
    ;;
  *)
    usage
    ;;
esac
