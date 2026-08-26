# Native PowerShell CLI smoke verifier for Civic Dynasty
param(
    [Parameter(Position=0)]
    [ValidateSet("core", "art", "gameplay", "all")]
    [string]$Mode = "core"
)

$ErrorActionPreference = "Stop"

$ProjectRoot = (Resolve-Path "$PSScriptRoot\..").Path
Set-Location $ProjectRoot

function Fail([string]$Message, [string]$StdErrPath = $null) {
    if ($StdErrPath -and (Test-Path $StdErrPath)) {
        $errText = (Get-Content -Raw -Encoding utf8 $StdErrPath).Trim()
        if ($errText) {
            Write-Host $errText -ForegroundColor Yellow
        }
    }
    Write-Error "CLI smoke failure: $Message"
    exit 1
}

function Require-NonEmptyFile([string]$FilePath, [string]$Description) {
    if (-not (Test-Path $FilePath) -or (Get-Item $FilePath).Length -eq 0) {
        Fail "$Description was not created or is empty"
    }
}

function Require-Literal([string]$Needle, [string]$FilePath, [string]$Description, [switch]$IgnoreCase) {
    $content = Get-Content -Raw -Encoding utf8 $FilePath
    if ($IgnoreCase) {
        if ($content.IndexOf($Needle, [System.StringComparison]::OrdinalIgnoreCase) -lt 0) {
            Fail "$Description did not contain '$Needle'"
        }
    } else {
        if (-not $content.Contains($Needle)) {
            Fail "$Description did not contain '$Needle'"
        }
    }
}

function Resolve-PythonInterpreter {
    if ($env:CIVIC_DYNASTY_PYTHON) {
        if (Get-Command $env:CIVIC_DYNASTY_PYTHON -ErrorAction SilentlyContinue) {
            return $env:CIVIC_DYNASTY_PYTHON
        }
        Fail "CIVIC_DYNASTY_PYTHON is not executable: $($env:CIVIC_DYNASTY_PYTHON)"
    }
    foreach ($cmd in @("python3", "python", "py")) {
        $found = Get-Command $cmd -ErrorAction SilentlyContinue
        if ($found) {
            return $found.Source
        }
    }
    Fail "Python is required to validate CLI structured output; install Python or set CIVIC_DYNASTY_PYTHON"
}

$Profile = if ($env:CIVIC_DYNASTY_PROFILE) { $env:CIVIC_DYNASTY_PROFILE } else { "debug" }
$CargoProfileArgs = @()
if ($Profile -eq "release") {
    $CargoProfileArgs += "--release"
} elseif ($Profile -ne "debug") {
    Fail "unsupported CIVIC_DYNASTY_PROFILE '$Profile' (expected debug or release)"
}

$Binary = if ($env:CIVIC_DYNASTY_BINARY) {
    $env:CIVIC_DYNASTY_BINARY
} else {
    "target\$Profile\civic-dynasty.exe"
}

if (-not (Test-Path $Binary) -and (Test-Path "$Binary.exe")) {
    $Binary = "$Binary.exe"
}

if (-not $env:CIVIC_DYNASTY_BINARY) {
    # Match verify_cli.sh: rebuild whenever no explicit binary was supplied so a
    # direct invocation never exercises a stale CLI. Cargo incrementality keeps
    # an already-current build at no-op cost.
    Write-Host "Building $Profile CLI binary..."
    & cargo build --quiet --locked @CargoProfileArgs --bin civic-dynasty
    if ($LASTEXITCODE -ne 0) {
        Fail "CLI $Profile binary build failed"
    }
}

if (-not (Test-Path $Binary)) {
    Fail "CLI binary '$Binary' was not found"
}

$Binary = (Resolve-Path $Binary).Path
$WorkDir = Join-Path ([System.IO.Path]::GetTempPath()) ("civic-dynasty-cli-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$Python = Resolve-PythonInterpreter

function Format-ArgumentString([string[]]$ArgsList) {
    $formatted = foreach ($arg in $ArgsList) {
        if ($arg -match '[\s"\{\}\[\]]') {
            '"' + $arg.Replace('"', '\"') + '"'
        } else {
            $arg
        }
    }
    return ($formatted -join " ")
}

function Run-ProcessWithCapture([string[]]$CommandArgs, [string]$StdOutPath = $null, [string]$StdErrPath = $null) {
    # System.Diagnostics.Process spawns in tens of milliseconds; Start-Process
    # costs roughly a second per invocation on this machine, which dominated
    # every smoke lane. Argument escaping matches the previous Start-Process
    # contract: Format-ArgumentString output is handed straight to
    # CreateProcess, so quoting behavior is unchanged.
    if (-not $StdOutPath) { $StdOutPath = Join-Path $WorkDir ("stdout-" + [System.Guid]::NewGuid().ToString("N") + ".txt") }
    if (-not $StdErrPath) { $StdErrPath = Join-Path $WorkDir ("stderr-" + [System.Guid]::NewGuid().ToString("N") + ".txt") }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Binary
    $psi.Arguments = Format-ArgumentString $CommandArgs
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $proc = [System.Diagnostics.Process]::Start($psi)
    # Drain both pipes concurrently so a chatty child can never fill a pipe
    # buffer and deadlock before WaitForExit returns.
    $stdoutTask = $proc.StandardOutput.ReadToEndAsync()
    $stderrText = $proc.StandardError.ReadToEnd()
    $stdoutText = $stdoutTask.GetAwaiter().GetResult()
    $proc.WaitForExit()
    # BOM-less UTF-8: the static Encoding.UTF8 instance emits a BOM on .NET
    # Framework, which breaks the JSON parsers downstream.
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($StdOutPath, $stdoutText, $utf8NoBom)
    [System.IO.File]::WriteAllText($StdErrPath, $stderrText, $utf8NoBom)
    return [PSCustomObject]@{
        ExitCode = $proc.ExitCode
        StdOutPath = $StdOutPath
        StdErrPath = $StdErrPath
    }
}

try {
    function Run-CoreSmoke {
        $campaign = Join-Path $WorkDir "campaign.json"
        $summary = Join-Path $WorkDir "summary.json"
        $projection = Join-Path $WorkDir "projection.json"
        $dashboardDir = Join-Path $WorkDir "exports\dashboard"
        New-Item -ItemType Directory -Path $dashboardDir -Force | Out-Null
        $dashboard = Join-Path $dashboardDir "campaign.html"

        $res = Run-ProcessWithCapture @("new", "--output", $campaign, "--seed", "42", "--dynasty", "Valeri", "--founder", "Elian Valeri", "--background", "baker", "--advance", "30")
        if ($res.ExitCode -ne 0) { Fail "campaign creation command failed" $res.StdErrPath }

        $res = Run-ProcessWithCapture @("simulate", $campaign, "--days", "30")
        if ($res.ExitCode -ne 0) { Fail "campaign simulation command failed" $res.StdErrPath }

        $res = Run-ProcessWithCapture @("execute", $campaign, "--command", '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}')
        if ($res.ExitCode -ne 0) { Fail "player command execution failed" $res.StdErrPath }

        $res = Run-ProcessWithCapture @("summary", $campaign, "--json") -StdOutPath $summary
        if ($res.ExitCode -ne 0) { Fail "JSON summary command failed" $res.StdErrPath }

        $res = Run-ProcessWithCapture @("inspect", $campaign) -StdOutPath $projection
        if ($res.ExitCode -ne 0) { Fail "campaign projection command failed" $res.StdErrPath }

        $res = Run-ProcessWithCapture @("dashboard", $campaign, "--output", $dashboard)
        if ($res.ExitCode -ne 0) { Fail "dashboard rendering command failed" $res.StdErrPath }

        $res = Run-ProcessWithCapture @("validate", $campaign)
        if ($res.ExitCode -ne 0) { Fail "save validation command failed" $res.StdErrPath }

        Require-NonEmptyFile $campaign "campaign save"
        Require-NonEmptyFile $dashboard "HTML dashboard"
        Require-Literal "<!doctype html>" $dashboard "dashboard HTML header" -IgnoreCase
        Require-Literal 'id="campaign-data"' $dashboard "dashboard embedded projection"

        $checkCorePy = Join-Path $WorkDir "check_core.py"
        $pyCode = @'
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
    "city_outstanding_civic_debts",
    "city_civic_debt_balance",
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
'@
        [System.IO.File]::WriteAllText($checkCorePy, $pyCode, [System.Text.Encoding]::UTF8)
        & $Python $checkCorePy $summary $projection
        if ($LASTEXITCODE -ne 0) { Fail "summary/projection validation script failed" }

        $invalidOut = Join-Path $WorkDir "invalid.json"
        $res = Run-ProcessWithCapture @("new", "--output", $invalidOut, "--dynasty", "   ")
        if ($res.ExitCode -eq 0) {
            Fail "empty dynasty name unexpectedly succeeded"
        }
        if (Test-Path $invalidOut) {
            Fail "invalid campaign command created a save file"
        }
    }

    function Run-ArtSmoke {
        $artSheet = Join-Path $WorkDir "sprite-review.html"
        $artReport = Join-Path $WorkDir "sprite-review.json"

        $res = Run-ProcessWithCapture @("art", "--output", $artSheet, "--role", "baker", "--seeds", "1", "--scale", "4", "--fail-on-critical")
        if ($res.ExitCode -ne 0) { Fail "sprite review sheet generation failed" $res.StdErrPath }

        $res = Run-ProcessWithCapture @("art", "--output", $artReport, "--role", "merchant", "--seeds", "1", "--json")
        if ($res.ExitCode -ne 0) { Fail "sprite review report generation failed" $res.StdErrPath }

        $invalidArt = Join-Path $WorkDir "invalid-art.html"
        $res = Run-ProcessWithCapture @("art", "--output", $invalidArt, "--seeds", "0")
        if ($res.ExitCode -eq 0) {
            Fail "sprite review accepted an invalid zero seed count"
        }
        if (Test-Path $invalidArt) {
            Fail "invalid sprite review created an output file"
        }

        Require-NonEmptyFile $artSheet "sprite review sheet"
        Require-NonEmptyFile $artReport "sprite review report"
        Require-Literal "<!doctype html>" $artSheet "sprite review HTML document" -IgnoreCase
        Require-Literal "data:image/png;base64," $artSheet "sprite review base64 image data"

        $checkArtPy = Join-Path $WorkDir "check_art.py"
        $pyArtCode = @'
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
'@
        [System.IO.File]::WriteAllText($checkArtPy, $pyArtCode, [System.Text.Encoding]::UTF8)
        & $Python $checkArtPy $artReport
        if ($LASTEXITCODE -ne 0) { Fail "art review report validation script failed" }
    }

    function Run-GameplaySmoke {
        $playtestDir = Join-Path $WorkDir "exports\playtest"
        New-Item -ItemType Directory -Path $playtestDir -Force | Out-Null
        $playtest = Join-Path $playtestDir "playtest.json"

        $res = Run-ProcessWithCapture @("playtest", "--days", "30", "--persona", "steward", "--background", "baker", "--trace-limit", "3", "--json", "--output", $playtest)
        if ($res.ExitCode -ne 0) { Fail "gameplay harness command failed" $res.StdErrPath }
        Require-NonEmptyFile $playtest "gameplay harness JSON report"

        $progress = Get-Content -Raw -Encoding utf8 $res.StdErrPath
        if ($progress -notmatch 'playtest \d+\.\d{3}s \(') {
            Fail "playtest progress line did not report concise timing"
        }

        $checkPlaytestPy = Join-Path $WorkDir "check_playtest.py"
        $pyPlaytestCode = @'
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
'@
        [System.IO.File]::WriteAllText($checkPlaytestPy, $pyPlaytestCode, [System.Text.Encoding]::UTF8)
        & $Python $checkPlaytestPy $playtest
        if ($LASTEXITCODE -ne 0) { Fail "playtest JSON report validation script failed" }

        $gatedOutput = Join-Path $WorkDir "gated-playtest.txt"
        $res = Run-ProcessWithCapture @("playtest", "--days", "7", "--persona", "steward", "--background", "baker", "--trace-limit", "1", "--minimum-overall", "100", "--output", $gatedOutput)
        if ($res.ExitCode -eq 0) {
            Fail "gameplay quality gate unexpectedly succeeded"
        }
        Require-NonEmptyFile $gatedOutput "gated gameplay report"

        $gateReason = Get-Content -Raw -Encoding utf8 $res.StdErrPath
        if ($null -eq $gateReason -or $gateReason -notmatch 'overall score') {
            Fail "gameplay quality gate failure must report the score reason" $res.StdErrPath
        }
    }

    switch ($Mode) {
        "core"     { Run-CoreSmoke }
        "art"      { Run-ArtSmoke }
        "gameplay" { Run-GameplaySmoke }
        "all"      { Run-CoreSmoke; Run-ArtSmoke; Run-GameplaySmoke }
    }

    Write-Host "CLI $Mode smoke verification passed." -ForegroundColor Green
} finally {
    if (Test-Path $WorkDir) {
        Remove-Item -Path $WorkDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
