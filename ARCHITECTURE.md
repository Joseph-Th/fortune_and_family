# Architecture

This document describes how the repository is organized and where behavior belongs. Read it before changing cross-cutting code.

## System shape

Civic Dynasty is a deterministic simulation kernel with explicit state and explicit mutation paths.

```text
immutable Registry definitions
            +
serializable AppState
            |
            v
canonical systems
  bootstrap | commands | simulation | strategic | transactions
            |
            +--> invariant validation
            +--> durable state, audit, chronicle, and outbox records
            |
            v
read-only projections and adapters
  persistence | CLI | JSON projection | HTML dashboard | gameplay harness
```

Dependency direction is one way:

1. Registry definitions and core records define data.
2. Systems read definitions and state, validate operations, and mutate state.
3. Persistence and presentation layers translate data at external boundaries.
4. Adapters never own business rules.

## Repository map

| Path | Responsibility |
|---|---|
| `src/core/records.rs` | Primary economic and population records. |
| `src/core/extended.rs` | Strategic records such as laws, loans, property, institutions, relationships, crises, and family state. |
| `src/core/state.rs` | `AppState`, synchronized stores, ID allocation, clock, and public read access. |
| `src/ids.rs` | Typed persistent identifiers. |
| `src/money.rs` | Fixed-point `Money`, `Quantity`, cost, affordability, and ratio arithmetic. |
| `src/registry/mod.rs` | Immutable Rivergate definitions and lookup validation. |
| `src/rng.rs` | Serializable deterministic random number generator. |
| `src/systems/bootstrap.rs` | New campaign construction and authored starting state. |
| `src/systems/commands.rs` | Public player command schema, validation, and command commits. |
| `src/systems/simulation.rs` | Canonical daily simulation pipeline and economic planning. |
| `src/systems/strategic.rs` | Weekly, monthly, annual, and cross-domain strategic systems. |
| `src/systems/transactions.rs` | Reusable validated transaction primitives and simulation errors. |
| `src/systems/invariants.rs` | Debug assertions for references, indexes, accounting, lifecycle, and derived state. |
| `src/persistence.rs` | Versioned JSON save/load, migrations, and release-mode state validation. |
| `src/projection.rs` | Read-only campaign projections and self-contained HTML rendering. |
| `src/gameplay.rs` | Deterministic player agents, counterfactual analysis, scores, traces, and findings. |
| `src/main.rs` | CLI parsing and adapter behavior. |
| `src/*_tests.rs` | Large domain-organized test suites loaded by their production modules. |
| `src/test_support.rs` | Deterministic fixtures and state-difference assertions. |

## Ownership model

### Registry

`Registry` owns immutable scenario definitions. Definitions answer what can exist and how authored content is configured. They are built once by `build_rivergate_registry`.

Registry data must not contain mutable campaign state. Runtime records refer to definitions through typed IDs.

### AppState

`AppState` owns every value required to resume deterministic execution:

- Clock, RNG state, schema version, and next-ID allocators.
- Dynasties, characters, households, businesses, and institutions.
- Market, contracts, loans, property, employment, laws, and relationships.
- Districts, public works, courts, routes, crises, information, and AI objectives.
- Outbox, chronicle, and audit history.

If a generated value affects future behavior, it belongs in `AppState` or in a record owned by `AppState`.

### Records

Records hold identity, references, local values, and lifecycle state. They expose read-only access publicly. Consequential mutation belongs in systems.

### Synchronized stores

Character, household, and business stores own both records and derived indexes. Callers use their atomic methods rather than editing backing collections independently.

## Canonical flows

### New campaign

```text
NewGameConfig
  -> build_new_game
  -> validate authored input
  -> allocate IDs and records
  -> initialize strategic state
  -> validate invariants
  -> AppState
```

Entry point: `src/systems/bootstrap.rs`.

### Player command

```text
PlayerCommand
  -> apply_player_command
  -> validate all preconditions
  -> calculate resolved values
  -> commit one atomic mutation
  -> append durable feedback and audit data
  -> validate invariants at the adapter boundary
```

Entry point: `src/systems/commands.rs`.

Cross-record operations use validated tokens when state may change between validation and commit. Tokens are consumed by `commit` and revalidate current state.

### Simulation

`advance_days` validates the requested range, registry compatibility, and market definitions before the first mutation. Each day runs this sequence:

1. Reset market flow counters.
2. Apply routes, laws, and active crisis effects.
3. Decide and apply business purchases.
4. Decide and apply production.
5. Decide and apply business sales.
6. Decide and apply household consumption.
7. Decide and apply maintenance.
8. Apply spoilage and update prices.
9. Apply law price controls.
10. Update business lifecycle state.
11. Advance the clock.
12. Run weekly systems on week boundaries.
13. Run monthly systems every 30 days.
14. Run annual and succession systems every 360 days.
15. Append the day audit record.
16. Validate debug invariants.

Entry points: `src/systems/simulation.rs` and `src/systems/strategic.rs`.

Order is part of the simulation contract. Change it only with tests that demonstrate the intended causal effect.

### Persistence

```text
save_state
  -> release-mode state validation
  -> serialize AppState with its embedded schema version
  -> write and synchronize same-directory temporary file
  -> atomically replace destination

load_state
  -> parse JSON and read the embedded schema version
  -> migrate supported older schema
  -> deserialize AppState
  -> rebuild or verify derived ownership data
  -> release-mode state validation
```

Entry point: `src/persistence.rs`.

Schema changes require a version increment, migration code, migration tests, round-trip tests, and documentation updates.

### Projection

`build_campaign_projection` converts immutable registry and state data into adapter-safe views. `render_campaign_html` consumes the projection. Projection code may format or aggregate, but it must not mutate state or recreate business rules.

## Determinism contract

Given the same registry, state, seed, command sequence, and day count, execution must produce identical state.

Required practices:

- Use `state.rng` for all simulation randomness.
- Use ordered collections or explicit sorting before result-affecting iteration.
- Break ties with stable typed IDs.
- Persist RNG state and all generated inputs that affect later behavior.
- Do not read wall-clock time, environment state, filesystem order, or external services inside core systems.

## Mutation and accounting rules

- Validate every reference, lifecycle state, permission, range, ownership claim, capacity, and arithmetic boundary before mutation.
- Failed operations leave state unchanged.
- Multi-record transfers calculate all resulting balances before committing any balance.
- Use fixed-point helpers in `src/money.rs`; do not use floating point for economic state.
- Use wide intermediates for multiply-then-divide arithmetic and saturate only the final result.
- Keep indexes, ownership, occupancy, collateral, employment, and lifecycle state synchronized in the same operation.
- Durable external work is represented in state before an adapter performs it.

## Invariant layers

The project enforces state validity at three layers:

1. Type and visibility constraints prevent direct invalid mutation.
2. System validation rejects invalid operations before commit.
3. `validate_invariants` checks cheap runtime invariants in debug execution, while persistence validation checks loaded and saved states in release mode.

Important invariant groups:

- Registry and record references resolve.
- Store indexes are complete and unique.
- Exclusive ownership and occupancy are consistent.
- Lifecycle states agree across related records.
- Financial values and quantities obey their domain bounds.
- Public-work progress, administrative load, and other derived values match source records.
- Histories are chronological.
- Next-ID allocators are ahead of all allocated IDs and still allocatable.

## Where to implement a change

| Change | Primary owner | Usually also update |
|---|---|---|
| Add immutable Rivergate content | `src/registry/mod.rs` | Bootstrap, registry tests, projections if visible. |
| Add persistent runtime state | `src/core/*`, `src/core/state.rs` | Persistence migration, validation, invariants, projection, tests. |
| Add a player action | `src/systems/commands.rs` | Command tests, projection discoverability, gameplay candidates and coverage. |
| Add an economic daily rule | `src/systems/simulation.rs` | Simulation tests, invariants, causal projection fields. |
| Add weekly/monthly/annual behavior | `src/systems/strategic.rs` | Strategic tests, outbox/audit feedback, gameplay snapshots. |
| Add a cross-record transaction | Owning system or `transactions.rs` | Dedicated error variants, rollback tests, stale-token tests. |
| Add a read-only view | `src/projection.rs` | Projection tests and HTML rendering when applicable. |
| Change save format | `src/persistence.rs` | Schema version, migration and validation tests, `STATUS.md`. |
| Add gameplay evaluation | `src/gameplay.rs` | Harness tests and `GAMEPLAY_HARNESS.md`. |
| Add CLI syntax | `src/main.rs` | `scripts/verify_cli.sh` and `README.md`. |

## Public API

The library facade in `src/lib.rs` exports the supported integration surface. Prefer adding stable operations there rather than exposing record internals.

Primary entry points:

- `build_rivergate_registry`
- `build_new_game`
- `advance_days`
- `apply_player_command`
- `quote_business_acquisition`
- `build_campaign_projection`
- `render_campaign_html`
- `save_state` and `load_state`
- `run_gameplay_harness` and `render_gameplay_report`
- `validate_invariants`

## Design boundaries

The engine models one detailed city and abstract regional connections. Tactical combat, manual character movement, equal-detail multi-city simulation, repetitive crafting, and routine dialogue trees are outside the architecture. See `DESIGN.md` for product scope and `STATUS.md` for current implementation coverage.
