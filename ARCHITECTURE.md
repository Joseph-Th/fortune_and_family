# Architecture

Defines code ownership, dependency direction, canonical mutation flows, simulation order, and extension points.

Profiles: **Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation**.

Related authorities:

- [AGENTS.md](AGENTS.md) — execution card
- [STATUS.md](STATUS.md) — current scope
- [TESTING.md](TESTING.md) — verification
- [DESIGN.md](DESIGN.md) — product intent
- [GAMEPLAY_HARNESS.md](GAMEPLAY_HARNESS.md) — harness semantics
- Root `../AGENTS.md` — workspace coordination

## System model

Deterministic kernel: immutable definitions + mutable state flow through canonical systems to boundary adapters.

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

1. Core types and registry define data.
2. Systems read definitions and state, validate, and mutate state.
3. Adapters serialize, render, inspect, or invoke systems.
4. Adapters own no domain rules.

## Ownership map

| Path | Responsibility |
|---|---|
| `src/core/records.rs` | Core population and economic records. |
| `src/core/extended.rs` | Strategic, civic, family, finance, property, labor, relationship, and crisis records. |
| `src/core/mod.rs` | Core facade: record and state re-exports. |
| `src/core/state.rs` | `AppState`, clock, ID allocation, synchronized stores. |
| `src/ids.rs` | Typed persistent IDs. |
| `src/money.rs` | Fixed-point `Money`/`Quantity`, affordability, ratio arithmetic, ceil-division helpers. |
| `src/registry/mod.rs` | Immutable Rivergate definitions and lookup validation. |
| `src/rng.rs` | Serializable deterministic RNG. |
| `src/systems/bootstrap.rs` | New campaign construction. |
| `src/systems/commands/` | `PlayerCommand` schema, dispatch, and command-owned mutation. |
| `src/systems/mod.rs` | Systems facade: entry-point re-exports, scheduling helpers, `capacity_weighted_route_disruption` shared by household/trade availability and crisis detection. |
| `src/systems/legal.rs` | Grounded debt and contract claims. |
| `src/systems/progression.rs` | Monotonic campaign progression. |
| `src/systems/simulation/` | Daily economic pipeline: `mod.rs` orchestrates the day; `purchases.rs` owns input procurement; `market.rs` owns spoilage, pricing, and break-even floors; `mod.rs` owns workshop maintenance and lifecycle. |
| `src/systems/strategic/` | Scheduled strategic systems by domain (see below). |
| `src/systems/transactions.rs` | Reusable validated transaction primitives. |
| `src/systems/invariants.rs` | Runtime cross-record invariants. |

Strategic submodules — one domain per file behind `strategic/mod.rs`:

| Submodule | Responsibility |
|---|---|
| `strategic/mod.rs` | Scheduling, shared relationship plumbing, law appliers, information reports, annual family systems. |
| `strategic/contracts.rs` | Supply contracts: terms, validation, capacity, weekly settlement. |
| `strategic/credit.rs` | Private and municipal credit: loans, civic debts, interest, collateral seizure. |
| `strategic/property.rs` | Real estate, tenancy, rents, district conditions, value drift, public works. |
| `strategic/households.rs` | Household living costs and monthly family pressure. |
| `strategic/businesses.rs` | Business ownership: capitalization, distributions, acquisitions, dividends. |
| `strategic/labor.rs` | Weekly employment settlement, market wage fairness, workforce disputes. |
| `strategic/offices.rs` | Political office: duties, stipends, powers, directives, elections. |
| `strategic/ai.rs` | Autonomous houses: objectives, upkeep, credit participation, recovery. |
| `strategic/legal_cases.rs` | Legal cases: hearings, judgments, execution, settlements, terminal write-offs. |
| `strategic/crises.rs` | Crisis detection, escalation, response effects, route risk. |
| `strategic/initialization.rs` | Deterministic bootstrap of strategic state. |

Commands submodules — one family per file behind `commands/mod.rs`:

| Submodule | Responsibility |
|---|---|
| `commands/mod.rs` | Schema, dispatch, shared spending plumbing, cooldown lookups. |
| `commands/consts.rs` | Tuning constants per command family. |
| `commands/error.rs` | Typed `CommandError` variants and conversions. |
| `commands/holdings.rs` | Owned-business transfers, capital, policy, wages. |
| `commands/trade.rs` | Supply contracts and private-credit negotiation. |
| `commands/law.rs` | Law sponsorship and municipal debt issuance. |
| `commands/property_cmd.rs` | Property purchase and liquidation. |
| `commands/civic.rs` | Public-work sponsorship and funding. |
| `commands/legal_cmd.rs` | Legal-case filing and settlement. |
| `commands/family.rs` | Governance, councils, heirs, wards, education. |
| `commands/politics.rs` | Institutions, patronage, offices, nominations, directives. |
| `commands/response.rs` | Crisis and labor-dispute responses. |
| `commands/information.rs` | Intelligence commissioning, leverage, notifications. |
| `src/persistence.rs` | Current-schema save/load, release validation, atomic writes. |
| `src/projection.rs` | Immutable read models and self-contained HTML rendering. |
| `src/gameplay/` | Deterministic player agents, counterfactual attribution, scores, findings, traces. |
| `src/art/*` | Deterministic procedural sprite rendering and review. |
| `src/main.rs` | CLI adapter. |

## State ownership

### Registry

`Registry` holds immutable definitions: goods, recipes, districts, institutions, routes, backgrounds. Runtime records reference them through typed IDs.

### AppState

`AppState` holds every mutable value required for deterministic continuation: clock, RNG, ID allocators, records, derived stores, strategic state, durable histories.

Any generated value that affects future behavior belongs in `AppState` or a record it owns.

### Records and synchronized stores

Records hold identity, references, local values, and lifecycle state. Mutation belongs in systems.

Character, household, and business stores own records plus derived indexes. Use store methods for insertion, removal, and ownership changes; do not update records and indexes independently.

Histories (audit log, chronicle, outbox) use `HistoryLog`: an append-only vector with cheap clones via shared immutable bulk plus an exclusive tail. Iteration order and serialized shape match a plain vector. Use `push`, iteration, `retain`, `partition_point`.

History rules:

- Text is append-only and immutable after construction.
- Audit days are nondecreasing (enforced invariant). Cooldown and recency scans stop at the day boundary via `latest_cooldown_audit_day`, `audit_records_from`/`audit_records_within_cooldown`, and `partition_point`.
- Unbounded reverse scans apply only to predicates that need arbitrary age ("has this ever happened").
- Fingerprint stores with `stable_serialized_checksum`; fingerprint histories with `HistoryLog::structural_checksum`. Do not reserialize a history for its checksum.

Non-persisted derivation memos are pure functions of persisted state. They are excluded from serialization and `AppState` equality (extend the hand-written `PartialEq` when adding fields), rebuilt lazily, and never observable.

Two memos exist: `CampaignEvidenceMemo` (`src/core/state.rs`) folds campaign-phase audit evidence for `refresh_campaign_phases`; `HistoryLog` checksum memo (`src/core/checksum.rs`) extends a running structural fold in constant time. Non-append mutations (`retain`, `iter_mut`, reordering) mark the checksum stale so the next read rebuilds once.

## Canonical flows

### New campaign

```text
NewGameConfig -> build_new_game -> validate input -> allocate records/IDs
  -> initialize strategic state -> validate invariants -> AppState
```

Owner: `src/systems/bootstrap.rs`.

### Player command

```text
PlayerCommand -> apply_player_command -> validate refs/permissions/lifecycle/capacity/ranges/arithmetic
  -> resolve complete result -> commit atomically -> append durable feedback -> validate invariants
```

Owner: `src/systems/commands/` and the subsystem it invokes.

Multi-record operations use consumed validated tokens. A deferred commit revalidates its dependencies; stale tokens fail without mutation.

### Time advancement

`advance_days` executes the requested range against a working copy and replaces the caller only if every day succeeds. A simulation error leaves the original state unchanged.

Each day runs in fixed order:

1. Reset market flow counters.
2. Apply routes, laws, active crisis effects, AI business recovery.
3. Decide and apply business purchases.
4. Decide and apply production.
5. Decide and apply business sales.
6. Decide and apply household consumption.
7. Decide and apply maintenance.
8. Apply spoilage and update prices.
9. Apply price controls.
10. Update business lifecycle state.
11. Advance the clock.
12. Expire time-limited reports and office directives after inclusive expiry.
13. Run weekly systems on week boundaries: employment wages (business → households directly), external regional income (outside silver scaled by route health, not via clearing account).
14. Run monthly systems every 30 days, including AI objectives, upkeep, legal filings, and institution selections.
15. Run annual and succession systems every 360 days.
16. Refresh campaign progression from durable milestones.
17. Append the day audit record.
18. Validate runtime invariants.

Owners: `src/systems/simulation/`, `src/systems/strategic/`, `src/systems/progression.rs`.

`advance_days` clones once and replaces only after every day succeeds. `advance_days_scratch` is the in-place variant for exclusively owned disposable branches such as harness counterfactuals: identical day loop, no defensive copy, caller must discard state on failure.

Execution order is causal behavior. Change it only with tests that establish the intended effect.

Strategic scheduling (`src/systems/strategic/`):

- **Daily**: routes, crisis effects, AI business recovery, external route supply.
- **Weekly**: wage settlement, contracts, loans, civic debts, property rents (district-indexed, discounted for fire damage), employment, dividends, public works, relationship/reputation updates.
- **Monthly**: district conditions (value drift, 180 bp building repair), household living costs (scaled by desirability; market staples pay separately), institution selections, office duties/directives, AI objectives/upkeep/credit/legal, crisis detection.
- **Annual**: character health, succession, dynastic milestones.

The market clearing account is the market's internal cash pool.

Credits: business purchases, unmodeled operating/maintenance costs, player public-work sponsorships and direct contributions, unmodeled service spends (law, councils, wards, education, nominations, crisis mobilizations, information), public-work tool/material/labor costs, unowned-property proceeds, banking-panic deposit flight, AI upkeep/campaigning/patronage.

Debits: business sales, vacancy income, crisis profiteering, office toll draws.

Weekly external regional income is outside silver paid directly to households, scaled by capacity-weighted external-route health. The NobleDemand levy is the one exception: it leaves Rivergate for the prince's court with no internal counterparty.

AI houses act on the same cadence through `recover_ai_businesses` (daily), `advance_ai_objectives`, `apply_ai_dynasty_upkeep`, `advance_ai_credit_participation`, `file_grounded_ai_legal_cases`, and `resolve_institution_selections` (monthly).

### Campaign phases

`CampaignPhase` (`src/core/records.rs`) is persistent and monotonic: `Foundation`, `Establishment`, `Ascendancy`, `Dominion`, `Legacy`. `CampaignPhase::label` maps to product names:

| Variant | Label |
|---|---|
| `Foundation` | Foundation |
| `Establishment` | Establishment |
| `Ascendancy` | Institutional ascent |
| `Dominion` | Dynastic governance |
| `Legacy` | Succession and legacy |

`refresh_campaign_phases` (`src/systems/progression.rs`) derives the phase from durable commercial, institutional, civic, and succession milestones and never moves it backwards.

### Persistence

```text
save_state / save_state_cas / save_state_new
  -> release validation (including registry fingerprint)
  -> verify destination non-existence or SaveRevision compare-and-swap
  -> serialize AppState -> write + sync same-directory temp file
  -> atomically replace destination (visibility commit point)
  -> sync parent directory where supported (returns SaveOutcome)

load_state / load_state_with_revision
  -> read bounded file, compute SaveRevision
  -> reject duplicate JSON members
  -> require current schema_version
  -> deserialize AppState -> verify indexes/refs/fingerprint -> release validation
```

Owner: `src/persistence.rs`.

Schema changes require an increment, current-schema round-trip tests, rejection of non-current schemas, and `STATUS.md` updates. Older schemas are unsupported. Atomic staging commits visibility before directory sync; compare-and-swap prevents stale multi-process overwrites.

### Projection and rendering

`build_state_summary` and `build_campaign_projection` derive read models from registry and state. `render_campaign_html` consumes the campaign projection. `CampaignProjection::attention` is the single canonical classification of conditions needing player attention; dashboard and CLI summary format that list.

Projection code may aggregate and format. It must not mutate state or recreate command validation.

### Gameplay harness

Generates state-derived candidates, validates via `apply_player_command` on clones, commits through the same API, advances via `advance_days`, and compares action vs no-action branches.

Independent campaigns run in parallel; each owns its `AppState` and the shared `Registry` is immutable, so ordering and determinism are preserved. The harness does not directly mutate domain records.

Bounded work: `max_candidate_probes`, decision intervals, consequence horizons, and `trace_limit` bound every run in domain terms. Wall-clock diagnostics (`simulated_days/s`) are advisory. See `GAMEPLAY_HARNESS.md`.

### Art

Owns deterministic rendering specifications, integer geometry/shading, sprite composition, encoding, automated review, and review HTML. It owns no campaign state or gameplay rules.

Rendering is one-way: `CharacterSpec` → indexed `Canvas` → PNG bytes → embedded data URI. No round-trip or editability is claimed. Generated HTML/PNG are derived; `CharacterSpec`/palette/rig are the source of truth.

## Determinism contract

Given the same registry, state, seed, command sequence, and day count, execution produces identical state.

```text
registry (fingerprint-bound) + serialized AppState (clock, RNG, allocators, records)
  + ordered explicit inputs (commands + day count) = bit-identical successor AppState
```

- Use `state.rng` for simulation randomness; do not read OS entropy, thread scheduling, or wall-clock time.
- Use ordered collections (`BTreeMap`/`BTreeSet`) or explicit sorting for result-affecting iteration.
- Use typed IDs as stable tie-breakers.
- Persist RNG state and every generated value that can affect future behavior.
- Exclude wall-clock time, environment, filesystem order, external services, and sleeps from core logic.
- Use fixed-point `Money`/`Quantity` with wide `i128` intermediates; no floating point participates in authored or simulated values.
- Execution envelope: same-process repeatability and same-build replay on the supported stable toolchain (`Cargo.toml` `rust-version` + `rust-toolchain.toml`). Cross-toolchain or cross-platform byte identity beyond fixed-point/behavioral equivalence is not claimed; `registry_fingerprint` and `schema_version` bind saves to definitions and fail closed on mismatch.

## Mutation and accounting contract

- Validate references, ownership, permissions, lifecycle, capacities, ranges, and arithmetic before mutation.
- Failed operations leave state unchanged.
- Calculate complete multi-record results before commit.
- Hot commits reserve every durable identifier before mutating, so the mutation phase is infallible without a defensive whole-campaign copy. Failed reservation restores the allocator snapshot while state is untouched.
- Use fixed-point helpers from `src/money.rs` with wide intermediates for multiply-then-divide.
- Use shared checked scheduling helpers for future dates.
- Keep indexes, ownership, occupancy, collateral, employment, and lifecycle state synchronized.
- Represent durable external work in state before an adapter performs it.

## Invariant layers

1. Types and visibility restrict unsupported mutation.
2. System validation before commit.
3. Runtime invariants during simulation.
4. Release-mode validation at persistence boundaries.

Groups include registry references, derived indexes, ownership and occupancy, lifecycle agreement, numeric bounds, histories, and ID allocator validity.

## Extension map

| Change | Primary owner | Adjacent work |
|---|---|---|
| Immutable content | `src/registry/mod.rs` | Registry tests, bootstrap, projection if visible. |
| Persistent state | `src/core/*`, `src/core/state.rs` | Bootstrap, validation, invariants, projection, tests. |
| Player command | `src/systems/commands/` | Command tests, feedback, projection, harness, CLI smoke if needed. |
| Daily rule | `src/systems/simulation/` | Simulation tests, ordering, invariants. |
| Scheduled rule | `src/systems/strategic/` domain file | Strategic tests, feedback, snapshots. |
| Cross-record transaction | Owning system or `src/systems/transactions.rs` | Typed errors, atomicity, stale-token tests. |
| Read-only output | `src/projection.rs` | Projection/rendering tests. |
| Save format | `src/persistence.rs` | Schema, validation, round trip, status. |
| Gameplay evaluation | `src/gameplay/` | Report schema, tests, `GAMEPLAY_HARNESS.md`. |
| CLI syntax | `src/main.rs` | CLI smoke, README workflow if needed. |
| Art primitive/subject/check | `src/art/*` | Determinism, review coverage, schema/status for serialized output changes. |

## Public API

`src/lib.rs` defines the supported integration surface. Prefer stable operations there over exposing record internals. `PlayerCommand` in `src/systems/commands/` is the authoritative player mutation schema.
