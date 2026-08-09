# Agent Guide

This file defines how to change the repository safely. It is an execution contract, not a product specification.

Read `README.md` first. Use `ARCHITECTURE.md` for ownership and flow, `DESIGN.md` for product intent, `STATUS.md` for current capability, and `TESTING.md` for validation.

## Cold start

Before editing:

1. Run `git status --short` and preserve unrelated changes.
2. Identify the owning module and sibling test suite.
3. Read the relevant architecture and status sections.
4. Trace the canonical path from public entry point to mutation and invariant validation.
5. Run the narrowest relevant test.

Primary entry points:

| Concern | Entry point |
|---|---|
| New campaign | `build_new_game` in `src/systems/bootstrap.rs` |
| Player command | `apply_player_command` in `src/systems/commands.rs` |
| Time advancement | `advance_days` in `src/systems/simulation.rs` |
| Scheduled systems | `src/systems/strategic.rs` |
| Persistence | `src/persistence.rs` |
| Read models and HTML | `src/projection.rs` |
| Gameplay analysis | `src/gameplay.rs` |

## Core rules

### Ownership

- `Registry` owns immutable scenario definitions.
- `AppState` owns mutable, serializable campaign state.
- Records own identity, references, local values, and lifecycle state.
- Systems own validation and mutation.
- Adapters translate external input and output only.

Do not place mutable runtime state in the registry. Do not place business rules in CLI, persistence, projections, rendering, tests, or gameplay-report formatting.

### Canonical mutation

Every operation follows one path:

```text
input -> validation -> resolution -> atomic commit -> durable feedback -> invariant check
```

CLI commands, AI, migrations, tests, and administrative utilities must not create parallel mutation semantics.

Validate every reference, ownership claim, permission, lifecycle state, range, capacity, and arithmetic result before mutation. A failed operation must leave state unchanged.

For multi-record operations, calculate all resulting values before committing any change. Use a consumed `Validated*` token when validation and commit are separated; commit must revalidate current state.

### Determinism

The same registry, state, seed, command sequence, and day count must produce identical state.

- Use the state-owned RNG for simulation randomness.
- Use ordered collections, explicit sorting, and typed-ID tie-breakers for result-affecting iteration.
- Do not use wall-clock time, environment-dependent values, unordered filesystem enumeration, sleeps, or external services in core systems.

### Arithmetic

Economic state uses `Money` and `Quantity` from `src/money.rs`.

- Do not use floating point for economic state.
- Use shared cost, affordability, and ratio helpers.
- Use wide intermediates for multiply-then-divide calculations.
- Reject overflow before player-initiated and cross-record transfers.
- Saturation does not replace a domain bound check.

### Identity and references

- Persistent references use typed IDs from `src/ids.rs`.
- Generated records use `NextIds` owned by `AppState`.
- Optional references use `Option`.
- Raw strings are for authored keys and user-facing text, not runtime identity.
- Save validation must reject missing references, stale indexes, duplicate ownership, and allocator
  sentinel values. The terminal valid allocator counter is serializable; its next allocation must
  fail atomically with `IdentifierAllocationError` rather than wrap.

### Derived state and lifecycle

Derived indexes and counters have one authoritative source. Update source records and every owned derived structure in the same atomic operation.

Related records must agree on lifecycle. Closure, default, repayment, sale, office turnover, employment change, occupancy change, and succession must update all dependent records together.

### Boundaries

Core systems perform no implicit IO. Persistence, CLI, projection, rendering, and gameplay reporting are boundary layers. Durable external work must be represented in state before an adapter acts on it.

## Change procedures

### Add persistent state

1. Add or extend a record in `src/core/records.rs` or `src/core/extended.rs`.
2. Add ownership to `AppState` or an existing synchronized store.
3. Add typed IDs and allocation if required.
4. Update bootstrap or migration construction.
5. Update persistence validation and runtime invariants.
6. Expose the state through projections when users or adapters need it.
7. Add behavioral, invalid-state, and round-trip tests.
8. Increment the save schema when serialized compatibility changes.

### Add a player command

1. Add an exhaustive `PlayerCommand` variant.
2. Add typed errors for each precondition class.
3. Validate the complete operation before mutation.
4. Commit through `apply_player_command` or a canonical subsystem function.
5. Add durable audit, chronicle, or outbox feedback when consequential.
6. Add success, rejection, atomicity, serialization, and boundary tests.
7. Update projections if the result must be observable.
8. Update gameplay candidates, classification, snapshots, attribution, and coverage.
9. Update CLI smoke coverage when syntax or output changes.

### Add simulation behavior

1. Assign an explicit cadence: daily, weekly, monthly, or annual.
2. Use `simulation.rs` for the daily economic pipeline and `strategic.rs` for scheduled cross-domain systems.
3. Keep read-only planning separate from narrow mutation when useful.
4. Preserve the documented execution order or update it with causal tests.
5. Add durable feedback for player-relevant delayed outcomes.
6. Add invariants and focused tests; add soak coverage for accumulating behavior.

### Change persistence

1. Increment `CURRENT_SCHEMA_VERSION` for serialized contract changes.
2. Add one deterministic migration from the previous version.
3. Keep migrations explicit and version-by-version.
4. Add migration input, exact post-migration assertions, and current-schema round-trip coverage.
5. Preserve release-mode validation and atomic same-directory replacement.
6. Update `STATUS.md`.

### Extend projections or gameplay reports

Projection code reads immutable state and may format or aggregate. It must not recreate domain rules.

For gameplay-report changes, update candidate generation, command classification, snapshots, attribution, findings, traces, schema version, tests, and `GAMEPLAY_HARNESS.md` together.

## Code conventions

- Match project-owned enums exhaustively; do not use wildcard arms.
- Keep consequential fields private and mutate them through systems.
- Prefer concrete records and functions over dynamic dispatch in core domains.
- Keep top-level execution order visible.
- Pass the narrowest mutable context required.
- Delete replaced paths instead of retaining unused compatibility layers.
- Give every source file a concise `//!` module description.
- Use dedicated error enums with contextual variants; do not return stringly typed errors.

Naming:

| Intent | Prefix or form |
|---|---|
| Keyed lookup | `get_*` |
| Conditional scan | `find_*` |
| Final derivation | `resolve_*` |
| Aggregate construction | `build_*` |
| Collection insertion | `insert_*` |
| Registry definition insertion | `register_*` |
| Removal | `remove_*` |
| Predicate | `is_*`, `has_*`, `can_*` |
| Read-only decision | `decide_*` |
| Apply decided mutation | `apply_*` |
| Deferred precondition check | `validate_*` returning `Validated*` |

Test fixtures may use `make_test_*`. Avoid new `create_*`, `execute_*`, `perform_*`, or `attempt_*` names unless required by an external interface.

## Tests and documentation

Use sibling `*_tests.rs` files for large suites. Test canonical behavior, not helper reachability. Consequential mutations normally require success, rejection with unchanged state, arithmetic or capacity boundaries, stale-token coverage when applicable, persistence coverage when serialized, and deterministic replay when ordering or randomness matters.

`TESTING.md` is authoritative for commands and test design.

Update documentation in the same change when architecture, behavior, commands, schemas, public APIs, test workflows, or deliberate scope changes. Keep documentation current-state and forward-facing. Do not add implementation diaries, repair histories, or dated verification narratives.

## Completion gate

Run focused checks during development. Before finishing cross-cutting work, run:

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

Confirm that failed operations remain atomic, deterministic ordering is explicit, persistent state is complete and migratable, public consequences are observable, and documentation matches the resulting code.
