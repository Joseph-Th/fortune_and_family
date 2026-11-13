#!/usr/bin/env bash
# Real text-play session: create a campaign, commit one player command, advance
# days, read the projection, and render the dashboard. This proves the
# surviving play surface end to end rather than testing it through the library.
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

fail() {
  printf 'text-play session failure: %s\n' "$1" >&2
  exit 1
}

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

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/civic-dynasty-play.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT

campaign="$work_dir/campaign.json"
projection="$work_dir/projection.json"
dashboard="$work_dir/dashboard.html"

"$binary" new \
  --output "$campaign" \
  --seed 7 \
  --dynasty Valeri \
  --founder "Elian Valeri" \
  --background baker \
  >"$work_dir/new.txt" 2>"$work_dir/new-progress.txt" \
  || fail 'campaign creation failed'

# One ordinary player action, committed through the same canonical path the
# library exposes. Governance is chosen because the projection reports it back
# verbatim, so the session can prove the command actually landed.
"$binary" execute "$campaign" \
  --command '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}' \
  >"$work_dir/execute.txt" 2>"$work_dir/execute-progress.txt" \
  || fail 'player command failed'

"$binary" simulate "$campaign" --days 30 \
  >"$work_dir/simulate.txt" 2>"$work_dir/simulate-progress.txt" \
  || fail 'simulation failed'

"$binary" inspect "$campaign" >"$projection" 2>"$work_dir/inspect-progress.txt" \
  || fail 'projection failed'
"$binary" dashboard "$campaign" --output "$dashboard" \
  >"$work_dir/dashboard.txt" 2>"$work_dir/dashboard-progress.txt" \
  || fail 'dashboard rendering failed'

test -s "$campaign" || fail 'campaign save is empty'
test -s "$projection" || fail 'projection is empty'
test -s "$dashboard" || fail 'dashboard is empty'
grep -Fiq '<!doctype html>' "$dashboard" || fail 'dashboard is not a complete HTML document'

python_command=
for candidate in python3 python py; do
  if command -v "$candidate" >/dev/null 2>&1; then
    python_command=$candidate
    break
  fi
done
[[ -n "$python_command" ]] || fail 'Python is required to validate the play session projection'

"$python_command" - "$projection" <<'PY'
import json
import sys
from pathlib import Path

projection = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))

if projection["scenario"]["elapsed_days"] != 30:
    raise SystemExit(
        "play session must advance 30 days after campaign creation, "
        f"observed {projection['scenario']['elapsed_days']}"
    )
if projection["family"]["governance"] != "FamilyPartnership":
    raise SystemExit(
        "play session must persist the governance command, "
        f"observed {projection['family']['governance']}"
    )
if not projection["businesses"] or not projection["market"]:
    raise SystemExit("play session projection must contain business and market views")
PY

printf 'Text-play session passed: created, executed one command, simulated 30 days, inspected projection, rendered dashboard.\n'
