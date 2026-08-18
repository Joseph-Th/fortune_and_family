#!/usr/bin/env bash
# Point this repository at the version-controlled local hooks in scripts/hooks.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
git config core.hooksPath scripts/hooks
printf 'Local git hooks installed: %s\n' "$(git config core.hooksPath)"
