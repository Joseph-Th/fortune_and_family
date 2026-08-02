#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

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
  "$@" > "$output" || fail "$description command failed"
}

cargo build --quiet --locked --bin civic-dynasty || fail 'CLI binary build failed'

binary="target/debug/civic-dynasty"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/civic-dynasty-cli.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT

campaign="$work_dir/campaign.json"
summary="$work_dir/summary.json"
projection="$work_dir/projection.json"
dashboard="$work_dir/campaign.html"

if python3 --version > /dev/null 2>&1; then
  python_command=python3
elif python --version > /dev/null 2>&1; then
  python_command=python
else
  fail 'Python is required to validate CLI JSON output'
fi

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
  --command '{"EnactLaw":{"kind":"BreadPriceCeiling","value":30}}'
run_to_file 'JSON summary' "$summary" "$binary" summary "$campaign" --json
run_to_file 'campaign projection' "$projection" "$binary" inspect "$campaign"
run_to_file 'dashboard rendering' "$work_dir/dashboard.txt" \
  "$binary" dashboard "$campaign" --output "$dashboard"
run_to_file 'save validation' "$work_dir/validate.txt" "$binary" validate "$campaign"

require_nonempty_file "$campaign" 'campaign save'
require_nonempty_file "$dashboard" 'HTML dashboard'
require_literal 'Enacted law' "$work_dir/execute.txt" 'command result'
require_literal 'Validated ' "$work_dir/validate.txt" 'validation result'
grep -Fiq '<!doctype html>' "$dashboard" || fail 'dashboard is not a complete HTML document'

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
    "dynasties",
    "districts",
    "market",
    "contracts",
    "institutions",
    "notifications",
}
missing_projection = sorted(required_projection - projection.keys())
if missing_projection:
    raise SystemExit(f"projection JSON missing views: {', '.join(missing_projection)}")
if not projection["districts"] or not projection["market"]:
    raise SystemExit("projection must contain district and market views")
if projection["scenario"]["elapsed_days"] != 60:
    raise SystemExit("projection did not preserve both simulation advances")
active_ceiling = [
    law
    for law in projection.get("laws", [])
    if law.get("active")
    and law.get("kind") == "BreadPriceCeiling"
    and law.get("value") == 30
]
if len(active_ceiling) != 1:
    raise SystemExit("projection must contain the enacted bread price ceiling")
PY

if "$binary" new \
  --output "$work_dir/invalid.json" \
  --dynasty '   ' \
  > "$work_dir/invalid-stdout.txt" \
  2> "$work_dir/invalid-stderr.txt"
then
  fail 'empty dynasty name unexpectedly succeeded'
fi

require_literal 'dynasty name must not be empty' "$work_dir/invalid-stderr.txt" 'invalid-input error'
test ! -e "$work_dir/invalid.json" || fail 'invalid campaign command created a save file'

printf 'CLI smoke verification passed.\n'
