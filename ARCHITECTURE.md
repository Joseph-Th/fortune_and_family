# Architecture

This document defines code ownership, dependency direction, canonical mutation flows, simulation order, and extension points.

## System model

Civic Dynasty is a deterministic simulation kernel with explicit definitions, state, and mutation paths.

```text
Registry definitions + AppState
            |
            v
canonical systems
  bootstrap | commands | invariants | legal | progression | simulation | strategic | transactions
            |
            +--> invariants
            +--> audit / chronicle / outbox
            |
            v
boundary adapters
  persistence | CLI | projections | HTML | gameplay harness | art
```

Dependency direction is one way:

1. Core types and registry definitions define data.
2. Systems read definitions and state, validate operations, and mutate state.
3. Adapters serialize, render, inspect, or invoke systems.
4. Adapters do not own domain rules.

## Ownership map

| Path | Responsibility |
|---|---|
| `src/core/records.rs` | Core population and economic records. |
| `src/core/extended.rs` | Strategic, civic, family, finance, property, labor, relationship, and crisis records. |
| `src/core/mod.rs` | Core facade: record and state type re-exports. |
| `src/core/state.rs` | `AppState`, clock, ID allocation, synchronized stores, state access. |
| `src/ids.rs` | Typed persistent IDs. |
| `src/money.rs` | Fixed-point `Money`, `Quantity`, affordability, and ratio arithmetic. |
| `src/registry/mod.rs` | Immutable Rivergate definitions and lookup validation. |
| `src/rng.rs` | Serializable deterministic RNG. |
| `src/systems/bootstrap.rs` | New campaign construction. |
| `src/systems/commands.rs` | `PlayerCommand`, validation, dispatch, and command-owned mutation. |
| `src/systems/mod.rs` | Systems facade: entry-point re-exports and shared scheduling and worker helpers. |
| `src/systems/legal.rs` | Grounded debt and contract claims. |
| `src/systems/progression.rs` | Monotonic campaign progression. |
| `src/systems/simulation.rs` | Daily economic pipeline and time advancement. |
| `src/systems/strategic.rs` | Daily, weekly, monthly, annual, and cross-domain systems. |
| `src/systems/transactions.rs` | Reusable validated transaction primitives. |
| `src/systems/invariants.rs` | Runtime cross-record invariants. |
| `src/persistence.rs` | Current-schema save/load, release validation, atomic writes. |
| `src/projection.rs` | Immutable read models and self-contained HTML rendering. |
| `src/gameplay/` | Deterministic player agents, counterfactual attribution, scores, findings, traces. |
| `src/art/*` | Deterministic procedural sprite rendering and review. |
| `src/main.rs` | CLI adapter. |

## State ownership

### Registry

`Registry` contains immutable authored definitions such as goods, recipes, districts, institutions, routes, and starting backgrounds. Runtime records refer to definitions through typed IDs.

### AppState

`AppState` contains every mutable value required to resume deterministic execution, including the clock, RNG, ID allocators, records, derived stores, strategic state, and durable histories.

If a generated value can affect future behavior, it belongs in `AppState` or a record owned by it.

### Records and synchronized stores

Records contain identity, references, local values, and lifecycle state. Consequential mutation belongs in systems.

Character, household, and business stores own records plus derived indexes. Use store methods for insertion, removal, and ownership changes; do not update backing records and indexes independently.

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
  -> validate references, permissions, lifecycle, capacity, and arithmetic
  -> resolve complete result
  -> commit atomically
  -> append durable feedback when consequential
  -> validate invariants
```

Owner: `src/systems/commands.rs` and the subsystem functions it invokes.

Multi-record operations may use consumed validated tokens. A deferred commit must revalidate the state it depends on; stale tokens must fail without mutation.

### Time advancement

`advance_days` executes the requested range against a working copy and replaces the caller's state only if every requested day succeeds. A simulation error therefore leaves the original state unchanged.

Each simulated day runs in this order:

1. Reset market flow counters.
2. Apply routes, laws, active crisis effects, and AI business recovery.
3. Decide and apply business purchases.
4. Decide and apply production.
5. Decide and apply business sales.
6. Decide and apply household consumption.
7. Decide and apply maintenance.
8. Apply spoilage (market and business inventories) and update prices.
9. Apply price controls.
10. Update business lifecycle state.
11. Advance the clock.
12. Expire time-limited reports and office directives after their inclusive expiry day.
13. Run weekly systems on week boundaries. Employment wages transfer directly from each business to its households; weekly external regional income arrives as outside silver scaled by route health and does not draw on the market clearing account.
14. Run monthly systems every 30 days, including AI objectives, AI dynasty upkeep, AI legal filings, and institution selections.
15. Run annual and succession systems every 360 days.
16. Refresh campaign progression from durable milestones.
17. Append the day audit record.
18. Validate runtime invariants.

Owners: `src/systems/simulation.rs`, `src/systems/strategic.rs`, and `src/systems/progression.rs`.

Execution order is causal behavior. Change it only with tests that establish the intended effect.

Strategic scheduling lives in `src/systems/strategic.rs`:

- **Daily**: routes, crisis effects, AI business recovery, external route supply.
- **Weekly**: household wage settlement, contracts, loans, civic debts, property rents, employment, dividends, public works, relationship and reputation updates.
- **Monthly**: district conditions (including property value drift), institution selections, office duties and directives, AI objectives, AI dynasty upkeep, AI credit participation, AI legal filings and case resolution, crisis detection.
- **Annual**: character health, succession, dynastic milestones.

The market clearing account is the market's internal cash pool: business purchases, unmodeled operating and maintenance costs, public-work tool purchases and their construction labor/materials residual, unowned-property sale proceeds, banking-panic deposit flight, AI dynasty upkeep, AI campaigning spend, and AI legitimacy patronage credit it; business sales, vacancy income, crisis profiteering extraction, and office toll-revenue draws against it debit it.

Weekly external regional income is outside silver paid directly to households. Its rate scales with average external-route health, so trade disruption tightens household budgets instead of draining the pool.

AI dynasties act on the same cadence through `recover_ai_businesses` (daily), `advance_ai_objectives`, `apply_ai_dynasty_upkeep`, `advance_ai_credit_participation`, `file_grounded_ai_legal_cases`, and `resolve_institution_selections` (monthly).

### Campaign phases

`CampaignPhase` in `src/core/records.rs` is the persistent, monotonically advancing phase enum. Its variants are `Foundation`, `Establishment`, `Ascendancy`, `Dominion`, and `Legacy`; `CampaignPhase::label` maps them to the product-facing names used by `DESIGN.md` and `GAMEPLAY_HARNESS.md`:

| Variant | Product label |
|---|---|
| `Foundation` | Foundation |
| `Establishment` | Establishment |
| `Ascendancy` | Institutional ascent |
| `Dominion` | Dynastic governance |
| `Legacy` | Succession and legacy |

`refresh_campaign_phases` in `src/systems/progression.rs` derives the phase from durable commercial, institutional, civic, and succession milestones and never moves it backwards.

### Persistence

```text
save_state / save_state_cas / save_state_new
  -> release validation (including canonical registry fingerprint verification)
  -> check destination non-existence or verify compare-and-swap SaveRevision
  -> serialize current AppState
  -> write and synchronize same-directory temporary file
  -> atomically replace destination (visibility commit point)
  -> synchronize parent directory durability on platforms that support it (returns SaveOutcome)

load_state / load_state_with_revision
  -> read bounded save file and compute SaveRevision
  -> validate absence of duplicate JSON members
  -> read schema version and require current schema version
  -> deserialize AppState
  -> verify indexes, references, and registry fingerprint
  -> release validation
```

Owner: `src/persistence.rs`.

Serialized contract changes require a schema increment, current-schema round-trip tests, rejection coverage for non-current schemas, and `STATUS.md` updates. Older save schemas are unsupported rather than migrated. Atomic staging commits visibility before directory synchronization, and compare-and-swap validation prevents stale multi-process writer conflicts.

### Projection and rendering

`build_state_summary` and `build_campaign_projection` derive read models from immutable registry and state data. `render_campaign_html` consumes the campaign projection. `CampaignProjection::attention` is the single canonical classification of conditions needing player attention; both the dashboard cards and the CLI summary format that one list instead of re-deriving their own rules.

Projection code may aggregate and format. It must not mutate state or recreate command validation.

### Gameplay harness

The harness generates state-derived command candidates, validates them through `apply_player_command` on cloned state, commits through the same API, advances through `advance_days`, and compares action and no-action branches.

Independent matrix campaigns run in parallel; each campaign owns its state and the shared registry is immutable, so campaign ordering and determinism are preserved. The harness does not directly mutate domain records during play. See `GAMEPLAY_HARNESS.md`.

### Art

The art layer owns deterministic rendering specifications, integer geometry/shading, sprite composition, encoding, automated review, and review HTML. It owns no campaign state and no gameplay rules.

## Determinism contract

Given the same registry, state, seed, command sequence, and day count, execution must produce identical state.

- Use `state.rng` for simulation randomness.
- Use ordered collections or explicit sorting for result-affecting iteration.
- Use typed IDs as stable tie-breakers.
- Persist RNG state and generated values that affect future behavior.
- Exclude wall-clock time, environment state, filesystem order, external services, and sleeps from core logic.

## Mutation and accounting contract

- Validate references, ownership, permissions, lifecycle, capacities, ranges, and arithmetic before mutation.
- Failed operations leave state unchanged.
- Calculate complete balance and ownership results before multi-record commits.
- Use fixed-point helpers from `src/money.rs`.
- Use wide intermediates for multiply-then-divide arithmetic.
- Use shared checked scheduling helpers for future dates.
- Keep indexes, ownership, occupancy, collateral, employment, and lifecycle state synchronized.
- Represent durable external work in state before an adapter performs it.

## Invariant layers

State validity is enforced by:

1. Types and visibility that restrict unsupported mutation.
2. System validation before commit.
3. Runtime invariants during simulation.
4. Release-mode validation at persistence boundaries.

Important invariant groups include registry references, derived indexes, ownership and occupancy, lifecycle agreement, numeric bounds, histories, and ID allocator validity.

## Extension map

| Change | Primary owner | Adjacent work |
|---|---|---|
| Immutable Rivergate content | `src/registry/mod.rs` | Registry tests, bootstrap, projection when visible. |
| Persistent state | `src/core/*`, `src/core/state.rs` | Bootstrap, validation, invariants, projection, tests. |
| Player command | `src/systems/commands.rs` | Command tests, feedback, projection, gameplay integration, CLI smoke when needed. |
| Daily economic rule | `src/systems/simulation.rs` | Simulation tests, causal ordering, invariants. |
| Scheduled strategic rule | `src/systems/strategic.rs` | Strategic tests, feedback, gameplay snapshots. |
| Cross-record transaction | Owning system or `src/systems/transactions.rs` | Typed errors, atomicity, stale-token tests. |
| Read-only output | `src/projection.rs` | Projection/rendering tests. |
| Save format | `src/persistence.rs` | Current schema, release validation, round trip, status. |
| Gameplay evaluation | `src/gameplay/` | Report schema, tests, `GAMEPLAY_HARNESS.md`. |
| CLI syntax | `src/main.rs` | CLI smoke, README workflow when relevant. |
| Art primitive/subject/check | `src/art/*` | Determinism, review coverage, schema/status when serialized output changes. |

## Public API

`src/lib.rs` defines the supported integration surface. Prefer stable operations there over exposing record internals. `PlayerCommand` in `src/systems/commands.rs` is the authoritative player mutation schema.
