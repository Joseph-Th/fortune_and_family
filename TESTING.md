# Testing

This document defines test tiers, suite organization, assertion standards, and completion gates.

## Commands

| Goal | Command | Warm cost | When to use |
|---|---|---|---|
| Fastest syntax check | `bash scripts/test.sh check [filter]` | ~1s | Editor feedback — no tests run |
| One domain or behavior | `bash scripts/test.sh fast <filter>` | ~2s | Tight edit loop (e.g. `fast simulation`) |
| Fastest library-only loop | `bash scripts/test.sh quick <filter>` | ~2s | Alias for fast, never triggers docs/CLI |
| All ordinary library tests | `bash scripts/test.sh fast` | ~2s | Full library sweep |
| Normal pre-commit loop | `bash scripts/test.sh standard` | ~4s | Pre-commit: syntax + lib + docs + core CLI |
| List matching tests | `bash scripts/test.sh list <filter>` | <1s | Discover filter names |
| One exact test | `bash scripts/test.sh exact <fully-qualified-name>` | ~1s | Pinpoint a single test |
| One exact test with output | `bash scripts/test.sh debug <fully-qualified-name>` | ~1s | Pinpoint with --nocapture |
| Deterministic soaks | `bash scripts/test.sh soak` | ~1s warm / ~50s cold | Long-horizon invariants (release) |
| Documentation checks | `bash scripts/test.sh docs` | ~1s warm | Docs + doctests |
| Core CLI smoke | `bash scripts/test.sh cli` | ~1s warm | Campaign/projection CLI |
| Art CLI smoke | `bash scripts/test.sh art-cli` | ~1s warm | Sprite review CLI |
| Gameplay CLI smoke | `bash scripts/test.sh gameplay-cli` | ~1s warm | Harness CLI (30-day) |
| All adapter smoke groups | `bash scripts/test.sh adapters` | ~2s warm | All CLI surfaces (one CLI build) |
| Focused harness run | `bash scripts/test.sh playtest [args...]` | <1s warm (debug) | Single campaign iteration (debug by default) |
| Release gameplay gates | `bash scripts/test.sh gameplay` | ~16s warm | 36 + 3 campaigns, 60k days (release, one CLI build) |
| Deep gameplay design audit | `bash scripts/test.sh gameplay-audit` | ~30s warm | Multi-seed / generation / credit stress (release) |
| Fast CI verification lane | `bash scripts/test.sh ci-verify` | ~6s warm | Format + clippy + lib + docs + doc warnings |
| Deep CI gates lane | `bash scripts/test.sh ci-gates` | ~1 min | Release + soaks + adapters + gameplay + audit |
| Heavy release gates without audit | `bash scripts/test.sh slow` | ~45s | Release gates without security audit |
| Full deep design gate | `bash scripts/test.sh deep` | ~1.2 min | slow + gameplay-audit |
| Complete scripted test tier | `bash scripts/test.sh all` | ~25s | Standard + soak + adapters + gameplay |

Successful steps print concise summaries with elapsed time. Failures print the complete command output, including compiler diagnostics when a CLI build fails. A filter matching no executable library test exits with code 2.

On Windows without bash on PATH, every mode is also available through `.\scripts\test.ps1 <mode> [filter]`, which mirrors `scripts/test.sh` including receipts, CLI reuse, and lane timing.

## Runner environment

| Variable | Effect |
|---|---|
| `CIVIC_DYNASTY_JOBS=<n>` | Forwards `--jobs <n>` to every `cargo test` / `cargo build` and caps gameplay-harness campaign parallelism; lower it to keep a busy machine responsive. |
| `CIVIC_DYNASTY_NEXTEST=1` | Runs library tests under `cargo-nextest` instead of plain `cargo test`: per-test isolation at some warm-run speed. |
| `CIVIC_DYNASTY_SKIP_CLI_BUILD=1` | Skips CLI rebuilds when iterating on library code only. |
| `CIVIC_DYNASTY_PROFILE=release` | Makes `adapters`/`playtest` use a release binary. Gate lanes (`gameplay`, `ci-gates`, `slow`) always use release regardless. |
| `CIVIC_DYNASTY_BINARY=<path>` | Uses an existing binary for smoke groups so gates share one build. |
| `CIVIC_DYNASTY_BINARY_OVERRIDE=<path>` | Pins an exact binary over every profile choice. Gameplay gates still rebuild release when a debug binary would otherwise leak into them. |
| `CIVIC_DYNASTY_PRE_PUSH=standard` | Strengthens the pre-push hook from its `quick` default. |
| `CIVIC_DYNASTY_PYTHON=<interpreter>` | Selects the Python interpreter (auto-detects `python3`, `python`, Windows `py`). |

## Build profiles

- `dev` / `test`: this crate builds at `opt-level = 1` with dependencies at `2`, so simulation-heavy tests finish in seconds. Incremental compilation and 16 codegen units keep an edited file rebuild to ~1s. Keep new tests free of sleeps and wall-clock dependence.
- `check`: inherits `dev` but is never executed — used by `cargo check` for sub-second syntax feedback.
- `release`: the everyday optimized profile for soaks and gameplay gates — `opt-level = 3`, 16 codegen units, incremental, no LTO, so a warm edited-file rebuild is ~1s while throughput stays within ~10% of peak and gameplay output is identical.
- `release-max`: serialized single-unit + thin-LTO. Build only when peak performance itself is under measurement (`cargo build --profile release-max`).

Build tuning for a solo local machine is intentionally minimal:
`.cargo/config.toml` keeps `incremental = true`, `pipelining = true`, and `jobs = 0` (all cores),
and `Cargo.toml` keeps 16 parallel codegen units in every profile.
No remote cache or wrapper is required — warm `fast` is ~2s, `standard` is ~4s,
`ci-verify` is ~6s warm (incremental clippy <1s after the first build), and a focused
`playtest` is <1s because the debug CLI stays hot until you need a release gate.
`gameplay` is ~16s warm (one release CLI build + 39 campaigns, 60k simulated days).
Each lane reuses the same incremental cache; a one-line lib change rebuilds
only that crate in ~1s, not the whole workspace.

Targeted iteration: use `check` for syntax (~1s, no tests),
`fast <filter>` for one domain (~2s), `standard` once behavior is ready (~4-5s).
`adapters` and `gameplay` lanes reuse a single CLI build across sub-gates
so you never pay one build per smoke group.
`CIVIC_DYNASTY_JOBS=4` caps both cargo parallelism and harness campaign
fan-out for a loaded machine.

Plain `cargo test` shares campaign-fixture setup inside one process and is the fastest default runner. Long-running gameplay JSON assertions live in `scripts/check_gameplay.py`.

The `adapters` and `gameplay` modes build their local CLI once and reuse it across all sub-gates. `playtest` uses the debug CLI by default for <1s warm iteration — set `CIVIC_DYNASTY_PROFILE=release` or run `gameplay`/`ci-gates` when you need gate-faithful throughput.

## Receipts and hooks

Successful unfiltered `quick`/`fast`, `standard`, and `all` runs record a content-addressed receipt under local Git metadata. The pre-push hook reuses a current receipt of equal or broader routine strength instead of recompiling identical repository bytes; any tracked or non-ignored content change invalidates it, and receipt-eligible lanes refuse to issue evidence if repository bytes change mid-run.

Optional hooks install with `bash scripts/install_hooks.sh`: pre-commit runs format, shell syntax, and whitespace checks; pre-push defaults to `quick` (2s warm). This keeps solo workflow snappy — a push reuses a receipt when the tree hasn't changed, so you are never forced into a 40s release build just to push a doc fix. Use `git commit --no-verify` during focused iteration and `CIVIC_DYNASTY_PRE_PUSH=standard` when you want the fuller 4s gate on push.

## Test tiers

| Tier | Purpose | Expected use | Warm budget |
|---|---|---|---|
| Check | Syntax/type only (`cargo check --lib --bins`) | Editor feedback before compile | ~1s |
| Fast library | Deterministic unit and focused behavioral coverage | Normal edit-test cycle | ~2s |
| Standard | Check + fast library + docs + core CLI | Normal pre-commit | ~4s |
| Adapter smoke | External CLI contracts grouped by core, art, and gameplay | Adapter changes | ~2s |
| Soak | Long deterministic invariant and multi-generation behavior | Accumulating simulation changes | ~1s warm |
| Gameplay | Release-mode systemic quality and succession gates | Cross-domain gameplay changes | ~16s |
| Gameplay audit | Larger matrices for rare and mature behavior | Design review | ~30s |
| CI verify | The exact fast CI verification lane | Reproducing the fast lane locally | ~6s warm |
| CI gates | The deep CI lane; requires `cargo-audit` | Reproducing release, adapter, gameplay, and security gates | ~1 min |
| Slow | Release gates without the security audit or design audit | Deep verification without the audit dependency | ~45s |
| Deep | The complete design gate: slow gates plus gameplay audit | Design review and deepest verification | ~1.2 min |
| All | Standard + soak + adapters + gameplay gates | Cross-cutting test coverage | ~25s |

Only `fast`/`quick`/`check` belong in the inner loop. Do not run `ci-gates`, `slow`, `deep`, or `all` before every commit — they exist for release or deep-design checkpoints and intentionally trade thoroughness for 40s–60s of build + simulation. The pre-push hook defaults to `quick` precisely so a solo developer is never blocked by a heavy gate after a one-line fix.

Fast tests must not use sleeps, wall-clock time, external services, or environment-dependent behavior. Soak tests always run in release mode (debug would be ~100× slower for thousands of simulated days); their assertions stay identical across profiles. Use `check` (no test execution) when you only need to prove syntax compiles.

## Suite organization

Large suites live beside their production owner:

| Area | Test file |
|---|---|
| Core state and stores | `src/core/state_tests.rs` |
| Campaign bootstrap | `src/systems/bootstrap_tests.rs` |
| Player commands | `src/systems/commands/commands_tests.rs` |
| Daily simulation | `src/systems/simulation/simulation_tests.rs` |
| Strategic systems | `src/systems/strategic/strategic_tests.rs` |
| Persistence | `src/persistence_tests.rs` |
| Projections and HTML | `src/projection_tests.rs` |
| Gameplay harness | `src/gameplay_tests.rs` |
| Art | `src/art/*` module tests |
| Shared fixtures | `src/test_support.rs` |

Use stable nested modules so filters remain useful. Put a test in the narrowest domain that owns the behavior. Create an external integration-test target only for a true external crate boundary.

## Fixtures

- `rivergate_registry_for_test` returns the shared immutable registry.
- `make_test_campaign` returns an isolated clone of the deterministic default campaign.
- `make_test_campaign_with` is for tests whose seed, name, or starting background is part of the contract.

Select fixture data semantically. Prefer "property owned by the player and not pledged" over a hard-coded ID or collection position.

Extract setup helpers when they describe reusable domain conditions. Extract assertion helpers only when reuse improves clarity or diagnostics; mark shared assertion helpers `#[track_caller]`.

## Assertion standards

Assert public behavior, durable state, accounting, or explicit invariants.

- Use exact values for accounting, arithmetic boundaries, schemas, serialization, and ordering when order is a contract.
- Use relational assertions for intentionally flexible emergent behavior.
- Compare sets for exhaustive route or enum coverage and report missing/unexpected members.
- Assert preconditions when a test could otherwise pass vacuously.
- Prefer typed error variants and fields over formatted error text.
- For successful domain commands, assert durable state and typed feedback categories rather than user-facing prose unless text is the contract.
- When matching history, prove the command added the relevant event with a count delta or newly appended typed record.
- Use `assert_state_unchanged` for rejected mutations and stale-token commits.
- Use `assert_state_eq` when full-state equality is the contract and first-difference diagnostics are useful.
- Derive expected values from arranged state when fixture details are not themselves the contract.

Avoid incidental IDs, generated prose, internal call order, and unrelated state.

## Coverage expectations

Consequential mutation normally requires:

1. Successful state transition.
2. Rejected precondition with unchanged state.
3. Arithmetic, range, capacity, or lifecycle boundary coverage.
4. Stale-token coverage when validation and commit are separate.
5. Persistence coverage when serialized state changes.
6. Deterministic replay when ordering or randomness matters.
7. Invariant coverage for new cross-record requirements.
8. Projection or durable-feedback coverage when the result must be observable.

Save-schema changes additionally require a schema increment, rejection tests for non-current schema versions, current-schema round-trip equality, invalid-state rejection, and atomic-write tests when write behavior changes.

Gameplay-harness changes should cover candidate discoverability, classification, pacing, consequence attribution, relevant resilience metrics, progression, and structured report semantics. Keep finding-rule tests cheap; long-horizon behavior belongs in explicit harness or release gameplay tiers. See `GAMEPLAY_HARNESS.md`.

## Failure diagnostics

A useful failure identifies the behavior, expected and observed values, relevant entity IDs/state, and the first differing path when state should remain equal.

Collection helpers should show observed members. Candidate and finding helpers should show available candidates or finding titles.

## Completion gate

Run the narrowest relevant subset while editing. Once behavior is ready, run one routine completion lane rather than climbing through progressively broader tiers:

```bash
bash scripts/test.sh check          # ~1s syntax only, before any test run
bash scripts/test.sh fast simulation # ~2s for the one domain you touched
bash scripts/test.sh standard        # ~4-5s normal pre-commit once behavior is ready
```

Specialized lanes are selected by the contract that changed:

- `soak` for long-horizon simulation, determinism, or invariant evidence;
- `adapters` for CLI/adapter surfaces;
- `gameplay` or `gameplay-audit` for gameplay-report and design-evaluation contracts;
- `docs` for documentation infrastructure;
- `slow` for release-profile behavior that can differ from development;
- `ci-gates`, `all`, or `deep` only for verification topology, dependency/security work, broadly shared build configuration, or a deliberate release/deep-design checkpoint.

Solo-dev rule: if `fast <filter>` already covered the changed surface and `standard` is green, you are done. Do not escalate to `all`/`slow`/`ci-gates` for a typical feature — those lanes exist to verify cross-cutting changes, not to gate every doc fix.

Persistence, public APIs, command schemas, simulation order, arithmetic, invariants, shared state, and gameplay-report schemas require focused owner coverage plus the relevant specialized lane above. This is a coverage requirement, not automatically two invocations: when the selected lane already executes the necessary owner coverage, do not rerun a focused test beforehand. They do not automatically require every deep command in the repository.

Do not run a compile-only or lint build immediately before an executable lane that necessarily recompiles the same changed surface unless the separate diagnostic is itself required. Prefer one build-producing operation per checkpoint. GitHub Actions and hosted runners are not part of the verification path.

The local runner owns the scripted lanes so focused and complete reproduction use the same commands. The `all` tier reuses one debug CLI build across adapter smoke groups and remains an explicit broad tier rather than a routine prerequisite.
