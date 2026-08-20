# Native PowerShell Test and Verification Runner for Civic Dynasty
param(
    [Parameter(Position=0)]
    [string]$Mode = "fast",

    [Parameter(Position=1)]
    [string]$Filter = ""
)

$ErrorActionPreference = "Stop"

$ProjectRoot = (Resolve-Path "$PSScriptRoot\..").Path
Set-Location $ProjectRoot

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
  .\scripts\test.ps1 fast [filter]       run non-ignored library tests (default loop)
  .\scripts\test.ps1 quick [filter]      fastest loop: same as fast, skips docs/CLI
  .\scripts\test.ps1 standard            pre-commit loop: syntax, library, docs, core CLI smoke
  .\scripts\test.ps1 exact <test-name>   run one fully qualified library test
  .\scripts\test.ps1 debug <test-name>   run one exact test with captured output enabled
  .\scripts\test.ps1 list [filter]       list matching library tests
  .\scripts\test.ps1 soak                run soak tests in release mode
  .\scripts\test.ps1 docs                run documentation consistency and doctests
  .\scripts\test.ps1 cli                 run core campaign/projection/dashboard CLI smoke tests
  .\scripts\test.ps1 art-cli             run procedural-art CLI smoke tests
  .\scripts\test.ps1 gameplay-cli        run gameplay-harness CLI smoke tests
  .\scripts\test.ps1 adapters            run all CLI smoke groups
  .\scripts\test.ps1 gameplay            run release gameplay and generation-length quality gates
  .\scripts\test.ps1 gameplay-audit      run mature multi-seed, generation, and credit-stress audits
  .\scripts\test.ps1 ci-verify           fast CI lane: format, clippy, library, docs
  .\scripts\test.ps1 ci-gates            deep CI lane: release tests, soaks, adapters, gameplay, audit
  .\scripts\test.ps1 all                 everything: standard + soak + adapters + gameplay
  .\scripts\test.ps1 slow                heavy release gates (ci-gates minus audit)
  .\scripts\test.ps1 deep                deepest design gates (slow + gameplay-audit)

environment:
  CIVIC_DYNASTY_JOBS           pass --jobs N to cargo test/build commands
  CIVIC_DYNASTY_PROFILE        debug (default) or release for adapter smoke builds
  CIVIC_DYNASTY_BINARY         reuse a prebuilt CLI binary for adapter smoke groups
  CIVIC_DYNASTY_SKIP_CLI_BUILD skip CLI rebuild when set (fast lib-only iteration)
  CIVIC_DYNASTY_PYTHON         select an explicit Python interpreter
"@ | Write-Host
    exit 2
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

function Run-Fast([string]$TestFilter) {
    $label = if ($TestFilter) { "Library tests matching '$TestFilter'" } else { "Library tests" }
    if ((Has-Nextest) -and (-not $env:CIVIC_DYNASTY_NO_NEXTEST)) {
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
    Run-Step "Gameplay quality gate" {
        & $env:CIVIC_DYNASTY_BINARY playtest --minimum-overall 75 --fail-on-critical --json --output target\gameplay-quality-gate.json
        if ($LASTEXITCODE -ne 0) { throw "Gameplay quality gate failed" }
    }
    Run-Step "Generation-length gameplay gate" {
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
    Run-Step "Mature multi-seed gameplay audit" {
        & $env:CIVIC_DYNASTY_BINARY playtest --days 3600 --start-seed 1 --seeds 2 --trace-limit 30 --minimum-overall 75 --fail-on-critical --json --output target\gameplay-deep-audit.json
        if ($LASTEXITCODE -ne 0) { throw "Mature multi-seed gameplay audit failed" }
    }
    Run-Step "Generation-length persona audit" {
        & $env:CIVIC_DYNASTY_BINARY playtest --days 7200 --persona steward --persona entrepreneur --persona power-broker --persona opportunist --background baker --trace-limit 30 --minimum-overall 75 --fail-on-critical --json --output target\gameplay-generation-matrix.json
        if ($LASTEXITCODE -ne 0) { throw "Generation persona audit failed" }
    }
    Run-Step "Opportunist credit stress audit" {
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
    Run-Fast
    Run-Docs
    if (-not $env:CIVIC_DYNASTY_SKIP_CLI_BUILD) {
        Ensure-CliBinary $(if ($env:CIVIC_DYNASTY_PROFILE) { $env:CIVIC_DYNASTY_PROFILE } else { "debug" })
        Run-CliGroup "core"
    } else {
        Write-Host "`n==> Core CLI smoke tests (skipped via CIVIC_DYNASTY_SKIP_CLI_BUILD)" -ForegroundColor Yellow
    }
}

function Run-CiVerify {
    Run-Step "Format" {
        & cargo fmt --all -- --check
        if ($LASTEXITCODE -ne 0) { throw "Formatting check failed" }
    }
    Run-Step "Compile and lint" {
        & cargo clippy --all-targets --all-features --locked -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw "Clippy lint failed" }
    }
    Run-Fast
    Run-Docs
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
    "fast"           { Run-Fast $Filter }
    "quick"          { Run-Fast $Filter }
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
    "ci-verify"      { Run-CiVerify }
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
