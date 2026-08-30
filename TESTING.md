# Testing

This document defines test tiers, suite organization, assertion standards, and completion gates.

## Commands

| Goal | Command |
|---|---|
| One domain or behavior | `bash scripts/test.sh fast <filter>` |
| Fastest library-only loop | `bash scripts/test.sh quick <filter>` |
| All ordinary library tests | `bash scripts/test.sh fast` |
| Normal pre-commit loop | `bash scripts/test.sh standard` |
| List matching tests | `bash scripts/test.sh list <filter>` |
| One exact test | `bash scripts/test.sh exact <fully-qualified-name>` |
| One exact test with output | `bash scripts/test.sh debug <fully-qualified-name>` |
| Deterministic soaks | `bash scripts/test.sh soak` |
| Documentation checks | `bash scripts/test.sh docs` |
| Core CLI smoke | `bash scripts/test.sh cli` |
| Art CLI smoke | `bash scripts/test.sh art-cli` |
| Gameplay CLI smoke | `bash scripts/test.sh gameplay-cli` |
| All adapter smoke groups | `bash scripts/test.sh adapters` |
| Focused harness run | `bash scripts/test.sh playtest [args...]` |
| Release gameplay gates | `bash scripts/test.sh gameplay` |
| Deep gameplay design audit | `bash scripts/test.sh gameplay-audit` |
| Fast CI verification lane | `bash scripts/test.sh ci-verify` |
| Deep CI gates lane | `bash scripts/test.sh ci-gates` |
| Heavy release gates without audit | `bash scripts/test.sh slow` |
| Full deep design gate | `bash scripts/test.sh deep` |
| Complete scripted test tier | `bash scripts/test.sh all` |

Successful steps print concise summaries with elapsed time. Failures print the complete command output, including compiler diagnostics when a CLI build fails. A filter matching no executable library test exits with code 2.

On Windows without bash on PATH, every mode is also available through `.\scripts\test.ps1 <mode> [filter]`, which mirrors `scripts/test.sh` including receipts, CLI reuse, and lane timing.

## Runner environment

| Variable | Effect |
|---|---|
| `CIVIC_DYNASTY_JOBS=<n>` | Forwards `--jobs <n>` to every `cargo test` / `cargo build`; lower it to keep a busy machine responsive. |
| `CIVIC_DYNASTY_NEXTEST=1` | Runs library tests under `cargo-nextest` instead of plain `cargo test`: per-test isolation at some warm-run speed. |
| `CIVIC_DYNASTY_SKIP_CLI_BUILD=1` | Skips CLI rebuilds when iterating on library code only. |
| `CIVIC_DYNASTY_PROFILE=release` | Makes adapter smoke groups use a release binary. |
| `CIVIC_DYNASTY_BINARY=<path>` | Uses an existing binary for smoke groups so gates share one build. |
| `CIVIC_DYNASTY_BINARY_OVERRIDE=<path>` | Pins an exact binary over every profile choice. Gameplay gates still rebuild release when a debug binary would otherwise leak into them. |
| `CIVIC_DYNASTY_PRE_PUSH=standard` | Strengthens the pre-push hook from its `quick` default. |
| `CIVIC_DYNASTY_PYTHON=<interpreter>` | Selects the Python interpreter (auto-detects `python3`, `python`, Windows `py`). |

## Build profiles

- `dev` / `test`: this crate builds at `opt-level = 1` with dependencies fully optimized, so simulation-heavy tests finish in seconds. Keep new tests free of sleeps and wall-clock dependence.
- `release`: the everyday optimized profile for soaks, adapter smokes, and gameplay gates — parallel codegen units keep an edited file's optimized rebuild in seconds while throughput stays near peak and gameplay output stays identical.
- `release-max`: build only when peak performance itself is under measurement.

Plain `cargo test` shares campaign-fixture setup inside one process and is the fastest default runner. Long-running gameplay JSON assertions live in `scripts/check_gameplay.py`.

The `adapters` and `gameplay` modes build their local CLI once and reuse it across all sub-gates.

## Receipts and hooks

Successful unfiltered `quick`/`fast`, `standard`, and `all` runs record a content-addressed receipt under local Git metadata. The pre-push hook reuses a current receipt of equal or broader routine strength instead of recompiling identical repository bytes; any tracked or non-ignored content change invalidates it, and receipt-eligible lanes refuse to issue evidence if repository bytes change mid-run.

Optional hooks install with `bash scripts/install_hooks.sh`: pre-commit runs format, shell syntax, and whitespace checks; pre-push defaults to `quick`. Use `git commit --no-verify` during focused iteration.

## Test tiers

| Tier | Purpose | Expected use |
|---|---|---|
| Fast library | Deterministic unit and focused behavioral coverage | Normal edit-test cycle |
| Standard | Syntax, fast library, docs, core CLI | Normal pre-commit |
| Adapter smoke | External CLI contracts grouped by core, art, and gameplay | Adapter changes |
| Soak | Long deterministic invariant and multi-generation behavior | Accumulating simulation changes |
| Gameplay | Release-mode systemic quality and succession gates | Cross-domain gameplay changes |
| Gameplay audit | Larger matrices for rare and mature behavior | Design review |
| CI verify | The exact fast CI verification lane | Reproducing the fast lane locally |
| CI gates | The deep CI lane; requires `cargo-audit` | Reproducing release, adapter, gameplay, and security gates |
| Slow | Release gates without the security audit or design audit | Deep verification without the audit dependency |
| Deep | The complete design gate: slow gates plus gameplay audit | Design review and deepest verification |
| All | Standard + soak + adapters + gameplay gates | Cross-cutting test coverage |

Fast tests must not use sleeps, wall-clock time, external services, or environment-dependent behavior. Soak tests always run in release mode; their assertions stay identical across profiles.

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
bash scripts/test.sh standard
```

Specialized lanes are selected by the contract that changed:

- `soak` for long-horizon simulation, determinism, or invariant evidence;
- `adapters` for CLI/adapter surfaces;
- `gameplay` or `gameplay-audit` for gameplay-report and design-evaluation contracts;
- `docs` for documentation infrastructure;
- `slow` for release-profile behavior that can differ from development;
- `ci-gates`, `all`, or `deep` only for verification topology, dependency/security work, broadly shared build configuration, or a deliberate release/deep-design checkpoint.

Persistence, public APIs, command schemas, simulation order, arithmetic, invariants, shared state, and gameplay-report schemas require focused owner coverage plus the relevant specialized lane above. This is a coverage requirement, not automatically two invocations: when the selected lane already executes the necessary owner coverage, do not rerun a focused test beforehand. They do not automatically require every deep command in the repository.

Do not run a compile-only or lint build immediately before an executable lane that necessarily recompiles the same changed surface unless the separate diagnostic is itself required. Prefer one build-producing operation per checkpoint. GitHub Actions and hosted runners are not part of the verification path.

The local runner owns the scripted lanes so focused and complete reproduction use the same commands. The `all` tier reuses one debug CLI build across adapter smoke groups and remains an explicit broad tier rather than a routine prerequisite.
