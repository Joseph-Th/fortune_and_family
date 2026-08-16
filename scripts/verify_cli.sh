#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

mode=${1:-core}

usage() {
  cat >&2 <<EOF
usage: $0 [core|art|gameplay|all]

  core      exercise campaign, projection, dashboard, and validation commands
  art       exercise sprite-review HTML/JSON output and argument rejection
  gameplay  exercise focused playtest output and quality-gate failure
  all       run every CLI smoke group
EOF
  exit 2
}

fail() {
  printf 'CLI smoke failure: %s\n' "$1" >&2
  exit 1
}

require_nonempty_file() {
  local path=$1
  local description=$2
  test -s "$path" || fail "$description was not created or is empty"
}

require_literal() {
  local needle=$1
  local path=$2
  local description=$3
  grep -Fq "$needle" "$path" || fail "$description did not contain '$needle'"
}

run_to_file() {
  local description=$1
  local output=$2
  shift 2
  "$@" >"$output" || fail "$description command failed"
}

resolve_python() {
  if python3 --version >/dev/null 2>&1; then
    printf 'python3'
  elif python --version >/dev/null 2>&1; then
    printf 'python'
  else
    fail 'Python is required to validate CLI structured output'
  fi
}

case "$mode" in
  core|art|gameplay|all) ;;
  *) usage ;;
esac

profile=${CIVIC_DYNASTY_PROFILE:-debug}
case "$profile" in
  debug)
    cargo_profile_args=()
    ;;
  release)
    cargo_profile_args=(--release)
    ;;
  *)
    fail "unsupported CIVIC_DYNASTY_PROFILE '$profile' (expected debug or release)"
    ;;
esac

binary=${CIVIC_DYNASTY_BINARY:-target/$profile/civic-dynasty}
if [[ -z "${CIVIC_DYNASTY_BINARY:-}" ]]; then
  cargo build --quiet --locked "${cargo_profile_args[@]}" --bin civic-dynasty \
    || fail "CLI $profile binary build failed"
fi
if [[ ! -x "$binary" && -x "${binary}.exe" ]]; then
  binary="${binary}.exe"
fi
test -x "$binary" || fail "CLI binary '$binary' was not found"

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/civic-dynasty-cli.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT
python_command=$(resolve_python)

run_core_smoke() {
  local campaign="$work_dir/campaign.json"
  local summary="$work_dir/summary.json"
  local projection="$work_dir/projection.json"
  local dashboard="$work_dir/exports/dashboard/campaign.html"

  run_to_file 'campaign creation' "$work_dir/new.txt" "$binary" new \
    --output "$campaign" \
    --seed 42 \
    --dynasty Valeri \
    --founder "Elian Valeri" \
    --background baker \
    --advance 30
  run_to_file 'campaign simulation' "$work_dir/simulate.txt" \
    "$binary" simulate "$campaign" --days 30
  run_to_file 'player command' "$work_dir/execute.txt" "$binary" execute "$campaign" \
    --command '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}'
  run_to_file 'JSON summary' "$summary" "$binary" summary "$campaign" --json
  run_to_file 'campaign projection' "$projection" "$binary" inspect "$campaign"
  run_to_file 'dashboard rendering' "$work_dir/dashboard.txt" \
    "$binary" dashboard "$campaign" --output "$dashboard"
  run_to_file 'save validation' "$work_dir/validate.txt" "$binary" validate "$campaign"

  require_nonempty_file "$campaign" 'campaign save'
  require_nonempty_file "$dashboard" 'HTML dashboard'
  grep -Fiq '<!doctype html>' "$dashboard" || fail 'dashboard is not a complete HTML document'
  require_literal 'id="campaign-data"' "$dashboard" 'dashboard embedded projection'

  "$python_command" - "$summary" "$projection" <<'PY'
import json
import sys
from pathlib import Path

summary_path, projection_path = map(Path, sys.argv[1:])
summary = json.loads(summary_path.read_text(encoding="utf-8"))
projection = json.loads(projection_path.read_text(encoding="utf-8"))

required_summary = {
    "scenario_name",
    "elapsed_days",
    "dynasty_name",
    "businesses",
    "population_groups",
    "outstanding_civic_debts",
    "civic_debt_balance",
}
missing_summary = sorted(required_summary - summary.keys())
if missing_summary:
    raise SystemExit(f"summary JSON missing fields: {', '.join(missing_summary)}")
if summary["elapsed_days"] != 60:
    raise SystemExit(f"summary elapsed_days was {summary['elapsed_days']}, expected 60")
if summary["businesses"] <= 0 or summary["population_groups"] <= 0:
    raise SystemExit("summary must report businesses and population groups")

required_projection = {
    "scenario",
    "player",
    "family",
    "dynasties",
    "districts",
    "businesses",
    "employment",
    "market",
    "contracts",
    "loans",
    "properties",
    "civic_debts",
    "institutions",
    "legal_cases",
    "information",
    "notifications",
}
missing_projection = sorted(required_projection - projection.keys())
if missing_projection:
    raise SystemExit(f"projection JSON missing views: {', '.join(missing_projection)}")
if not projection["districts"] or not projection["businesses"] or not projection["market"]:
    raise SystemExit("projection must contain district, business, and market views")
if projection["scenario"]["elapsed_days"] != 60:
    raise SystemExit("projection did not preserve both simulation advances")
if projection["family"]["governance"] != "FamilyPartnership":
    raise SystemExit(
        "projection did not expose the governance change committed through the execute command"
    )
PY

  if "$binary" new \
    --output "$work_dir/invalid.json" \
    --dynasty '   ' \
    >"$work_dir/invalid-stdout.txt" \
    2>"$work_dir/invalid-stderr.txt"
  then
    fail 'empty dynasty name unexpectedly succeeded'
  fi
  require_nonempty_file "$work_dir/invalid-stderr.txt" 'invalid-input error output'
  test ! -e "$work_dir/invalid.json" || fail 'invalid campaign command created a save file'
}

run_art_smoke() {
  local art_sheet="$work_dir/sprite-review.html"
  local art_report="$work_dir/sprite-review.json"

  run_to_file 'sprite review sheet' "$work_dir/art.txt" "$binary" art \
    --output "$art_sheet" \
    --role baker \
    --seeds 1 \
    --scale 4 \
    --fail-on-critical
  run_to_file 'sprite review report' "$work_dir/art-json.txt" "$binary" art \
    --output "$art_report" \
    --role merchant \
    --seeds 1 \
    --json

  local invalid_art="$work_dir/invalid-art.html"
  if "$binary" art --output "$invalid_art" --seeds 0 >"$work_dir/invalid-art.txt" 2>&1; then
    fail 'sprite review accepted an invalid zero seed count'
  fi
  test ! -e "$invalid_art" || fail 'invalid sprite review created an output file'

  require_nonempty_file "$art_sheet" 'sprite review sheet'
  require_nonempty_file "$art_report" 'sprite review report'
  grep -Fiq '<!doctype html>' "$art_sheet" || fail 'sprite review sheet is not a complete HTML document'
  grep -Fq 'data:image/png;base64,' "$art_sheet" || fail 'sprite review sheet does not embed sprite images'

  "$python_command" - "$art_report" <<'PY'
import json
import sys
from pathlib import Path

art = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
if art["schema_version"] < 1:
    raise SystemExit("art review report must carry a schema version")
if len(art["subjects"]) != 1:
    raise SystemExit("focused art review must render exactly one subject")
clips = art["subjects"][0]["clips"]
expected_clips = {"idle", "walk", "work", "carry"}
if len(clips) != len(set(clips)) or set(clips) != expected_clips:
    raise SystemExit(
        f"art review clips differ; expected {sorted(expected_clips)}, observed {clips}"
    )
if art["critical_findings"] != 0:
    raise SystemExit("art review must not report critical findings")
PY
}

run_gameplay_smoke() {
  local playtest="$work_dir/exports/playtest/playtest.json"

  "$binary" playtest \
    --days 30 \
    --persona steward \
    --background baker \
    --trace-limit 3 \
    --json \
    --output "$playtest" \
    >"$work_dir/playtest.txt" || fail 'gameplay harness command failed'
  require_nonempty_file "$playtest" 'gameplay harness JSON report'

  "$python_command" - "$playtest" <<'PY'
import json
import sys
from pathlib import Path

playtest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
required = {
    "schema_version",
    "config",
    "aggregate",
    "persona_aggregates",
    "campaigns",
    "findings",
}
missing = sorted(required - playtest.keys())
if missing:
    raise SystemExit(f"playtest JSON missing fields: {', '.join(missing)}")
if playtest["aggregate"]["campaigns"] != 1:
    raise SystemExit("focused playtest must run exactly one campaign")
if playtest["aggregate"]["simulated_days"] != 30:
    raise SystemExit("focused playtest must simulate 30 days")
if playtest["aggregate"]["successful_actions"] <= 0:
    raise SystemExit("focused playtest must execute player actions")
if len(playtest["persona_aggregates"]) != 1:
    raise SystemExit("focused playtest must expose one persona aggregate")
if not playtest["campaigns"] or not playtest["campaigns"][0]["trace"]:
    raise SystemExit("focused playtest must contain a reproducible trace")
PY

  if "$binary" playtest \
    --days 7 \
    --persona steward \
    --background baker \
    --trace-limit 1 \
    --minimum-overall 100 \
    --output "$work_dir/gated-playtest.txt" \
    >"$work_dir/gated-playtest-stdout.txt" \
    2>"$work_dir/gated-playtest-stderr.txt"
  then
    fail 'gameplay quality gate unexpectedly succeeded'
  fi
  require_nonempty_file "$work_dir/gated-playtest.txt" 'gated gameplay report'
  require_nonempty_file "$work_dir/gated-playtest-stderr.txt" 'gameplay quality gate error output'
}

case "$mode" in
  core)
    run_core_smoke
    ;;
  art)
    run_art_smoke
    ;;
  gameplay)
    run_gameplay_smoke
    ;;
  all)
    run_core_smoke
    run_art_smoke
    run_gameplay_smoke
    ;;
esac

printf 'CLI %s smoke verification passed.\n' "$mode"
