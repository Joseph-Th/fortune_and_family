# Testing

Defines test tiers, suite organization, assertion standards, and completion gates.

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
| Focused harness run | `bash scripts/test.sh playtest [args...]` | <1s warm (debug) | Single campaign iteration |
| Release gameplay gates | `bash scripts/test.sh gameplay` | ~16s warm | 36 + 3 campaigns, 60k days (release) |
| Deep gameplay design audit | `bash scripts/test.sh gameplay-audit` | ~30s warm | Multi-seed / generation / credit stress (release) |
| Fast CI verification lane | `bash scripts/test.sh ci-verify` | ~6s warm | Format + clippy + lib + docs + doc warnings |
| Deep CI gates lane | `bash scripts/test.sh ci-gates` | ~1 min | Release + soaks + adapters + gameplay + audit |
| Heavy release gates without audit | `bash scripts/test.sh slow` | ~45s | Release gates without security audit |
| Full deep design gate | `bash scripts/test.sh deep` | ~1.2 min | slow + gameplay-audit |
| Complete scripted test tier | `bash scripts/test.sh all` | ~25s | Standard + soak + adapters + gameplay |

Failures print complete output including compiler diagnostics. A filter matching no executable test exits with code 2.

On Windows without bash on PATH, use `.\scripts\test.ps1 <mode> [filter]` (mirrors `scripts/test.sh`).

## Runner environment

| Variable | Effect |
|---|---|
| `CIVIC_DYNASTY_JOBS=<n>` | Forwards `--jobs <n>` to cargo and caps harness campaign parallelism. |
| `CIVIC_DYNASTY_NEXTEST=1` | Runs library tests under `cargo-nextest` (per-test isolation). |
| `CIVIC_DYNASTY_SKIP_CLI_BUILD=1` | Skips CLI rebuilds when iterating on library code. |
| `CIVIC_DYNASTY_PROFILE=release` | Forces `adapters`/`playtest` to use a release binary. Gate lanes always use release. |
| `CIVIC_DYNASTY_BINARY=<path>` | Reuses an existing binary for smoke groups. |
| `CIVIC_DYNASTY_BINARY_OVERRIDE=<path>` | Pins an exact binary over every profile choice. |
| `CIVIC_DYNASTY_PRE_PUSH=standard` | Strengthens the pre-push hook from its `quick` default. |
| `CIVIC_DYNASTY_PYTHON=<interpreter>` | Selects the Python interpreter. |

## Build profiles

- `dev` / `test`: crate `opt-level = 1`, dependencies `2`. Simulation-heavy tests finish in seconds. 16 codegen units, incremental compilation.
- `check`: inherits `dev`, never executed; used by `cargo check` for sub-second feedback.
- `release`: soaks and gameplay gates — `opt-level = 3`, 16 codegen units, incremental, no LTO. Warm rebuild stays ~1s; throughput within ~10% of peak.
- `release-max`: single codegen unit + thin-LTO. Use only when measuring peak throughput.

Shared tuning: `.cargo/config.toml` (`incremental = true`, `pipelining = true`, `jobs = 0`) and `Cargo.toml` (16 codegen units per profile). No remote cache required.

Warm budgets: `fast` ~2s, `standard` ~4s, `ci-verify` ~6s, debug `playtest` <1s, `gameplay` ~16s (one release build + 39 campaigns, 60k days). Each lane reuses the incremental cache.

Long gameplay JSON assertions live in `scripts/check_gameplay.py`. `adapters`/`gameplay` lanes build their CLI once and reuse it.

## Receipts and hooks

Successful unfiltered `quick`/`fast`, `standard`, and `all` runs record a content-addressed receipt under local Git metadata. The pre-push hook reuses a current receipt of equal or broader strength instead of recompiling identical bytes. Any tracked or non-ignored change invalidates it; receipt-eligible lanes refuse to issue evidence if bytes change mid-run.

Install optional hooks with `bash scripts/install_hooks.sh`: pre-commit runs format, shell syntax, and whitespace checks; pre-push defaults to `quick` (2s warm). Use `git commit --no-verify` during focused iteration and `CIVIC_DYNASTY_PRE_PUSH=standard` when a fuller gate is needed on push.

## Test tiers

| Tier | Purpose | Warm budget |
|---|---|---|
| Check | Syntax/type only | ~1s |
| Fast library | Deterministic unit and focused behavioral coverage | ~2s |
| Standard | Check + fast library + docs + core CLI | ~4s |
| Adapter smoke | CLI contracts grouped by core, art, gameplay | ~2s |
| Soak | Long deterministic invariant and multi-generation behavior | ~1s warm |
| Gameplay | Release-mode systemic quality and succession gates | ~16s |
| Gameplay audit | Larger matrices for rare and mature behavior | ~30s |
| CI verify | Fast CI lane | ~6s warm |
| CI gates | Deep CI lane; requires `cargo-audit` | ~1 min |
| Slow | Release gates without audit | ~45s |
| Deep | Complete design gate: slow + gameplay audit | ~1.2 min |
| All | Standard + soak + adapters + gameplay | ~25s |

Only `fast`/`quick`/`check` belong in the inner loop. `ci-gates`, `slow`, `deep`, and `all` are release or deep-design checkpoints.

Fast tests must not use sleeps, wall-clock time, external services, or environment-dependent behavior. Soak tests always run in release mode (debug would be ~100× slower for long horizons); assertions are identical across profiles. Use `check` when only syntax needs proof.

## Suite organization

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

- `rivergate_registry_for_test` — shared immutable registry.
- `make_test_campaign` — isolated clone of the deterministic default campaign.
- `make_test_campaign_with` — variant where seed, name, or background is part of the contract.

Select fixture data semantically: prefer "property owned by the player and not pledged" over a hard-coded ID or position.

Extract setup helpers when they describe reusable domain conditions. Extract assertion helpers only when reuse improves clarity or diagnostics; mark shared assertion helpers `#[track_caller]`.

## Assertion standards

Assert public behavior, durable state, accounting, or explicit invariants.

- Use exact values for accounting, arithmetic boundaries, schemas, serialization, and ordering when order is a contract.
- Use relational assertions for intentionally flexible emergent behavior.
- Compare sets for exhaustive route or enum coverage and report missing/unexpected members.
- Assert preconditions when a test could otherwise pass vacuously.
- Prefer typed error variants and fields over formatted error text.
- For successful commands, assert durable state and typed feedback categories rather than prose unless text is the contract.
- When matching history, prove the command added the event with a count delta or newly appended typed record.
- Use `assert_state_unchanged` for rejected mutations and stale-token commits.
- Use `assert_state_eq` when full-state equality is the contract.
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

Save-schema changes additionally require a schema increment, rejection tests for non-current schemas, current-schema round-trip equality, invalid-state rejection, and atomic-write tests when write behavior changes.

Gameplay-harness changes should cover candidate discoverability, classification, pacing, consequence attribution, resilience metrics, progression, and report semantics. Keep finding-rule tests cheap; long-horizon behavior belongs in harness or release tiers. See `GAMEPLAY_HARNESS.md`.

## Failure diagnostics

A useful failure identifies the behavior, expected and observed values, relevant entity IDs/state, and the first differing path when state should remain equal.

Collection helpers should show observed members. Candidate and finding helpers should show available candidates or finding titles.

## Completion gate

Run the narrowest relevant subset while editing. Once behavior is ready, run one routine lane rather than climbing through broader tiers:

```bash
bash scripts/test.sh check          # ~1s syntax only
bash scripts/test.sh fast simulation # ~2s for the one domain you touched
bash scripts/test.sh standard        # ~4-5s normal pre-commit
```

Select specialized lanes by changed contract:

- `soak` — long-horizon simulation, determinism, invariant evidence
- `adapters` — CLI/adapter surfaces
- `gameplay` / `gameplay-audit` — gameplay report and design-evaluation contracts
- `docs` — documentation infrastructure
- `slow` — release-profile behavior that can differ from development
- `ci-gates`, `all`, `deep` — verification topology, dependency/security work, broadly shared build config, or a deliberate release checkpoint

If `fast <filter>` already covered the changed surface and `standard` is green, the work is complete. Do not escalate to `all`/`slow`/`ci-gates` for a typical feature.

Persistence, public APIs, command schemas, simulation order, arithmetic, invariants, shared state, and report schemas require focused owner coverage plus the relevant specialized lane above. This is a coverage requirement, not automatically two invocations: when the selected lane already executes the necessary owner coverage, do not rerun a focused test beforehand.

Do not run a compile-only or lint build immediately before an executable lane that recompiles the same surface unless the separate diagnostic is required. Prefer one build-producing operation per checkpoint.

The local runner owns the scripted lanes so focused and complete reproduction use the same commands. The `all` tier reuses one debug CLI build across adapter smoke groups and remains an explicit broad tier rather than a routine prerequisite.
