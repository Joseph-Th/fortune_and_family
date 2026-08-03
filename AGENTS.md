# Agent Guide

This file defines how to work in the repository. Product intent is in `DESIGN.md`; implementation structure is in `ARCHITECTURE.md`; current coverage is in `STATUS.md`.

## Cold-start sequence

Before editing:

1. Run `git status --short` and preserve unrelated working-tree changes.
2. Read `README.md`, `ARCHITECTURE.md`, and the relevant section of `STATUS.md`.
3. Locate the owning module and its sibling `*_tests.rs` file.
4. Run a focused test before changing behavior.
5. Trace the canonical path from public entry point to mutation and invariant validation.

For cross-domain work, start from these entry points:

- New campaign: `build_new_game` in `src/systems/bootstrap.rs`
- Player mutation: `apply_player_command` in `src/systems/commands.rs`
- Time advancement: `advance_days` in `src/systems/simulation.rs`
- Strategic cadence: `src/systems/strategic.rs`
- Save/load: `src/persistence.rs`
- Read-only output: `src/projection.rs`
- Gameplay evaluation: `src/gameplay.rs`

## Non-negotiable rules

### Explicit ownership

- `Registry` owns immutable definitions.
- `AppState` owns mutable, serializable campaign state.
- Records own identity and local lifecycle data.
- Systems own validation, decisions, mutation, and invariant preservation.
- Adapters own input/output translation only.

Do not place mutable runtime values in registry definitions. Do not place business rules in CLI, persistence, projection, rendering, or tests.

### One mutation path

Each operation has one canonical path:

```text
input -> validation -> resolution -> atomic commit -> durable feedback -> invariant check
```

Tests, migrations, AI, CLI commands, and administrative code must not create parallel mutation semantics.

### Validate before mutation

Validate all references, lifecycle states, permissions, ownership, capacities, ranges, and arithmetic results before changing state.

A failed operation leaves state unchanged. For multi-record operations, calculate every resulting value first and then commit all changes together.

Use a consumed `Validated*` token when validation and commit can be separated. The token must revalidate state at commit time.

### Determinism

The same registry, state, seed, input sequence, and day count must produce identical state.

- Use only the state-owned RNG for simulation randomness.
- Use `BTreeMap`, `BTreeSet`, explicit sorting, or another stable order for result-affecting iteration.
- Use typed IDs as stable tie-breakers.
- Do not use wall-clock time, unordered filesystem enumeration, environment-dependent values, sleeps, or external services in core systems.

### Fixed-point arithmetic

Economic values use `Money` and `Quantity` from `src/money.rs`.

- Do not introduce floating-point economic state.
- Use the shared cost, affordability, and ratio helpers.
- For multiply-then-divide calculations, use a wide intermediate and saturate only the final result.
- Reject overflow before player-initiated or cross-record transfers.
- Saturation is not a substitute for a domain bound check.

### References and IDs

- Persistent references use typed IDs from `src/ids.rs`.
- New generated records use `NextIds` owned by `AppState`.
- Load and save validation must reject missing references, stale indexes, and exhausted allocators.
- Optional references are represented with `Option`.
- Raw strings are for authored keys or user-facing text, not internal record identity.

### Derived data

Derived indexes and counters have one owner. Update source records and all owned derived structures atomically.

If a value is persisted, either make it authoritative or validate that it exactly matches its source data. Do not allow two independent sources of truth.

### Lifecycle consistency

Related records must agree about active, disputed, suspended, insolvent, closed, completed, defaulted, or resolved state.

A lifecycle change must update every dependent record in the same canonical operation. Examples include employment after business closure, collateral after repayment or default, occupancy after ownership transfer, and officeholding after elections.

### External boundaries

Core systems perform no implicit IO. Persistence, CLI, and rendering are adapters. Durable external work must be represented in state before an adapter acts on it.

## Change recipes

### Add persistent state

1. Define or extend a record in `src/core/records.rs` or `src/core/extended.rs`.
2. Add ownership to `AppState` or an existing synchronized store.
3. Add typed IDs and allocation when needed.
4. Add getters required by public read paths.
5. Update bootstrap or migration construction.
6. Update persistence validation.
7. Add debug invariants.
8. Add projection fields when the player or adapter must observe it.
9. Add round-trip, invalid-state, and behavioral tests.
10. Increment the save schema when serialized compatibility changes.

### Add a player command

1. Add an exhaustive `PlayerCommand` variant.
2. Add dedicated input fields and a dedicated error variant for each new precondition class.
3. Validate the full operation before mutation.
4. Commit through `apply_player_command` or a canonical owned subsystem function.
5. Add audit and player-facing feedback when the action is consequential.
6. Add rollback, success, serialization, and boundary tests.
7. Expose required state in projections.
8. Add gameplay-harness candidates, snapshots, attribution, and coverage.
9. Extend CLI smoke coverage when syntax or output changes.

### Add simulation behavior

1. Decide whether the behavior is daily, weekly, monthly, or annual.
2. Put it in `simulation.rs` for the daily economic pipeline or `strategic.rs` for scheduled cross-domain systems.
3. Separate broad read-only calculation from narrow mutation when useful.
4. Preserve the documented execution order or update it explicitly with causal tests.
5. Add durable feedback for player-relevant delayed outcomes.
6. Add invariants for every new cross-record requirement.
7. Add focused tests plus soak coverage when the behavior accumulates over time.

### Change persistence

1. Increment `CURRENT_SCHEMA_VERSION` for a serialized contract change.
2. Add a deterministic migration from the previous version.
3. Keep migrations explicit and version-by-version.
4. Add migration fixtures and exact post-migration assertions.
5. Validate loaded and saved state in release mode.
6. Preserve atomic same-directory replacement.
7. Update `STATUS.md` and public documentation.

### Add a projection or adapter field

Read from immutable registry/state data. Do not infer new business rules in the projection. Add coverage proving each primary record appears once and rendered output remains escaped and script-safe.

### Extend the gameplay harness

Update all relevant parts together:

- Candidate generation and ranking
- Command-family and strategic-direction classification
- Snapshot fields
- Immediate and delayed comparison
- Findings and scores
- Trace rendering
- Schema version
- Harness tests and `GAMEPLAY_HARNESS.md`

## Naming

- Direct keyed lookup: `get_*`
- Conditional scan: `find_*`
- Derive a final value: `resolve_*`
- Plain accessor: noun form such as `status()`
- Plain constructor: `new()`
- Aggregate construction: `build_*`
- Collection insertion: `insert_*`
- Registry definition insertion: `register_*`
- Removal from state: `remove_*`
- Boolean predicate: `is_*`, `has_*`, or `can_*`
- Read-only decision: `decide_*`, returning a `Plan`, `Outcome`, or `Delta`
- Mutation of a decided value: `apply_*`
- Preconditions for a deferred commit: `validate_*`, returning `Validated*`

Do not add new `create_*`, `make_*`, `execute_*`, `perform_*`, or `attempt_*` names unless an external API requires the term. Test fixtures use `make_test_*`.

## Structural rules

- Project-owned enum matches are exhaustive. Do not use wildcard arms.
- Consequential fields stay private and are changed through systems.
- Prefer concrete records and functions over dynamic dispatch in core domains.
- Large structs should group related fields into explicit profiles.
- Struct-to-struct mappings name every field or profile intentionally.
- Pass the narrowest mutable context a phase needs.
- Keep top-level execution order visible in one place.
- Delete replaced paths rather than preserving unused compatibility shims.
- Every source file has a concise `//!` module description.

Internal `expect` and assertions are acceptable only after the code has already established the invariant. External or persisted input must return a typed error rather than panic.

## Errors

New fallible operations use dedicated error enums with contextual variants. Do not add stringly typed `Result` errors. Callers should not need to parse error text to identify a failed precondition.

Error variants should include relevant IDs, available values, required values, or lifecycle states.

## Tests

Use sibling test modules for large suites:

- `src/systems/bootstrap_tests.rs`
- `src/systems/commands_tests.rs`
- `src/systems/simulation_tests.rs`
- `src/systems/strategic_tests.rs`
- `src/persistence_tests.rs`
- `src/projection_tests.rs`
- `src/gameplay_tests.rs`
- `src/core/state_tests.rs`

Test through canonical public or subsystem paths. Helper-only tests must not make unreachable production behavior appear integrated.

Required coverage for consequential mutations:

- Successful state transition
- Rejected precondition with unchanged state
- Arithmetic and capacity boundaries
- Stale validated token when applicable
- Serialization or migration when persistent
- Deterministic replay when ordering or randomness is involved

See `TESTING.md` for commands and tier selection.

## Documentation

Update documentation in the same change when behavior, architecture, commands, save schema, public API, test workflow, or deliberate scope changes.

Keep documents forward-facing:

- Describe the current contract and intended extension points.
- Do not maintain audit diaries, repair histories, or chronological implementation narratives.
- Put product intent in `DESIGN.md`.
- Put implementation structure in `ARCHITECTURE.md`.
- Put current coverage and known boundaries in `STATUS.md`.
- Put operational commands in `README.md`, `TESTING.md`, or `GAMEPLAY_HARNESS.md`.

## Completion gate

Run the narrowest relevant tests during development. Before finishing a cross-cutting change, run:

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

Confirm that:

- No unrelated working-tree changes were reverted.
- Failed operations remain atomic.
- Deterministic ordering is explicit.
- Persistent state is complete and migratable.
- New invariants are enforced in debug and persistence validation where applicable.
- Public behavior is observable through projections or durable feedback.
- Documentation matches the resulting code.
