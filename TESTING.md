# Testing

This document defines the test tiers, layout, and expected coverage for code changes.

## Test tiers

| Tier | Command | Use |
|---|---|---|
| Focused fast | `bash scripts/test.sh fast <filter>` | Iterate on one domain or behavior. |
| All fast | `bash scripts/test.sh fast` | Run all non-ignored library tests. |
| List | `bash scripts/test.sh list <filter>` | Discover fully qualified test names. |
| Exact | `bash scripts/test.sh exact <name>` | Run one exact test, including an ignored soak test. |
| Soak | `bash scripts/test.sh soak` | Run deterministic long-horizon invariant tests. |
| CLI | `bash scripts/test.sh cli` | Run end-to-end command-line smoke coverage. |
| Complete scripted gate | `bash scripts/test.sh all` | Run shell checks, library tests, doctests, soak tests, and CLI checks. |
| Gameplay analysis | `cargo run --release --locked -- playtest ...` | Evaluate reachability, consequences, and campaign behavior. |

Examples:

```bash
bash scripts/test.sh fast contracts
bash scripts/test.sh fast persistence
bash scripts/test.sh fast gameplay::tests::candidates
bash scripts/test.sh fast gameplay::tests::findings
bash scripts/test.sh list labor
bash scripts/test.sh exact systems::strategic::tests::contracts::rejects_zero_week_duration
bash scripts/test.sh soak
bash scripts/test.sh all
```

`fast` excludes documentation tests, the CLI binary, and ignored long simulations. Use it for ordinary edit-test cycles. Filtered and exact modes invoke Cargo once and fail with exit code 2 when no executable test matches, including filters that select only ignored tests.

`exact` accepts a fully qualified test name and runs the selected test even when it is ignored by default.

`soak` runs the canonical 3,000-day core invariant scenario and the 7,200-day strategic multi-generation scenario serially.

`cli` verifies campaign creation, simulation, command execution, summary, inspection, dashboard generation, validation, gameplay output, and rejected input.

## Full completion gate

Run this before finishing a cross-cutting change:

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

Use the narrower relevant subset for isolated documentation or local implementation changes, but do not omit the full gate when changing persistence, commands, simulation order, arithmetic, invariants, or public APIs.

## Test layout

Large suites live beside their production modules and are loaded with `#[path]`:

| Production area | Test file |
|---|---|
| Core state and long-horizon determinism | `src/core/state_tests.rs` |
| Campaign bootstrap | `src/systems/bootstrap_tests.rs` |
| Player commands | `src/systems/commands_tests.rs` |
| Daily simulation | `src/systems/simulation_tests.rs` |
| Strategic systems | `src/systems/strategic_tests.rs` |
| Persistence and migrations | `src/persistence_tests.rs` |
| Projections and HTML | `src/projection_tests.rs` |
| Gameplay harness | `src/gameplay_tests.rs` |
| Shared fixtures and diagnostics | `src/test_support.rs` |

Small suites may remain in a local `#[cfg(test)] mod tests` at the bottom of the production file.

Do not create a separate integration-test tree unless the test requires a true external crate boundary that cannot be exercised through the existing module structure.

Large suites use stable domain modules so filters remain useful. Gameplay tests are grouped under `harness`, `candidates`, `metrics`, and `findings`. Add new tests to the narrowest applicable domain rather than leaving them at the suite root.

The shared Rivergate registry and default campaign baseline are initialized once. `make_test_campaign` returns an isolated clone of that deterministic baseline. Use `make_test_campaign_with` when a test requires a non-default seed, name, or starting background.

## Test design rules

- Name tests as behavior statements without a redundant `test_` prefix.
- Arrange only the state required for the behavior under test.
- Use fresh campaign state for each test.
- Use the shared immutable Rivergate registry fixture when applicable.
- Select records by semantic properties rather than incidental collection position.
- Assert preconditions when a missing precondition could make the test pass vacuously.
- Test through canonical command, transaction, simulation, persistence, or projection paths.
- Assert durable or public behavior rather than private sequencing unless sequencing is the contract.
- Use exact values for accounting, migration, serialization, and arithmetic boundaries.
- Use exact counts and ordering only when cardinality or order is part of the contract.
- Use relational assertions only for intentionally flexible emergent behavior.
- Prefer typed error variants and fields over matching formatted error text.
- When a helper asserts collection cardinality, include the observed values in its failure output.
- Mark shared assertion helpers with `#[track_caller]` so failures point to the test call site.
- Use `assert_state_unchanged` for rejected mutations and stale-token commits.
- Keep ordinary tests deterministic and free of sleeps, wall-clock dependencies, and external services.
- Put expensive accumulation and generation coverage in the ignored soak tier.

## Required mutation coverage

A consequential mutation should normally include:

1. A successful transition test.
2. A rejected-precondition test that proves state remains unchanged.
3. Arithmetic, capacity, or lifecycle boundary coverage.
4. A stale-token test when validation and commit are separated.
5. Persistence round-trip or migration coverage when state is serialized.
6. Deterministic replay coverage when ordering or randomness is involved.
7. Invariant coverage for every new cross-record requirement.

## Persistence coverage

A save-schema change requires:

- A schema version increment.
- A deterministic migration from the previous version.
- A migration fixture or equivalent serialized input.
- Exact post-migration assertions.
- Current-schema round-trip equality.
- Invalid-state rejection tests.
- Atomic save replacement coverage when write behavior changes.

## Gameplay harness coverage

The gameplay harness complements assertion-based tests. It does not replace them.

Use it when a change affects:

- Command discoverability or candidate quality
- Strategic pacing or cooldowns
- Delayed consequences
- Cross-domain interaction
- Business, food, labor, crisis, or notification resilience
- Multi-generation progression

Run focused one-persona reports while iterating and broader release-mode matrices for design review. See `GAMEPLAY_HARNESS.md`.

## Failure diagnostics

A useful failure identifies:

- The behavior under test
- The expected result
- The observed result
- The first differing state path when state should remain equal

The shared state-difference assertion truncates very large JSON values so failure output remains readable.

Candidate and finding helpers report the complete observed candidate set or available finding titles when an expectation fails. Maintain that diagnostic quality when adding new reusable assertions.
