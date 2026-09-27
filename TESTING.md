# Testing

Test tiers, suite organization, assertion standards, and completion gates.

## Commands

Solo-dev iteration is one command — other lanes run only when their contract changed.

| Goal | Command | Relative cost | When |
|---|---|---|---|
| Syntax check | `bash scripts/test.sh check [filter]` | fastest, cached | Editor feedback, no tests |
| One domain | `bash scripts/test.sh fast <filter>` | edit-bound, cached fast | Tight loop, e.g. `fast simulation` |
| Changed domains | `bash scripts/test.sh changed` | edit-bound, cached fast | Auto-detects touched domains from `git diff` |
| Library sweep | `bash scripts/test.sh fast` | edit-bound, cached fast | Full library |
| Pre-commit | `bash scripts/test.sh standard` | slower than fast | Syntax + lib + docs + core CLI |
| List candidates | `bash scripts/test.sh list <filter>` | fastest | Discover filter names |
| One test | `bash scripts/test.sh exact <name>` | fast | Pinpoint single test |
| One test with output | `bash scripts/test.sh debug <name>` | fast | With `--nocapture` |
| Long horizons | `bash scripts/test.sh soak` | fast once built (release) | Determinism and multi-generation invariants (release) |
| Docs | `bash scripts/test.sh docs` | fast | Links and prose contracts |
| Adapter smoke | `bash scripts/test.sh adapters` | fast | All CLI surfaces, one build |
| Harness smoke | `bash scripts/test.sh playtest [args]` | fast (debug) | 60-day single persona, debug |
| Gameplay gate | `bash scripts/test.sh gameplay` | medium (release) | 36+3 campaigns, 60k days (release) |
| Design audit | `bash scripts/test.sh gameplay-audit` | medium (release) | Multi-seed and credit stress (release) |
| CI verify | `bash scripts/test.sh ci-verify` | fast | Format + clippy + lib + docs (`bash scripts/test.sh ci` is an alias) |
| Deep CI | `bash scripts/test.sh ci-gates` | slow (release) | Release + soaks + adapters + gameplay + audit |
| Single CLI | `bash scripts/test.sh cli` | fast | Core CLI smoke |
| Art CLI | `bash scripts/test.sh art-cli` | fast | Sprite review CLI |
| Harness CLI | `bash scripts/test.sh gameplay-cli` | fast | Harness CLI (30-day) |
| Quick | `bash scripts/test.sh quick` | fast | Alias for `fast` (no docs/CLI) |
| Release without audit | `bash scripts/test.sh slow` | ~45s | Release gates without `cargo-audit` |
| Full design gate | `bash scripts/test.sh deep` | ~1.2min | `slow` + `gameplay-audit` |
| Everything | `bash scripts/test.sh all` | ~25s | `standard` + `soak` + `adapters` + `gameplay` |

Failures print full diagnostics. A filter matching no test exits with code 2. On Windows without bash, use `.\scripts\test.ps1 <mode> [filter>`.

## Rust agent diagnostics

The test script remains the verification owner. Use these Cargo tools only when they answer a narrower agent question:

- `cargo modules structure --lib --no-fns --no-traits --no-types --max-depth 4` provides a bounded ownership map when module/file headers and architecture docs do not yet identify the owner. Narrow with `--focus-on <module>`.
- `cargo mutants --list --file <owner.rs>` inventories candidate semantic changes. Execute only a narrow file or selected mutants when focused domain, persistence, or invariant tests may not constrain a meaningful wrong result. Mutation percentages are not quality targets.
- `cargo expand --lib <module::item>` is an on-demand inspection tool for material macro/derive output.

Do not add global module-cycle/orphan gates or mutation execution to `quick`, `standard`, `ci-verify`, or `deep` without separate project calibration. Keep mutation runs isolated and avoid `--in-place` on dirty/concurrent worktrees.

## Runner environment

| Variable | Effect |
|---|---|
| `CIVIC_DYNASTY_JOBS=<n>` | Caps cargo jobs and harness parallelism |
| `CIVIC_DYNASTY_NEXTEST=1` | Run under `cargo-nextest` |
| `CIVIC_DYNASTY_SKIP_CLI_BUILD=1` | Skip CLI rebuild for lib-only iteration |
| `CIVIC_DYNASTY_SKIP_DOCS=1` | Skip docs in `standard` |
| `CIVIC_DYNASTY_PROFILE=release` | Force `adapters`/`playtest` to release binary; gate lanes always use release |
| `CIVIC_DYNASTY_BINARY=<path>` | Reuse existing binary for smoke groups |
| `CIVIC_DYNASTY_PRE_PUSH=standard` | Use `standard` for pre-push hook (`quick` is default) |
| `CIVIC_DYNASTY_PYTHON=<interpreter>` | Select Python interpreter |

## Build profiles

- `check`: inherits `dev`, never executed; cached `cargo check` feedback is the fastest lane.
- `dev` / `test`: `opt-level = 1` (deps `2`), 16 codegen units, incremental. Single-crate incremental rebuilds are edit-bound and cached runs are fast; suite exec is faster than the compile. First cold clippy and release builds are one-time costs.
- `release`: `opt-level = 3`, 16 codegen units, incremental, no LTO. Used for `soak`/`gameplay`; warm rebuilds stay close to peak throughput.
- `release-max`: single codegen unit + thin LTO; peak measurement only.

`check` and `test` share dependency artifacts warm. Single-crate rebuild cost is the compile, not test exec.

## Receipts and hooks

Unfiltered `quick`/`fast`, `standard`, and `all` record a content-addressed receipt under Git metadata. The pre-push hook reuses a receipt of equal or broader strength instead of recompiling identical bytes. Any tracked or non-ignored change invalidates it; receipts refuse to issue if bytes change mid-run.

Install hooks once: `bash scripts/install_hooks.sh` (`core.hooksPath` → `scripts/hooks`). `pre-commit` is format + shell + whitespace. `pre-push` defaults to `quick` and skips the build when a current receipt exists. Use `git commit --no-verify` mid-edit; set `CIVIC_DYNASTY_PRE_PUSH=standard` for a stronger push gate.

## Test tiers

| Tier | Purpose | Relative cost |
|---|---|---|
| Check | Syntax and types | fastest |
| Fast library | Deterministic unit and focused behavior | fast |
| Standard | Check + fast + docs + core CLI | slower than fast |
| Adapter smoke | CLI contracts | fast |
| Soak | Long-horizon invariants | fast once built (release) |
| Gameplay | Systemic quality and succession | medium (release) |
| Gameplay audit | Rare and mature behavior | medium (release) |
| CI verify | Fast CI lane | fast |
| CI gates | Deep CI lane | slow (release) |
| All | Standard + soak + adapters + gameplay | medium (release) |

`fast`/`quick`/`check`/`changed` belong in the inner loop. `ci-gates`, `slow`, `deep`, `all` are release checkpoints. Fast tests use no sleeps or external services. Soak tests run in release (debug would be ~100× slower); assertions are identical across profiles.

## Suite organization

| Area | File |
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

Use stable nested modules so filters remain useful. Put each test in the narrowest domain that owns the behavior. Create an external integration-test target only for a true external crate boundary.

## Fixtures

- `rivergate_registry_for_test` — shared immutable registry.
- `make_test_campaign` — isolated clone of the deterministic default campaign.
- `make_test_campaign_with` — variant where seed, name, or background is the contract.

Select fixture data semantically: prefer "property owned by the player and not pledged" over a hard-coded ID. Extract setup helpers for reusable domain conditions; extract assertion helpers only when reuse improves clarity, marking them `#[track_caller]`.

## Assertion standards

Assert public behavior, durable state, accounting, or explicit invariants.

- Exact values for accounting, arithmetic, schemas, serialization, and ordering when order is a contract.
- Relational assertions (`assert_in_range`, `>=`, `>`) for intentionally flexible emergent behavior — do not pin emergent totals to incidental prose or IDs.
- Set comparison with `assert_set_eq` for exhaustive enum/route coverage.
- Assert preconditions when a test could otherwise pass vacuously.
- Typed error variants and fields over formatted text.
- For successful commands, durable state and typed feedback categories over prose unless text is the contract.
- History: prove the command added the event via count delta or newly appended typed record.
- `assert_state_unchanged` for rejected mutations and stale-token commits; `assert_state_eq` when full equality is the contract.
- Derive expected values from arranged state when fixture details are not the contract.

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

Schema changes additionally require a schema increment, rejection tests for non-current schemas, current-schema round-trip equality, invalid-state rejection, and atomic-write tests when write behavior changes.

Harness changes should cover candidate discoverability, classification, pacing, consequence attribution, resilience, progression, and report semantics. Keep finding-rule tests cheap; long-horizon behavior belongs in harness or release tiers. See `GAMEPLAY_HARNESS.md`.

## Failure diagnostics

A useful failure identifies the violated contract, expected and observed values, relevant entity IDs/state, and the first differing path when state should remain equal. Collection helpers show observed members; candidate and finding helpers show available candidates or finding titles.

## Policy gates

No GitHub Actions are used; `python ../tools/check_no_github_actions.py` must pass. Document contracts must stay consistent (`python scripts/check_docs.py` via `docs` lane). Portfolio structural checks via `python ../tools/check_standards.py` should pass after portfolio-standard changes.

## Completion gate

While editing, run the narrowest relevant subset. Once behavior is ready, run one routine lane:

```bash
bash scripts/test.sh check            # syntax (cached)
bash scripts/test.sh fast simulation  # one domain after a lib edit
bash scripts/test.sh changed          # auto-detect, same budget as fast <filter>
bash scripts/test.sh fast             # full library
bash scripts/test.sh playtest         # harness smoke (60 days, debug)
bash scripts/test.sh standard         # pre-commit (lib + docs + CLI)
```

Specialized lanes by contract:

- `soak` — long-horizon and determinism
- `adapters` — CLI surfaces
- `gameplay` / `gameplay-audit` — harness report and design evaluation
- `docs` — documentation infrastructure
- `slow` / `ci-gates` / `all` / `deep` — release or broadly shared build config

If `fast <filter>` already covered the changed surface and `standard` is green, the work is complete. Do not escalate to `all`/`slow`/`ci-gates` for a typical feature.

Persistence, public APIs, command schemas, simulation order, arithmetic, invariants, shared state, and report schemas require focused owner coverage plus the relevant specialized lane. When the selected lane already executes that coverage, do not rerun a focused test beforehand. Do not run a compile-only build immediately before an executable lane that recompiles the same surface unless the separate diagnostic is required — prefer one build per checkpoint.

Filtered `fast <filter>` and `changed` avoid running unrelated domains (they still trigger a single-crate
incremental compile but exec only the matched subset). `changed` maps `git diff HEAD` to the
narrowest filter in one build; `Cargo.toml`/`.cargo` changes trigger the full suite. Docs-only edits
run `docs` alone. `CIVIC_DYNASTY_SKIP_CLI_BUILD=1` / `CIVIC_DYNASTY_SKIP_DOCS=1` skip the debug CLI/docs build
for lib-only iteration. `playtest` without args is a 60-day single-persona debug check (trace-limit 8);
pass explicit `--days`/`--persona` or `CIVIC_DYNASTY_PROFILE=release` only when probing deeper design
questions. The `all` tier reuses one debug CLI build.
