# Architecture

This document defines code ownership, dependency direction, mutation flows, execution order, and extension points.

## System model

Civic Dynasty is a deterministic simulation kernel with explicit definitions, explicit state, and explicit mutation paths.

```text
Registry definitions + AppState
            |
            v
canonical systems
  bootstrap | commands | simulation | strategic | transactions
            |
            +--> runtime invariants
            +--> audit, chronicle, and outbox records
            |
            v
boundary adapters
  persistence | CLI | projections | HTML | gameplay harness
```

Dependency direction is one way:

1. Core types and registry definitions define data.
2. Systems read definitions and state, validate operations, and mutate state.
3. Boundary adapters serialize, render, or invoke canonical systems.
4. Adapters do not own domain rules.

## Repository map

| Path | Responsibility |
|---|---|
| `src/core/records.rs` | Primary population, household, business, market, and contract records. |
| `src/core/extended.rs` | Family, institutions, finance, property, labor, laws, relationships, crises, and other strategic records. |
| `src/core/state.rs` | `AppState`, clock, ID allocation, synchronized stores, and public state access. |
| `src/ids.rs` | Typed persistent IDs. |
| `src/money.rs` | Fixed-point `Money`, `Quantity`, costs, affordability, and ratio arithmetic. |
| `src/registry/mod.rs` | Immutable Rivergate definitions and lookup validation. |
| `src/rng.rs` | Serializable deterministic RNG. |
| `src/systems/bootstrap.rs` | New campaign construction. |
| `src/systems/commands.rs` | `PlayerCommand`, command validation, and dispatch. |
| `src/systems/simulation.rs` | Daily economic pipeline. |
| `src/systems/strategic.rs` | Weekly, monthly, annual, and cross-domain systems. |
| `src/systems/transactions.rs` | Reusable validated transaction primitives. |
| `src/systems/invariants.rs` | Debug runtime invariants. |
| `src/persistence.rs` | Versioned save/load, migrations, and release-mode validation. |
| `src/projection.rs` | Read-only projections and self-contained HTML rendering. |
| `src/gameplay.rs` | Player agents, counterfactual analysis, scores, findings, and traces. |
| `src/main.rs` | CLI parsing and adapter behavior. |
| `src/*_tests.rs` | Large sibling test suites. |
| `src/test_support.rs` | Shared deterministic fixtures and diagnostics. |

## Ownership

### Registry

`Registry` owns immutable scenario definitions: goods, recipes, districts, institutions, routes, backgrounds, and other authored content. Runtime records refer to definitions through typed IDs.

Mutable campaign values do not belong in the registry.

### AppState

`AppState` owns every value required to resume deterministic execution:

- Clock, RNG state, schema version, and next-ID allocators
- Dynasties, characters, households, businesses, and institutions
- Markets, contracts, loans, municipal debt, property, and employment
- Laws, public works, courts, routes, crises, information, and relationships
- Family governance, succession state, AI objectives, outbox, chronicle, and audit history

If a generated value can affect future behavior, it belongs in `AppState` or in a record owned by `AppState`.

### Records

Records hold identity, references, local values, and lifecycle state. Public code reads records through accessors. Consequential mutation belongs in systems.

### Synchronized stores

Character, household, and business stores own records and their derived indexes. Use store methods for insertion, removal, and ownership transfer. Do not mutate backing collections and indexes independently.

## Canonical flows

### New campaign

```text
NewGameConfig
  -> build_new_game
  -> validate authored input
  -> allocate records and IDs
  -> initialize strategic state
  -> validate invariants
  -> AppState
```

Owner: `src/systems/bootstrap.rs`.

### Player command

```text
PlayerCommand
  -> apply_player_command
  -> validate all preconditions
  -> calculate resolved values
  -> commit one atomic mutation
  -> append durable feedback and audit data
  -> validate invariants at the boundary
```

Owner: `src/systems/commands.rs` and the subsystem functions it invokes.

Cross-record operations may use consumed validated tokens. A token must revalidate state at commit time because the state may have changed after initial validation. Business-finance tokens also capture the finance version of each affected business, so an intervening valid finance mutation invalidates the stale token even when balances would still permit the original operation.

### Time advancement

`advance_days` validates the requested range and registry compatibility before mutation. It executes the complete requested range against a working copy and replaces the caller's state only after every requested day succeeds. Accounting overflow, finance-version exhaustion, or another typed simulation failure therefore leaves the original campaign unchanged. Each simulated day runs in this order:

1. Reset market flow counters.
2. Apply routes, laws, and active crisis effects.
3. Decide and apply business purchases.
4. Decide and apply production.
5. Decide and apply business sales.
6. Decide and apply household consumption.
7. Decide and apply maintenance.
8. Apply spoilage and update prices.
9. Apply price controls.
10. Update business lifecycle state.
11. Advance the clock.
12. Run weekly systems on week boundaries.
13. Run monthly systems every 30 days.
14. Run annual and succession systems every 360 days.
15. Append the day audit record.
16. Validate debug invariants.

Owners: `src/systems/simulation.rs` and `src/systems/strategic.rs`.

Execution order is part of the simulation contract. Change it only with tests that establish the intended causal effect.

### Persistence

```text
save_state
  -> release-mode validation
  -> serialize current AppState
  -> write and synchronize a same-directory temporary file
  -> atomically replace the destination

load_state
  -> parse schema version
  -> run explicit version-by-version migrations
  -> deserialize AppState
  -> verify derived ownership and indexes
  -> release-mode validation
```

Owner: `src/persistence.rs`.

A serialized contract change requires a schema increment, one migration from the previous version, migration coverage, round-trip coverage, and `STATUS.md` updates.

### Projection

`build_state_summary` and `build_campaign_projection` produce adapter-safe read models from immutable registry and state data. `render_campaign_html` consumes the campaign projection.

Projection code may aggregate and format. It must not mutate state, infer hidden commands, or recreate validation rules.

### Gameplay harness

The harness generates state-derived `PlayerCommand` candidates, validates them through the canonical command API on cloned state, commits one viable action, advances both action and baseline branches, and compares the outcomes.

The harness does not mutate records directly during play. See `GAMEPLAY_HARNESS.md`.

## Determinism contract

Given the same registry, state, seed, command sequence, and day count, execution must produce identical state.

Required practices:

- Use `state.rng` for simulation randomness.
- Use ordered collections or explicit sorting for result-affecting iteration.
- Use typed IDs as stable tie-breakers.
- Persist RNG state and generated values that affect later behavior.
- Exclude wall-clock time, environment state, filesystem order, external services, and sleeps from core logic.

## Mutation and accounting

- Validate references, ownership, permissions, lifecycle, capacities, ranges, and arithmetic before mutation.
- Failed operations leave state unchanged.
- Calculate all balances and ownership results before committing multi-record transfers.
- Use fixed-point helpers from `src/money.rs`.
- Use wide intermediates for multiply-then-divide arithmetic.
- Keep indexes, ownership, occupancy, collateral, employment, and lifecycle state synchronized.
- Represent durable external work in state before an adapter performs it.

## Invariant layers

State validity is enforced at three layers:

1. Types and visibility prevent unsupported direct mutation.
2. System validation rejects invalid operations before commit.
3. Runtime and persistence validators detect cross-record inconsistency.

Important invariant groups:

- Registry and record references resolve.
- Store indexes are complete and unique.
- Ownership, occupancy, tenancy, and collateral agree.
- Related lifecycle states agree.
- Financial and quantity values remain within domain bounds.
- Derived values match authoritative records.
- Histories are chronological.
- Next-ID allocators are ahead of all allocated IDs and remain allocatable.

## Extension map

| Change | Primary owner | Required adjacent work |
|---|---|---|
| Add immutable Rivergate content | `src/registry/mod.rs` | Registry tests, bootstrap, projections when visible. |
| Add persistent state | `src/core/*`, `src/core/state.rs` | Bootstrap or migration, validation, invariants, projections, tests. |
| Add a player command | `src/systems/commands.rs` | Command tests, feedback, projections, gameplay integration, CLI smoke when needed. |
| Add a daily economic rule | `src/systems/simulation.rs` | Simulation tests, invariants, causal observability. |
| Add scheduled strategic behavior | `src/systems/strategic.rs` | Strategic tests, feedback, gameplay snapshots. |
| Add cross-record transfer | Owning system or `transactions.rs` | Typed errors, atomicity tests, stale-token tests. |
| Add read-only output | `src/projection.rs` | Projection and rendering tests. |
| Change save format | `src/persistence.rs` | Schema, migration, validation, round-trip tests, status update. |
| Extend gameplay evaluation | `src/gameplay.rs` | Harness schema, tests, and documentation. |
| Add CLI syntax | `src/main.rs` | CLI smoke and README update. |

## Public API

`src/lib.rs` defines the supported integration surface. Prefer adding stable operations there instead of exposing record internals.

The authoritative player mutation schema is `PlayerCommand` in `src/systems/commands.rs`.
