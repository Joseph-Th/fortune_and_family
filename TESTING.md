# Testing

This document defines test tiers, suite organization, assertion standards, and completion gates.

## Commands

| Goal | Command |
|---|---|
| One domain or behavior | `bash scripts/test.sh fast <filter>` |
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
| Release gameplay gates | `bash scripts/test.sh gameplay` |
| Deep gameplay design audit | `bash scripts/test.sh gameplay-audit` |
| Fast CI verification lane | `bash scripts/test.sh ci-verify` |
| Deep CI gates lane | `bash scripts/test.sh ci-gates` |
| Complete scripted test tier | `bash scripts/test.sh all` |

Successful test and smoke steps print concise summaries with elapsed time. Failures print the complete command output, including compiler diagnostics when a CLI build fails. A filter matching no executable library test exits with code 2.

The CLI smoke runner accepts `CIVIC_DYNASTY_PROFILE=release` when a release
binary is desired, or `CIVIC_DYNASTY_BINARY` when a caller has already built
the exact binary to exercise. CI uses the latter so adapter and gameplay
gates share one release build instead of compiling a debug CLI first.
`CIVIC_DYNASTY_BINARY_OVERRIDE` takes precedence over every profile choice and
is intended for tooling that must pin an exact binary; the gameplay gates
always rebuild the release CLI when a debug binary selected by an earlier
smoke group would otherwise leak into the optimized gate run.
The `adapters` and `gameplay` modes also build their local CLI once and reuse
it across all sub-gates. Python-backed checks use `python3`, `python`, or
Windows `py` automatically; set `CIVIC_DYNASTY_PYTHON` to select an explicit
interpreter.
Long-running gameplay JSON assertions are centralized in
`scripts/check_gameplay.py`, which derives expected persona coverage from the
report configuration and prints a compact validation summary.
The harness parallelizes independent counterfactual probes for a single
campaign with a bounded worker count; campaign matrices parallelize campaigns
instead to keep memory use predictable. Probe results retain deterministic
candidate ordering.

## Test tiers

| Tier | Purpose | Expected use |
|---|---|---|
| Fast library | Deterministic unit and focused behavioral coverage | Normal edit-test cycle |
| Standard | Syntax, fast library, docs, core CLI | Normal pre-commit |
| Adapter smoke | External CLI contracts grouped by core, art, and gameplay | Adapter changes |
| Soak | Long deterministic invariant and multi-generation behavior | Accumulating simulation changes |
| Gameplay | Release-mode systemic quality and succession gates | Cross-domain gameplay changes |
| Gameplay audit | Larger matrices for rare and mature behavior | Design review |
| CI verify | The exact fast CI verification lane | Reproducing the required PR checks locally |
| CI gates | The exact deep CI lane; requires `cargo-audit` | Reproducing release, adapter, gameplay, and security gates |
| All | Standard + soak + adapters + gameplay gates | Cross-cutting test coverage |

Fast tests must not use sleeps, wall-clock time, external services, or environment-dependent behavior.

## Suite organization

Large suites live beside their production owner:

| Area | Test file |
|---|---|
| Core state and stores | `src/core/state_tests.rs` |
| Campaign bootstrap | `src/systems/bootstrap_tests.rs` |
| Player commands | `src/systems/commands_tests.rs` |
| Daily simulation | `src/systems/simulation_tests.rs` |
| Strategic systems | `src/systems/strategic_tests.rs` |
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

Select fixture data semantically. Prefer “property owned by the player and not pledged” over a hard-coded ID or collection position.

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

Run the narrowest relevant subset while editing.

For ordinary changes:

```bash
bash scripts/test.sh standard
```

For cross-cutting changes, run:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features --locked
bash scripts/test.sh all
cargo clippy --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
cargo test --release --quiet --locked --lib
cargo audit
git diff --check
```

Cross-cutting changes include persistence, public APIs, command schemas, simulation order, arithmetic, invariants, shared state, and gameplay-report schemas.

The commands above are the authoritative local full gate for cross-cutting changes. Ordinary changes use `bash scripts/test.sh standard`; cross-cutting changes run the complete command set above. GitHub Actions and hosted runners are not part of the verification path.

Clippy is intentionally the single all-target compile/lint gate in the full validation path; an additional duplicate compile pass should not be added without coverage value. The local runner owns the scripted lanes so focused and complete reproduction use the same commands. The `all` tier also reuses one debug CLI build across all adapter smoke groups.
