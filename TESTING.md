# Testing

This document defines test tiers, suite layout, assertion standards, and completion gates.

## Commands

| Goal | Command |
|---|---|
| Run one domain or behavior | `bash scripts/test.sh fast <filter>` |
| Run all ordinary library tests | `bash scripts/test.sh fast` |
| List matching tests | `bash scripts/test.sh list <filter>` |
| Run one exact test | `bash scripts/test.sh exact <fully-qualified-name>` |
| Run one exact test with output | `bash scripts/test.sh debug <fully-qualified-name>` |
| Run long deterministic scenarios | `bash scripts/test.sh soak` |
| Check documentation contracts and doctests | `bash scripts/test.sh docs` |
| Run CLI smoke coverage | `bash scripts/test.sh cli` |
| Run the scripted test gate | `bash scripts/test.sh all` |

Examples:

```bash
bash scripts/test.sh fast contracts
bash scripts/test.sh fast gameplay::tests::candidates
bash scripts/test.sh list liquidation
bash scripts/test.sh exact systems::strategic::tests::loans::aged_defaulted_credit_is_restructured_in_place
bash scripts/test.sh debug gameplay::tests::harness::candidate_scenarios_cover_every_command_family
```

Successful `fast` and `exact` runs print a concise summary. Failures print complete Cargo output. A filter that matches no executable test returns exit code 2.

## Test tiers

### Fast library tests

Use for normal edit-test cycles. This tier excludes documentation tests, the CLI binary, and ignored soak tests.

Tests in this tier should be deterministic, isolated, and quick after compilation. They must not use sleeps, wall-clock time, external services, or environment-dependent behavior.

### Soak tests

Use for behavior that accumulates across long simulation horizons. Soak tests are deterministic and ignored by the fast tier.

Current soak coverage exercises core invariants and strategic multi-generation behavior. Run soak tests serially through `scripts/test.sh`.

### CLI smoke tests

Use for the external command-line contract. `scripts/verify_cli.sh` covers campaign creation, simulation, command execution, summary, projection, dashboard rendering, validation, gameplay output, sprite review rendering and reporting, quality-gate failure, and rejected input.

### Visual art review

Use `cargo run --locked -- art` for sprite work. Rendering is deterministic, so every stage is testable in the fast tier: ramps, primitives, rig resolution, clip sampling, encoding, and the automated review checks. The generated HTML sheet answers questions the checks cannot: weight, readability, and whether a pose reads at one-to-one scale.

Art tests must cover determinism for any new specification, primitive, rig, or clip, and any new defect class must gain a check in `src/art/lint.rs` rather than a comment.

### Gameplay analysis

Use the release-mode gameplay harness for command reachability, pacing, delayed consequences, strategic variety, resilience, and multi-generation behavior. The harness complements assertion-based tests; it does not replace them.

See `GAMEPLAY_HARNESS.md`.

## Suite layout

Large suites live beside production modules and are loaded by path:

| Area | Test file |
|---|---|
| Core state and stores | `src/core/state_tests.rs` |
| Campaign bootstrap | `src/systems/bootstrap_tests.rs` |
| Player commands | `src/systems/commands_tests.rs` |
| Daily simulation | `src/systems/simulation_tests.rs` |
| Strategic systems | `src/systems/strategic_tests.rs` |
| Persistence and migrations | `src/persistence_tests.rs` |
| Projections and HTML | `src/projection_tests.rs` |
| Gameplay harness | `src/gameplay_tests.rs` |
| Procedural art and sprite review | `src/art/*` module tests |
| Shared fixtures and diagnostics | `src/test_support.rs` |

Use stable nested modules so filters remain useful. Add a test to the narrowest domain that owns the behavior.

Create an external integration-test target only when the behavior requires a true external crate boundary.

## Fixtures

`rivergate_registry_for_test` returns the shared immutable registry.

`make_test_campaign` returns an isolated clone of the deterministic default campaign. Use `make_test_campaign_with` when seed, name, or starting background is part of the test.

Prefer semantic fixture selection over incidental collection position. For example, select a property by owner and collateral state rather than assuming a particular ID or index.

Extract setup helpers when they express a reusable domain condition. Keep assertion logic in the test unless it is shared and improves diagnostics.

## Assertion standards

Tests should assert public behavior, durable state, accounting, or explicit invariants.

- Use exact values for accounting, serialization, migrations, arithmetic boundaries, schema contracts, and ordered output when order is part of the contract.
- Use relational assertions for intentionally flexible emergent behavior.
- Compare sets for exhaustive enum or route coverage. Report missing and unexpected members separately.
- Assert preconditions when a test could otherwise pass vacuously.
- Prefer typed error variants and fields over formatted error text.
- Use `assert_state_unchanged` for rejected mutations and stale-token commits.
- Use `assert_state_eq` when full-state equality is the contract and first-difference diagnostics are useful.
- Add context to assertions whose failure would otherwise be ambiguous.
- Mark shared assertion helpers with `#[track_caller]`.

Avoid assertions against incidental fixture values, generated prose, internal call order, or unrelated state. Derive expected values from the arranged state when the fixture detail is not itself the contract.

## Consequential mutation coverage

A consequential mutation normally requires:

1. A successful state transition.
2. A rejected precondition with unchanged state.
3. Arithmetic, range, capacity, or lifecycle boundary coverage.
4. A stale-token test when validation and commit are separate.
5. Persistence coverage when serialized state changes.
6. Deterministic replay when ordering or randomness matters.
7. Invariant coverage for every new cross-record requirement.
8. Projection or durable-feedback coverage when the result must be observable.

## Persistence coverage

A save-schema change requires:

- A schema version increment
- One deterministic migration from the previous version
- Serialized migration input or an equivalent fixture
- Exact post-migration assertions
- Current-schema round-trip equality
- Invalid-state rejection tests
- Atomic replacement coverage when write behavior changes

Do not rely on debug assertions alone. Persistence validation is the release-mode boundary contract.

## Gameplay-harness coverage

Update harness tests when a change affects:

- Candidate discoverability or viability
- Command classification
- Strategic pacing or cooldowns
- Immediate, persistent, or delayed consequences
- Cross-domain interactions
- Business, food, labor, credit, crisis, or notification resilience
- Succession and multi-generation progression
- Structured report fields or semantics

Run focused one-persona reports while iterating and broader release-mode matrices for design review.

## Failure diagnostics

A useful failure identifies:

- The behavior under test
- Expected and observed values
- Relevant entity IDs or state
- The first differing path when state should remain equal

Collection helpers should include observed values. Candidate and finding helpers should include the available candidates or finding titles.

## Completion gate

Run the narrowest relevant subset while editing. Before finishing a cross-cutting change, run:

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

A cross-cutting change includes persistence, public APIs, command schemas, simulation order, arithmetic, invariants, shared state, and gameplay-report schemas.
