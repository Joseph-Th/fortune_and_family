# Architecture

This document defines code ownership, dependency direction, mutation flows, execution order, and extension points.

## System model

Civic Dynasty is a deterministic simulation kernel with explicit definitions, explicit state, and explicit mutation paths.

```text
Registry definitions + AppState
            |
            v
canonical systems
  bootstrap | commands | legal | progression | simulation | strategic | transactions
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
| `src/systems/legal.rs` | Grounded debt and contract claims shared by player and rival litigation. |
| `src/systems/progression.rs` | Monotonic dynasty campaign progression and migration reconstruction. |
| `src/systems/simulation.rs` | Daily economic pipeline. |
| `src/systems/strategic.rs` | Weekly, monthly, annual, and cross-domain systems. |
| `src/systems/transactions.rs` | Reusable validated transaction primitives. |
| `src/systems/invariants.rs` | Debug runtime invariants. |
| `src/persistence.rs` | Versioned save/load, migrations, and release-mode validation. |
| `src/projection.rs` | Read-only projections and self-contained HTML rendering. |
| `src/gameplay.rs` | Player agents, counterfactual analysis, scores, findings, and traces. |
| `src/art/color.rs` | Integer color model, hue-shifted shading ramps, and indexed palettes. |
| `src/art/math.rs` | Fixed-point angles, trigonometry, and vector helpers. |
| `src/art/canvas.rs` | Indexed pixel buffers and compositing. |
| `src/art/surface.rs` | Material, light, and depth buffers and their resolution to indices. |
| `src/art/shape.rs` | Shaded rasterization primitives, the light model, and contour passes. |
| `src/art/rig.rs` | Skeletons, poses, and the humanoid rig. |
| `src/art/anim.rs` | Keyframed clips and pose sampling. |
| `src/art/sprite.rs` | Character specifications, posed drawing, and sheet composition. |
| `src/art/png.rs` | Dependency-free indexed PNG and base64 encoding. |
| `src/art/lint.rs` | Automated sprite review checks. |
| `src/art/harness.rs` | Batch sprite review, findings report, and the HTML contact sheet. |
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

Capital movement between a dynasty and its businesses is canonical system behavior, not adapter behavior. Recapitalization moves treasury into an owned business; protected owner distribution moves only cash above the business operating floor back to treasury. Both paths preflight finance versions and supported numeric ranges before mutation. Manual owner distributions reuse the same 21-day operating floor as automatic dividends so command and simulation behavior cannot disagree about what cash is safely distributable.

Civic capital movement is also canonical command behavior. `FundPublicWork` can move treasury only into an unfinished public work already sponsored by the player dynasty. The command preflights treasury, civic-contribution, and remaining-budget bounds, records the private spending as civic contribution, and reuses `apply_public_work_completion` when it closes the budget. Municipal weekly spending and direct dynasty patronage therefore converge on the same completed-infrastructure effects instead of maintaining parallel civic outcome logic.

Cross-record operations may use consumed validated tokens. A token must revalidate state at commit time because the state may have changed after initial validation. Business-finance tokens also capture the finance version of each affected business, so an intervening valid finance mutation invalidates the stale token even when balances would still permit the original operation.

### Time advancement

`advance_days` validates the requested range and registry compatibility before mutation. It executes the complete requested range against a working copy and replaces the caller's state only after every requested day succeeds. Accounting overflow, finance-version exhaustion, timeline exhaustion, or another typed simulation failure therefore leaves the original campaign unchanged. Future schedules must be created through `checked_future_day`; `i64::MAX` is outside the supported schedulable range so an obligation can never silently saturate into an unreachable due date. Each simulated day runs in this order:

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
12. Expire time-limited reports and office directives whose inclusive expiry day has passed.
13. Run weekly systems on week boundaries.
14. Run monthly systems every 30 days.
15. Run annual and succession systems every 360 days.
16. Refresh monotonic campaign progression from durable gameplay milestones.
17. Append the day audit record.
18. Validate debug invariants.

Owners: `src/systems/simulation.rs`, `src/systems/strategic.rs`, and `src/systems/progression.rs`.

Execution order is part of the simulation contract. Change it only with tests that establish the intended causal effect.

Production and maintenance both decide replacement-tool demand before their apply phase. Tool quantities are bounded by the market stock visible when the plan is created, and the corresponding spending is carved out of the operating or maintenance cost the business was already going to pay. Non-tool production may route up to 80% of existing operating overhead through replacement tools, while maintenance may route its full maintenance budget through tools when stock permits. Apply therefore commits a precomputed inter-business market flow without adding a second business charge or inventing demand during mutation.

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
  -> normalize legacy derived campaign phases and expired time-limited state when required
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

### Art layer

The art layer is a boundary adapter that reads specifications and produces images. It owns no campaign state and encodes no domain rules.

```text
CharacterSpec -> palette and materials -> skeleton and sampled pose -> shaded surface
  -> indexed canvas -> sprite sheet -> automated review -> HTML review page
```

Primitives never write palette indices. They write a material identifier, a per-mille light value, a depth value, and a dither flag into a `Surface`. The flag lets flat panels opt out of dithering that only curved forms need. Resolution maps light onto each material's ramp, applies ordered dithering between steps, and replaces silhouette pixels with the material's own darkest step. Form and color therefore stay independent, so a pose can be relit, recolored, or restyled without redrawing it.

All art arithmetic is integer. Angles are binary radians, joint positions resolve in sixteenth-pixel units, and shading uses fixed-point normals, so a specification renders identically on every platform and in both build profiles.

Owners: `src/art/*`. The harness entry point is `build_art_review`; the page renderer is `render_art_review_html`.

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
- Use `checked_future_day` for runtime schedules instead of saturating date arithmetic.
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
- Next-ID allocators are ahead of all allocated IDs and never use the reserved invalid sentinel.
  The terminal valid counter may be persisted; a subsequent allocation must fail atomically with
  `IdentifierAllocationError` rather than wrap or partially mutate state.

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
| Add a drawing primitive or material | `src/art/shape.rs`, `src/art/surface.rs` | Shading tests, determinism tests. |
| Add a sprite subject or clip | `src/art/sprite.rs`, `src/art/anim.rs` | Review checks, harness coverage, status update. |
| Add an art review check | `src/art/lint.rs` | Harness report, HTML sheet, and tests. |

## Public API

`src/lib.rs` defines the supported integration surface. Prefer adding stable operations there instead of exposing record internals.

The authoritative player mutation schema is `PlayerCommand` in `src/systems/commands.rs`.
