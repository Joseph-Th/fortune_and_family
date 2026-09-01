# Native PowerShell Test and Verification Runner for Civic Dynasty
param(
    [Parameter(Position=0)]
    [string]$Mode = "fast",

    [Parameter(Position=1, ValueFromRemainingArguments = $true)]
    [string[]]$Rest
)

$ErrorActionPreference = "Stop"

# The common modes take at most one filter argument; `playtest` forwards every
# remaining argument to the gameplay-harness CLI verbatim.
$Filter = if ($Rest -and $Rest.Count -gt 0) { $Rest[0] } else { "" }

$ProjectRoot = (Resolve-Path "$PSScriptRoot\..").Path
Set-Location $ProjectRoot

$LaneStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$JobArgs = @()
if ($env:CIVIC_DYNASTY_JOBS) {
    if ($env:CIVIC_DYNASTY_JOBS -notmatch '^[1-9][0-9]*$') {
        Write-Error "CIVIC_DYNASTY_JOBS must be a positive integer, got '$($env:CIVIC_DYNASTY_JOBS)'"
        exit 2
    }
    $JobArgs = @("--jobs", $env:CIVIC_DYNASTY_JOBS)
}

function Show-Usage {
    @"
usage:
  .\scripts\test.ps1 check [filter]      fastest syntax check (cargo check, ~0.3s warm, no tests)
  .\scripts\test.ps1 fast [filter]       library tests — default loop (~2s full, <1s filtered)
  .\scripts\test.ps1 quick [filter]      alias for fast (never triggers docs/CLI)
  .\scripts\test.ps1 changed             auto-targeted: only domains touched by current diff (<1s)
  .\scripts\test.ps1 standard            pre-commit: syntax + lib + docs + core CLI (~4s warm)
  .\scripts\test.ps1 exact <test-name>   one fully-qualified test
  .\scripts\test.ps1 debug <test-name>   one test with --nocapture output
  .\scripts\test.ps1 list [filter]       list matching library tests
  .\scripts\test.ps1 soak                long-horizon soaks (release, ~1s warm)
  .\scripts\test.ps1 docs                doc consistency + doctests (~1s warm)
  .\scripts\test.ps1 cli                 core CLI smoke  |  .\scripts\test.ps1 art-cli       art smoke
  .\scripts\test.ps1 gameplay-cli        harness CLI smoke  |  .\scripts\test.ps1 adapters    all CLI groups (~2s)
  .\scripts\test.ps1 playtest [args...]  focused harness — no args = 60-day debug check (<1s)
  .\scripts\test.ps1 gameplay            release quality gates: 36+3 campaigns, 60k days (~16s)
  .\scripts\test.ps1 gameplay-audit      deep design audit: multi-seed / generation / credit stress (~30s)
  .\scripts\test.ps1 ci-verify|ci        fast CI: format + clippy + lib + docs (~5s warm)
  .\scripts\test.ps1 ci-gates            deep CI: release lib + soaks + adapters + gameplay + audit
  .\scripts\test.ps1 all                 standard + soak + adapters + gameplay (~25s warm)
  .\scripts\test.ps1 slow                release gates without audit  |  .\scripts\test.ps1 deep  slow + audit (~1.2m)

inner loop — solo local, every lane incremental (no world rebuild):
  .\scripts\test.ps1 fast simulation   82 tests, 0.12s exec, <1s total warm
  .\scripts\test.ps1 changed            auto-detects touched domain, <1s warm
  .\scripts\test.ps1 check              syntax only, cargo check shares dev cache
  .\scripts\test.ps1 playtest            60-day single-persona harness smoke, debug <1s
  .\scripts\test.ps1 playtest --days 90 --persona steward  # debug harness <1s

warm budgets (incremental, after first build — cold once: clippy ~11s, release ~56s):
  check <1s  fast-filter <1s  fast ~2s  standard ~4s  ci-verify ~5s  gameplay ~16s  all ~25s  deep ~1.2m

environment:
  CIVIC_DYNASTY_JOBS           pass --jobs N to cargo test/build commands (also caps harness parallelism)
  CIVIC_DYNASTY_PROFILE        debug (default) or release for adapter smoke builds
  CIVIC_DYNASTY_BINARY         reuse a prebuilt CLI binary for adapter smoke groups
  CIVIC_DYNASTY_SKIP_CLI_BUILD skip CLI rebuild when set (fast lib-only iteration)
  CIVIC_DYNASTY_SKIP_DOCS      skip docs in standard (lib-only iteration)
  CIVIC_DYNASTY_PYTHON         select an explicit Python interpreter
"@ | Write-Host
    exit 2
}

function Resolve-BashInterpreter {
    $found = Get-Command bash -ErrorAction SilentlyContinue
    if ($found) {
        return $found.Source
    }
    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($git) {
        $gitDir = Split-Path $git.Source
        foreach ($relative in @("..\usr\bin\bash.exe", "..\bin\bash.exe")) {
            $candidate = [System.IO.Path]::GetFullPath((Join-Path $gitDir $relative))
            if (Test-Path $candidate) {
                return $candidate
            }
        }
    }
    return $null
}

function Run-ShellSyntaxCheck {
    # The git hooks and CI lanes execute the bash runners, so keep them
    # syntactically valid even when driving verification from PowerShell.
    $bash = Resolve-BashInterpreter
    if (-not $bash) {
        Write-Host "`n==> Shell syntax checks (skipped: bash not found)" -ForegroundColor Yellow
        return
    }
    Run-Step "Shell syntax checks" {
        & $bash -n scripts/test.sh scripts/verify_cli.sh
        if ($LASTEXITCODE -ne 0) { throw "Shell syntax check failed" }
    }
}

function Resolve-PythonInterpreter {
    if ($env:CIVIC_DYNASTY_PYTHON) {
        if (Get-Command $env:CIVIC_DYNASTY_PYTHON -ErrorAction SilentlyContinue) {
            return $env:CIVIC_DYNASTY_PYTHON
        }
        Write-Error "CIVIC_DYNASTY_PYTHON is not executable: $($env:CIVIC_DYNASTY_PYTHON)"
        exit 1
    }
    foreach ($cmd in @("python3", "python", "py")) {
        $found = Get-Command $cmd -ErrorAction SilentlyContinue
        if ($found) {
            return $found.Source
        }
    }
    Write-Error "Python is required for repository validation; install Python or set CIVIC_DYNASTY_PYTHON"
    exit 1
}

function Format-Duration([double]$Seconds) {
    if ($Seconds -lt 1.0) {
        return "<1s"
    }
    return "$([Math]::Round($Seconds))s"
}

function Run-Step([string]$Label, [scriptblock]$Action) {
    Write-Host "`n==> $Label" -ForegroundColor Cyan
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    # Native tools (cargo, the CLI) write progress to stderr; under the
    # script-wide Stop preference Windows PowerShell promotes those records
    # to terminating errors even when the command succeeds. Every step checks
    # $LASTEXITCODE explicitly, so relax the preference while the action runs.
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $Action
        $sw.Stop()
        $duration = Format-Duration $sw.Elapsed.TotalSeconds
        Write-Host "<== $Label passed in $duration" -ForegroundColor Green
    } catch {
        $sw.Stop()
        $duration = Format-Duration $sw.Elapsed.TotalSeconds
        Write-Host "<== $Label FAILED in $duration" -ForegroundColor Red
        Write-Error $_
        exit 1
    } finally {
        $ErrorActionPreference = $previousPreference
    }
}

function Ensure-CliBinary([string]$RequestedProfile = "debug") {
    if ($env:CIVIC_DYNASTY_BINARY_OVERRIDE) {
        $binary = $env:CIVIC_DYNASTY_BINARY_OVERRIDE
        if (-not (Test-Path $binary) -and (Test-Path "$binary.exe")) {
            $binary = "$binary.exe"
        }
        if (-not (Test-Path $binary)) {
            Write-Error "CIVIC_DYNASTY_BINARY_OVERRIDE is not executable: $binary"
            exit 1
        }
        $env:CIVIC_DYNASTY_BINARY = $binary
        return
    }

    if ($env:CIVIC_DYNASTY_BINARY) {
        if ($RequestedProfile -ne "release" -or $env:CIVIC_DYNASTY_BINARY -like "*\release\civic-dynasty*" -or $env:CIVIC_DYNASTY_BINARY -like "*\release\civic-dynasty.exe*") {
            $binary = $env:CIVIC_DYNASTY_BINARY
            if (-not (Test-Path $binary) -and (Test-Path "$binary.exe")) {
                $binary = "$binary.exe"
            }
            if (-not (Test-Path $binary)) {
                Write-Error "CIVIC_DYNASTY_BINARY is not executable: $binary"
                exit 1
            }
            $env:CIVIC_DYNASTY_BINARY = $binary
            return
        }
        Write-Warning "CIVIC_DYNASTY_BINARY is not a release binary; rebuilding the release CLI for gameplay gates"
    }

    $profileArgs = @()
    if ($RequestedProfile -eq "release") {
        $profileArgs = @("--release")
    }

    Run-Step "Build $RequestedProfile CLI once" {
        & cargo build --quiet --locked @JobArgs @profileArgs --bin civic-dynasty
        if ($LASTEXITCODE -ne 0) { throw "Cargo build failed" }
    }

    $binary = "target\$RequestedProfile\civic-dynasty.exe"
    if (-not (Test-Path $binary) -and (Test-Path "target\$RequestedProfile\civic-dynasty")) {
        $binary = "target\$RequestedProfile\civic-dynasty"
    }
    if (-not (Test-Path $binary)) {
        Write-Error "CLI binary '$binary' was not produced by the $RequestedProfile build"
        exit 1
    }
    $env:CIVIC_DYNASTY_BINARY = (Resolve-Path $binary).Path
}

function Has-Nextest {
    try {
        & cargo nextest --version 2>$null | Out-Null
        return $true
    } catch { return $false }
}

function Run-Check([string]$TestFilter) {
    $label = if ($TestFilter) { "Syntax check matching '$TestFilter'" } else { "Syntax check (cargo check)" }
    if ($TestFilter) {
        $matches = & cargo test --quiet --locked @JobArgs --lib $TestFilter -- --list 2>$null | Where-Object { $_ -match ': test$' }
        if (-not $matches) {
            Write-Host "`n==> $label" -ForegroundColor Cyan
            Write-Error "no library tests matched '$TestFilter'"
            exit 2
        }
    }
    Run-Step $label {
        & cargo check --quiet --locked @JobArgs --lib --bins
        if ($LASTEXITCODE -ne 0) { throw "Syntax check failed" }
    }
}

function Run-Fast([string]$TestFilter) {
    $label = if ($TestFilter) { "Library tests matching '$TestFilter'" } else { "Library tests" }
    # Measured on this suite: one warm `cargo test` process is the fastest
    # option because tests share campaign-fixture setup. Opt into cargo-nextest
    # with CIVIC_DYNASTY_NEXTEST=1 when per-test isolation helps debugging.
    if ((Has-Nextest) -and ($env:CIVIC_DYNASTY_NEXTEST -eq "1")) {
        $nextestArgs = @("nextest", "run", "--locked", "--lib") + $JobArgs + @("--no-fail-fast")
        if ($TestFilter) {
            $nextestArgs += @("-E", "test($TestFilter)")
        }
        Run-Step "$label (nextest)" {
            & cargo @nextestArgs
            if ($LASTEXITCODE -ne 0) { throw "Library tests failed" }
        }
        return
    }
    $testArgs = @("test", "--quiet", "--locked", "--lib") + $JobArgs
    if ($TestFilter) {
        $testArgs += $TestFilter
    }
    Run-Step $label {
        & cargo @testArgs
        if ($LASTEXITCODE -ne 0) { throw "Library tests failed" }
    }
}

function Run-Changed {
    $changedFiles = @()
    try {
        $diff = & git diff --name-only HEAD 2>$null
        if ($LASTEXITCODE -eq 0 -and $diff) { $changedFiles += $diff -split "`n" | Where-Object { $_ -ne "" } }
    } catch {}
    try {
        $others = & git ls-files --others --exclude-standard 2>$null
        if ($LASTEXITCODE -eq 0 -and $others) { $changedFiles += $others -split "`n" | Where-Object { $_ -ne "" } }
    } catch {}
    $changedFiles = $changedFiles | Sort-Object -Unique
    if (-not $changedFiles -or $changedFiles.Count -eq 0) {
        Write-Host "`n==> Changed domains: no tracked diff — running full library suite" -ForegroundColor Yellow
        Run-Fast
        return
    }
    $seen = @{}
    $filters = @()
    $docsOnly = $true
    foreach ($file in $changedFiles) {
        $candidate = $null
        switch -Wildcard ($file) {
            "src/systems/simulation/*" { $candidate = "simulation"; $docsOnly = $false }
            "src/systems/strategic/*" { $candidate = "strategic"; $docsOnly = $false }
            "src/systems/commands/*" { $candidate = "commands"; $docsOnly = $false }
            "src/systems/bootstrap*" { $candidate = "bootstrap"; $docsOnly = $false }
            "src/systems/invariants*" { $candidate = "invariants"; $docsOnly = $false }
            "src/systems/legal*" { $candidate = "legal"; $docsOnly = $false }
            "src/systems/progression*" { $candidate = "progression"; $docsOnly = $false }
            "src/systems/transactions*" { $candidate = "transactions"; $docsOnly = $false }
            "src/core/*" { $candidate = "core"; $docsOnly = $false }
            "src/ids.rs" { $candidate = "core"; $docsOnly = $false }
            "src/money.rs" { $candidate = "core"; $docsOnly = $false }
            "src/rng.rs" { $candidate = "core"; $docsOnly = $false }
            "src/persistence*" { $candidate = "persistence"; $docsOnly = $false }
            "src/projection*" { $candidate = "projection"; $docsOnly = $false }
            "src/registry/*" { $candidate = "persistence"; $docsOnly = $false }
            "src/gameplay/*" { $candidate = "gameplay"; $docsOnly = $false }
            "src/gameplay_tests.rs" { $candidate = "gameplay"; $docsOnly = $false }
            "src/art/*" { $candidate = "art"; $docsOnly = $false }
            "src/main.rs" { $candidate = ""; $docsOnly = $false }
            "src/lib.rs" { $candidate = ""; $docsOnly = $false }
            "scripts/*" { $candidate = ""; $docsOnly = $false }
            "*.md" { continue }
            "docs/*" { continue }
            default { continue }
        }
        if ($candidate -eq "") {
            Write-Host "`n==> Changed domains span multiple systems ($($changedFiles -join ', ')) — running full library suite" -ForegroundColor Yellow
            Run-Fast
            return
        }
        if ($candidate -and -not $seen.ContainsKey($candidate)) {
            $seen[$candidate] = $true
            $filters += $candidate
        }
    }
    if ($filters.Count -eq 0 -and $docsOnly) {
        Write-Host "`n==> Changed files are docs-only — running documentation checks" -ForegroundColor Cyan
        Run-Docs
        return
    }
    if ($filters.Count -eq 0) {
        Write-Host "`n==> Changed files do not map to a library domain — running full library suite" -ForegroundColor Yellow
        Run-Fast
        return
    }
    if ($filters.Count -eq 1) {
        Write-Host "`n==> Changed domain: $($filters[0]) — running filtered suite" -ForegroundColor Cyan
        Run-Fast $filters[0]
        return
    }
    if ($filters.Count -ge 2 -and $filters.Count -le 3) {
        Write-Host "`n==> Changed domains: $($filters -join ', ') — running targeted multi-filter suite" -ForegroundColor Cyan
        $label = "Library tests matching '$($filters -join ' ')'"
        if ((Has-Nextest) -and ($env:CIVIC_DYNASTY_NEXTEST -eq "1")) {
            $expr = ($filters | ForEach-Object { "test($_)" }) -join " | "
            Run-Step "$label (nextest)" {
                & cargo nextest run --locked --lib @JobArgs --no-fail-fast -E $expr
                if ($LASTEXITCODE -ne 0) { throw "Library tests failed" }
            }
            return
        }
        Run-Step $label {
            & cargo test --quiet --locked --lib @JobArgs -- @filters
            if ($LASTEXITCODE -ne 0) { throw "Library tests failed" }
        }
        return
    }
    Write-Host "`n==> Changed domains: $($filters -join ', ') — running full library suite" -ForegroundColor Yellow
    Run-Fast
}

function Run-Exact([string]$TestName) {
    $profileArgs = @()
    if ($TestName -like "*::soak::*") {
        $profileArgs += "--release"
    }
    Run-Step "Library test '$TestName'" {
        & cargo test --quiet --locked @JobArgs @profileArgs --lib $TestName -- --exact --include-ignored
        if ($LASTEXITCODE -ne 0) { throw "Exact test failed" }
    }
}

function Run-Debug([string]$TestName) {
    $profileArgs = @()
    if ($TestName -like "*::soak::*") {
        $profileArgs += "--release"
    }
    Run-Step "Debug library test '$TestName'" {
        & cargo test --locked @JobArgs @profileArgs --lib $TestName -- --exact --include-ignored --nocapture
        if ($LASTEXITCODE -ne 0) { throw "Debug test failed" }
    }
}

function List-Tests([string]$TestFilter) {
    $testArgs = @("test", "--quiet", "--locked", "--lib") + $JobArgs
    if ($TestFilter) {
        $testArgs += $TestFilter
    }
    $testArgs += @("--", "--list")
    $output = & cargo @testArgs
    $matches = $output | Where-Object { $_ -match ': test$' }
    if (-not $matches) {
        Write-Error "no library tests matched '$TestFilter'"
        exit 2
    }
    $matches | ForEach-Object { Write-Host $_ }
}

function Run-Soak {
    Run-Step "Deterministic soak tests (release)" {
        & cargo test --release --quiet --locked @JobArgs --lib "::soak::" -- --ignored
        if ($LASTEXITCODE -ne 0) { throw "Soak tests failed" }
    }
}

function Run-Docs {
    $python = Resolve-PythonInterpreter
    Run-Step "Documentation consistency" {
        & $python scripts\check_docs.py
        if ($LASTEXITCODE -ne 0) { throw "Documentation consistency check failed" }
    }
    Run-Step "Documentation tests" {
        & cargo test --quiet --locked @JobArgs --doc
        if ($LASTEXITCODE -ne 0) { throw "Doctests failed" }
    }
}

function Run-CliGroup([string]$Group) {
    Run-Step "$Group CLI smoke tests" {
        & powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify_cli.ps1 $Group
        if ($LASTEXITCODE -ne 0) { throw "$Group CLI smoke tests failed" }
    }
}

function Run-GameplayGates {
    Ensure-CliBinary "release"
    Run-Step "Gameplay quality gate (36 campaigns)" {
        & $env:CIVIC_DYNASTY_BINARY playtest --minimum-overall 75 --fail-on-critical --json --output target\gameplay-quality-gate.json
        if ($LASTEXITCODE -ne 0) { throw "Gameplay quality gate failed" }
    }
    Run-Step "Generation-length gameplay gate (3 campaigns, 7200 days)" {
        & $env:CIVIC_DYNASTY_BINARY playtest --days 7200 --persona steward --background baker --trace-limit 20 --minimum-overall 75 --fail-on-critical --json --output target\gameplay-generation-gate.json
        if ($LASTEXITCODE -ne 0) { throw "Generation gameplay gate failed" }
    }
    $python = Resolve-PythonInterpreter
    Run-Step "Generation-length fantasy validation" {
        & $python scripts\check_gameplay.py generation target\gameplay-generation-gate.json
        if ($LASTEXITCODE -ne 0) { throw "Generation fantasy validation failed" }
    }
}

function Run-GameplayAudit {
    Ensure-CliBinary "release"
    $python = Resolve-PythonInterpreter
    Run-Step "Mature multi-seed gameplay audit (2 seeds, 3600 days)" {
        & $env:CIVIC_DYNASTY_BINARY playtest --days 3600 --start-seed 1 --seeds 2 --trace-limit 30 --minimum-overall 75 --fail-on-critical --json --output target\gameplay-deep-audit.json
        if ($LASTEXITCODE -ne 0) { throw "Mature multi-seed gameplay audit failed" }
    }
    Run-Step "Generation-length persona audit (4 personas, 7200 days)" {
        & $env:CIVIC_DYNASTY_BINARY playtest --days 7200 --persona steward --persona entrepreneur --persona power-broker --persona opportunist --background baker --trace-limit 30 --minimum-overall 75 --fail-on-critical --json --output target\gameplay-generation-matrix.json
        if ($LASTEXITCODE -ne 0) { throw "Generation persona audit failed" }
    }
    Run-Step "Opportunist credit stress audit (2 seeds, 7200 days)" {
        & $env:CIVIC_DYNASTY_BINARY playtest --days 7200 --start-seed 1 --seeds 2 --persona opportunist --trace-limit 20 --minimum-overall 75 --fail-on-critical --json --output target\gameplay-credit-stress.json
        if ($LASTEXITCODE -ne 0) { throw "Opportunist credit stress audit failed" }
    }
    Run-Step "Credit stress validation" {
        & $python scripts\check_gameplay.py credit-stress target\gameplay-credit-stress.json
        if ($LASTEXITCODE -ne 0) { throw "Credit stress validation failed" }
    }
    Run-Step "Deep gameplay fantasy validation" {
        & $python scripts\check_gameplay.py generation-matrix target\gameplay-generation-matrix.json
        if ($LASTEXITCODE -ne 0) { throw "Deep gameplay fantasy validation failed" }
    }
}

function Run-Standard {
    Run-ShellSyntaxCheck
    Run-Fast
    if (-not $env:CIVIC_DYNASTY_SKIP_DOCS) {
        Run-Docs
    } else {
        Write-Host "`n==> Documentation checks (skipped via CIVIC_DYNASTY_SKIP_DOCS)" -ForegroundColor Yellow
    }
    if (-not $env:CIVIC_DYNASTY_SKIP_CLI_BUILD) {
        Ensure-CliBinary $(if ($env:CIVIC_DYNASTY_PROFILE) { $env:CIVIC_DYNASTY_PROFILE } else { "debug" })
        Run-CliGroup "core"
    } else {
        Write-Host "`n==> Core CLI smoke tests (skipped via CIVIC_DYNASTY_SKIP_CLI_BUILD)" -ForegroundColor Yellow
    }
}

function Run-CiVerify {
    Run-ShellSyntaxCheck
    Run-Step "Format" {
        & cargo fmt --all -- --check
        if ($LASTEXITCODE -ne 0) { throw "Formatting check failed" }
    }
    # Primary gate first so a failing lib test reports in ~2s warm before clippy/doc.
    Run-Fast
    Run-Docs
    Run-Step "Compile and lint" {
        & cargo clippy --all-targets --all-features --locked -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw "Clippy lint failed" }
    }
    Run-Step "Documentation warnings" {
        $env:RUSTDOCFLAGS = "-D warnings"
        & cargo doc --no-deps --locked
        $env:RUSTDOCFLAGS = $null
        if ($LASTEXITCODE -ne 0) { throw "Cargo doc warnings check failed" }
    }
    Run-Step "Whitespace errors" {
        & git diff-tree --check --no-commit-id --root -r -m HEAD
        if ($LASTEXITCODE -ne 0) { throw "Whitespace errors found" }
    }
}

function Run-CiGates {
    Run-Soak
    Run-Step "Release library tests" {
        & cargo test --release --quiet --locked @JobArgs --lib
        if ($LASTEXITCODE -ne 0) { throw "Release library tests failed" }
    }
    Ensure-CliBinary "release"
    Run-CliGroup "core"
    Run-CliGroup "art"
    Run-CliGroup "gameplay"
    Run-GameplayGates
    Run-Step "Dependency audit" {
        & cargo audit
        if ($LASTEXITCODE -ne 0) { throw "Cargo audit failed" }
    }
}

function Run-SlowGates {
    Run-Soak
    Run-Step "Release library tests" {
        & cargo test --release --quiet --locked @JobArgs --lib
        if ($LASTEXITCODE -ne 0) { throw "Release library tests failed" }
    }
    Ensure-CliBinary "release"
    Run-CliGroup "core"
    Run-CliGroup "art"
    Run-CliGroup "gameplay"
    Run-GameplayGates
}

function Run-All {
    Run-Standard
    Run-Soak
    Ensure-CliBinary $(if ($env:CIVIC_DYNASTY_PROFILE) { $env:CIVIC_DYNASTY_PROFILE } else { "debug" })
    Run-CliGroup "art"
    Run-CliGroup "gameplay"
    Run-GameplayGates
}

# Capture the exact repository bytes before a receipt-eligible lane starts. A
# concurrent edit during validation then prevents the receipt from being issued.
$ReceiptLane = $null
if (($Mode -eq "fast" -or $Mode -eq "quick") -and -not $Filter) {
    $ReceiptLane = "quick"
} elseif ($Mode -eq "standard" -or $Mode -eq "all") {
    $ReceiptLane = "standard"
}
$ReceiptPython = $null
$ReceiptStart = $null
if ($ReceiptLane) {
    $ReceiptPython = Resolve-PythonInterpreter
    $ReceiptStart = (& $ReceiptPython scripts/validation_receipt.py fingerprint).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $ReceiptStart) { throw "Failed to fingerprint repository before validation" }
}

switch ($Mode) {
    "check"          { Run-Check $Filter }
    "fast"           { Run-Fast $Filter }
    "quick"          { Run-Fast $Filter }
    "changed"        { Run-Changed }
    "standard"       { Run-Standard }
    "exact"          { if (-not $Filter) { Show-Usage }; Run-Exact $Filter }
    "debug"          { if (-not $Filter) { Show-Usage }; Run-Debug $Filter }
    "list"           { List-Tests $Filter }
    "soak"           { Run-Soak }
    "docs"           { Run-Docs }
    "cli"            { Ensure-CliBinary $(if ($env:CIVIC_DYNASTY_PROFILE) { $env:CIVIC_DYNASTY_PROFILE } else { "debug" }); Run-CliGroup "core" }
    "art-cli"        { Ensure-CliBinary $(if ($env:CIVIC_DYNASTY_PROFILE) { $env:CIVIC_DYNASTY_PROFILE } else { "debug" }); Run-CliGroup "art" }
    "gameplay-cli"   { Ensure-CliBinary $(if ($env:CIVIC_DYNASTY_PROFILE) { $env:CIVIC_DYNASTY_PROFILE } else { "debug" }); Run-CliGroup "gameplay" }
    "adapters"       {
        Ensure-CliBinary $(if ($env:CIVIC_DYNASTY_PROFILE) { $env:CIVIC_DYNASTY_PROFILE } else { "debug" })
        Run-CliGroup "core"
        Run-CliGroup "art"
        Run-CliGroup "gameplay"
    }
    "gameplay"       { Run-GameplayGates }
    "gameplay-audit" { Run-GameplayAudit }
    "playtest"       {
        $profile = if ($env:CIVIC_DYNASTY_PROFILE) { $env:CIVIC_DYNASTY_PROFILE } else { "debug" }
        Ensure-CliBinary $profile
        if (-not $Rest -or $Rest.Count -eq 0) {
            Run-Step "Gameplay harness run" {
                & $env:CIVIC_DYNASTY_BINARY playtest --days 60 --seeds 1 --persona steward --background baker --trace-limit 8
                if ($LASTEXITCODE -ne 0) { throw "Gameplay harness run failed" }
            }
        } else {
            Run-Step "Gameplay harness run" {
                & $env:CIVIC_DYNASTY_BINARY playtest @Rest
                if ($LASTEXITCODE -ne 0) { throw "Gameplay harness run failed" }
            }
        }
    }
    "ci-verify"      { Run-CiVerify }
    "ci"             { Run-CiVerify }
    "ci-gates"       { Run-CiGates }
    "slow"           { Run-SlowGates }
    "deep"           { Run-SlowGates; Run-GameplayAudit }
    "all"            { Run-All }
    default          { Show-Usage }
}

if ($ReceiptLane) {
    & $ReceiptPython scripts/validation_receipt.py record $ReceiptLane $ReceiptStart
    if ($LASTEXITCODE -ne 0) { throw "Failed to record validation receipt" }
}

$LaneStopwatch.Stop()
Write-Host ("`n==> Lane '{0}' completed in {1}" -f $Mode, (Format-Duration $LaneStopwatch.Elapsed.TotalSeconds)) -ForegroundColor Cyan
