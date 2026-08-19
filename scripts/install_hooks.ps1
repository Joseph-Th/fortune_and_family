# Point this repository at the version-controlled local hooks in scripts/hooks.
$ErrorActionPreference = "Stop"

$repoRoot = (git rev-parse --show-toplevel).Trim()
Set-Location $repoRoot
git config core.hooksPath scripts/hooks
Write-Host "Local git hooks installed: $(git config core.hooksPath)" -ForegroundColor Green
