# Testing

The test suite is divided by feedback speed and purpose.

## Commands

```bash
bash scripts/test.sh fast
bash scripts/test.sh fast contracts
bash scripts/test.sh list laws
bash scripts/test.sh exact systems::strategic::tests::contracts::rejects_zero_week_duration
bash scripts/test.sh soak
bash scripts/test.sh cli
bash scripts/test.sh all
cargo run --release --locked -- playtest --days 360
```

`fast` runs only non-ignored library tests. It intentionally excludes documentation tests, the CLI binary, and long simulations so ordinary edit-test cycles remain short. A substring filter can select a subsystem or behavior.

`exact` runs one fully qualified test name, including an ignored soak test when selected explicitly. Use `list` to discover names. Test modules are grouped by domain, so filters such as `contracts`, `loans`, `laws`, `crises`, `migrations`, and `validation` are stable and meaningful.

`soak` runs the ignored 3,000-day and 7,200-day deterministic simulations serially. `cli` runs the command-line smoke suite. `all` runs shell syntax checks, library tests, documentation tests, soak tests, and CLI smoke tests.

The gameplay harness complements assertion-based tests by making state-derived player decisions through the canonical command API. It measures command reachability, blocked choices, causal command-to-system edges, durable feedback, action concentration, and campaign survival. Use a focused one-persona run while iterating and broader release-mode runs for design review. See `GAMEPLAY_HARNESS.md` for its counterfactual attribution model and report fields.

## Layout

Large suites live beside, but not inside, their production modules:

- `src/systems/bootstrap_tests.rs`
- `src/systems/commands_tests.rs`
- `src/systems/simulation_tests.rs`
- `src/systems/strategic_tests.rs`
- `src/gameplay_tests.rs`
- `src/persistence_tests.rs`
- `src/projection_tests.rs`
- `src/core/state_tests.rs`
- `src/test_support.rs`

Smaller module-local suites remain in their source files. Shared fixtures use one immutable registry and create fresh campaign state for each case.

## Test design rules

- Name tests as behavior statements under a domain module.
- Arrange only the state required for the behavior under test.
- Select records by semantic properties, not incidental map order, whenever identity matters.
- Verify a test precondition when a missing precondition could make the assertion pass vacuously.
- Assert public or durable behavior rather than implementation sequencing unless sequencing is the contract.
- Use exact values for accounting, migration, and boundary rules. Use relational assertions for intentionally flexible emergent behavior.
- Use `assert_state_unchanged` for rejected mutations and stale-token commits.
- Keep ordinary tests deterministic, independent, and free of sleeps or external services.
- Put expensive multi-year coverage in the ignored soak tier.

Failure output should identify the behavior, expected result, and first differing state path. The shared state assertion truncates unusually large JSON values so failures remain readable.
