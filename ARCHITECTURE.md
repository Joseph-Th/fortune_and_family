# Architecture

Code ownership, dependency direction, canonical mutation flows, and execution order.

Profiles: **Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation**.

Related authorities: `AGENTS.md` (execution card), `STATUS.md` (current scope), `TESTING.md` (verification), `DESIGN.md` (intent), `GAMEPLAY_HARNESS.md` (harness). Root `../AGENTS.md` owns workspace coordination.

## System model

Deterministic kernel: immutable definitions + mutable state flow through canonical systems to boundary adapters.

```text
Registry definitions + AppState  ->  canonical systems  ->  boundary adapters
```

Dependency direction is one way:

1. Core types and registry define data.
2. Systems read definitions and state, validate, and mutate state.
3. Adapters serialize, render, inspect, or invoke systems; they own no domain rules.

## Ownership map

| Path | Responsibility |
|---|---|
| `src/core/records.rs` | Core population and economic records |
| `src/core/extended.rs` | Strategic, civic, family, finance, property, labor, relationship, crisis records |
| `src/core/history.rs` | Append-only `HistoryLog<T>` with cheap clones and incremental checksum |
| `src/core/mod.rs` | Core facade: record and state re-exports |
| `src/core/state.rs` | `AppState`, clock, ID allocation, synchronized stores |
| `src/ids.rs` | Typed persistent IDs |
| `src/money.rs` | Fixed-point `Money`/`Quantity` and ratio helpers |
| `src/registry/mod.rs` | Immutable Rivergate definitions and lookup validation |
| `src/rng.rs` | Serializable deterministic RNG |
| `src/systems/bootstrap.rs` | New campaign construction |
| `src/systems/commands/` | `PlayerCommand` schema, dispatch, and command-owned mutation |
| `src/systems/mod.rs` | Systems facade, scheduling helpers, shared route-disruption math |
| `src/systems/legal.rs` | Grounded legal claims |
| `src/systems/progression.rs` | Monotonic campaign progression |
| `src/systems/simulation/` | Daily pipeline (`mod.rs` orchestrates; `purchases.rs` procurement; `market.rs` spoilage/pricing; `succession.rs` annual health/succession); weekly external income is employment-scaled and route-dependent |
| `src/systems/strategic/` | Scheduled strategic systems by domain (see below) |
| `src/systems/transactions.rs` | Reusable validated transaction primitives |
| `src/systems/invariants.rs` | Runtime cross-record invariant checks |

Strategic submodules — one domain per file behind `strategic/mod.rs`:

| Submodule | Responsibility |
|---|---|
| `strategic/mod.rs` | Scheduling, relationship plumbing, law appliers, information, annual family systems |
| `strategic/contracts.rs` | Supply contracts: terms, validation, capacity, weekly settlement |
| `strategic/credit.rs` | Private and municipal credit: loans, civic debts, interest, collateral |
| `strategic/property.rs` | Real estate, tenancy, rents, district conditions, value drift, public works |
| `strategic/households.rs` | Household living costs and monthly family pressure |
| `strategic/businesses.rs` | Business ownership: capitalization, distributions, acquisitions |
| `strategic/labor.rs` | Weekly employment settlement, wage fairness, disputes |
| `strategic/offices.rs` | Political office: duties, stipends, powers, directives, elections |
| `strategic/ai.rs` | Autonomous houses: objectives, upkeep, credit participation, recovery |
| `strategic/legal_cases.rs` | Legal cases: hearings, judgments, execution, write-offs |
| `strategic/crises.rs` | Crisis detection, escalation, response, route risk |
| `strategic/initialization.rs` | Deterministic bootstrap of strategic state |

Command submodules — one family per file behind `commands/mod.rs`:

| Submodule | Responsibility |
|---|---|
| `commands/mod.rs` | Schema, dispatch, shared spending, cooldown lookups |
| `commands/consts.rs` | Tuning constants per command family |
| `commands/error.rs` | Typed `CommandError` variants |
| `commands/holdings.rs` | Owned-business transfers, capital, policy, wages |
| `commands/trade.rs` | Supply contracts and private-credit negotiation |
| `commands/law.rs` | Law sponsorship and municipal debt |
| `commands/property_cmd.rs` | Property purchase and liquidation |
| `commands/civic.rs` | Public-work sponsorship and funding |
| `commands/legal_cmd.rs` | Legal-case filing and settlement |
| `commands/family.rs` | Governance, councils, heirs, wards, education |
| `commands/politics.rs` | Institutions, patronage, offices, nominations |
| `commands/response.rs` | Crisis and labor responses |
| `commands/information.rs` | Intelligence commissioning, leverage, notifications |

Remaining adapters:

| Path | Responsibility |
|---|---|
| `src/persistence.rs` | Current-schema save/load, release validation, atomic writes |
| `src/projection.rs` | Read-only projections and self-contained HTML |
| `src/gameplay/` | Deterministic player agents, attribution, scores, findings, traces |
| `src/art/*` | Deterministic sprite rendering and review |
| `src/main.rs` | CLI adapter |

## State ownership

### Registry

`Registry` holds immutable definitions: goods, recipes, districts, institutions, routes, backgrounds. Runtime records reference them through typed IDs.

### AppState

`AppState` holds every mutable value required for deterministic continuation: clock, RNG, ID allocators, records, derived stores, strategic state, durable histories. Any generated value that affects future behavior belongs in `AppState` or a record it owns.

### Records and synchronized stores

Records hold identity, references, local values, and lifecycle state. Mutation belongs in systems.

Character, household, and business stores own records plus derived indexes. Use store methods for insertion, removal, and ownership changes; do not update records and indexes independently.

Histories (`audit_log`, `chronicle`, `outbox`) use `HistoryLog`: an append-only vector with cheap clones via shared bulk (`Arc`) plus an exclusive tail. Iteration order and serialized shape match a plain `Vec`. Use `push`, iteration, `retain`, `partition_point`.

History rules:

- Text is append-only and immutable after construction.
- Audit days are nondecreasing (enforced invariant). Cooldown scans stop at the day boundary via `latest_cooldown_audit_day`, `audit_records_from` / `within_cooldown`, and `partition_point`.
- Unbounded reverse scans apply only to predicates that need arbitrary age ("has this ever happened").
- Fingerprint stores with `stable_serialized_checksum`; histories fingerprint via `HistoryLog::structural_checksum`. Do not reserialize a history for its checksum.

Derived memos are pure functions of persisted state, excluded from serialization and `PartialEq` (extend the hand-written `PartialEq` when adding fields), and rebuilt lazily:

- `CampaignEvidenceMemo` (`src/core/state.rs`) folds campaign-phase audit evidence for `refresh_campaign_phases`.
- `HistoryLog` checksum memo (`src/core/checksum.rs`) extends the structural fold in constant time. Non-append mutations (`retain`, `iter_mut`, reordering) mark it stale for a one-time rebuild.

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

Owner: `src/systems/commands/` and the subsystem it invokes. Multi-record operations use consumed validated tokens; deferred commits revalidate dependencies and fail closed on stale tokens.

### Time advancement

`advance_days` executes the requested range against a working copy and replaces the caller only if every day succeeds. A simulation error leaves the original state unchanged.

Each day runs in fixed 18-step order:

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
13. Weekly systems on week boundaries: employment wages (business → households), external regional income (outside silver scaled by route health, not via clearing account).
14. Monthly systems every 30 days: AI objectives/upkeep/credit/legal, institution selections, district conditions, living costs, office duties.
15. Annual and succession systems every 360 days.
16. Refresh campaign progression from durable milestones.
17. Append the day audit record.
18. Validate runtime invariants.

Owners: `src/systems/simulation/`, `src/systems/strategic/`, `src/systems/progression.rs`. Execution order is a product contract; change it only with tests that establish the intended effect.

`advance_days_scratch` is the in-place variant for exclusively owned disposable branches (harness counterfactuals): identical day loop, no defensive copy, caller must discard state on failure.

Strategic scheduling (`src/systems/strategic/`):

- **Daily**: routes, crisis effects, AI business recovery, external route supply.
- **Weekly**: wage settlement, contracts, loans, civic debts, property rents (district-indexed, fire-discounted), employment, dividends, public works, relationship updates.
- **Monthly**: district conditions (value drift, 180 bp building repair), household living costs (scaled by desirability; market staples pay separately), institution selections, office duties/directives, AI objectives/upkeep/credit/legal, crisis detection.
- **Annual**: character health, succession, dynastic milestones.

Market clearing account (the market's internal cash pool):

- Credits: business purchases, operating/maintenance costs, player sponsorships, service spends, public-work costs, unowned-property proceeds, deposit flight, AI upkeep.
- Debits: business sales, vacancy income, crisis profiteering, office toll draws.

Weekly external regional income is outside silver paid directly to households, scaled by external-route health. The NobleDemand levy is the one exception: it leaves Rivergate with no internal counterparty.

AI acts on the same cadence via `recover_ai_businesses` (daily) and `advance_ai_objectives`, `apply_ai_dynasty_upkeep`, `advance_ai_credit_participation`, `file_grounded_ai_legal_cases`, `resolve_institution_selections` (monthly).

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
  -> release validation (including fingerprint)
  -> verify destination or CAS revision
  -> serialize AppState -> write + sync same-directory temp
  -> atomically replace destination (visibility commit)
  -> sync parent directory where supported (SaveOutcome)

load_state / load_state_with_revision
  -> read bounded file, compute SaveRevision
  -> reject duplicate JSON members
  -> require current schema_version
  -> deserialize AppState -> verify indexes/refs/fingerprint -> release validation
```

Owner: `src/persistence.rs`. Schema changes require increment, round-trip tests, rejection of non-current schemas, and `STATUS.md` updates. Atomic staging commits visibility before directory sync; CAS prevents stale multi-process overwrites.

### Projection and rendering

`build_state_summary` and `build_campaign_projection` derive read models from registry and state. `render_campaign_html` consumes the projection. `CampaignProjection::attention` is the single canonical attention classification; dashboard and CLI summary format that list.

Projection code may aggregate and format. It must not mutate state or recreate command validation.

### Gameplay harness

Generates state-derived candidates, validates via `apply_player_command` on clones, commits through the same API, advances via `advance_days`, and compares action vs no-action branches.

Independent campaigns run in parallel; each owns its `AppState` and the shared `Registry` is immutable, so ordering and determinism are preserved. The harness never directly mutates domain records.

Bounded work: `max_candidate_probes`, decision intervals, consequence horizons, and `trace_limit` bound every run. Wall-clock diagnostics (`simulated_days/s`) are advisory. See `GAMEPLAY_HARNESS.md`.

### Art

Owns rendering specifications, integer geometry/shading, sprite composition, encoding, automated review, and review HTML. It owns no campaign state or gameplay rules. Rendering is one-way: `CharacterSpec` → indexed `Canvas` → PNG bytes → embedded data URI. No round-trip is claimed.

Staged publication: every generated file — dashboard HTML, gameplay report JSON/HTML, art review HTML/PNG — writes to a synchronized same-directory temp and atomically replaces the destination via `write_generated_file`. A failed generation removes the temp or leaves the previous valid artifact untouched; no partial file is left at the final path.

Fidelity is explicit: HTML is standalone but not editable source, PNG is 8-bit indexed, report schemas are versioned and validated before rendering.

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
- Exclude wall-clock, environment, filesystem order, external services, and sleeps from core logic.
- Use fixed-point `Money`/`Quantity` with `i128` intermediates; no floating point participates in authored or simulated values.
- Execution envelope: same-process repeatability and same-build replay on the supported stable toolchain (`Cargo.toml` `rust-version` + `rust-toolchain.toml`). Cross-toolchain or cross-platform byte identity beyond fixed-point/behavioral equivalence is not claimed; `registry_fingerprint` and `schema_version` bind saves and fail closed on mismatch.

## Mutation and accounting contract

- Validate references, ownership, permissions, lifecycle, capacities, ranges, and arithmetic before mutation.
- Failed operations leave state unchanged.
- Calculate complete multi-record results before commit.
- Hot commits reserve every durable identifier before mutating, so the mutation phase is infallible. Failed reservation restores the allocator snapshot while state is untouched.
- Use `src/money.rs` fixed-point helpers with `i128` intermediates.
- Use shared checked scheduling helpers for future dates.
- Keep indexes, ownership, occupancy, collateral, employment, and lifecycle state synchronized.
- Represent durable external work in state before an adapter performs it.

## Invariant layers

Each layer guards what its owner can keep consistent.

1. **Type and visibility** — private fields, typed IDs (`src/ids.rs`), exhaustive enums, and `Option<T>` make unsupported mutation unrepresentable before runtime validation.
2. **System validation** — every consequential operation validates references, ownership, permission, lifecycle, capacity, ranges, and arithmetic before mutation and fails atomically on rejection.
3. **Runtime invariants** — `validate_invariants` (debug-only, zero cost in release via `prepare_invariant_ids` short-circuit) sweeps registry refs, synchronized indexes, lifecycle membership, numeric bounds, history monotonicity (`audit_log` days nondecreasing), and allocator coherence after every simulated day.
4. **Release persistence** — `validate_state` in `src/persistence.rs` re-proves the same properties plus schema/fingerprint/bounded-file checks on every load and before every save.

| Group | Debug runtime | Release persistence |
|---|---|---|
| Registry reference validity | `invariants::validate_*` via `RegistryIds` | `validate_definition_references` |
| Synchronized indexes & ownership exclusivity | index coherence asserts | `validate_primary_records` |
| Lifecycle agreement | `validate_characters` / `validate_institutions` | lifecycle checks |
| Numeric bounds & `Money`/`Quantity` ranges | `validate_numeric_ranges` | `validate_numeric_ranges` |
| History monotonicity & append-only `HistoryLog` | `validate_history` | `validate_history` |
| Allocator validity (`NextIds` exhaustion bands) | `validate_next_ids` | `validate_identifier_allocation` |
| Strategic scheduling (`is_schedulable_day`, settleability fortnight) | `is_settleable_weekly_due_day` | schedule checks |

## Extension map

| Change | Primary owner | Adjacent work |
|---|---|---|
| Immutable content | `src/registry/mod.rs` | Registry tests, bootstrap, projection if visible |
| Persistent state | `src/core/*`, `src/core/state.rs` | Bootstrap, validation, invariants, projection, tests |
| Player command | `src/systems/commands/` | Command tests, feedback, projection, harness, CLI smoke if needed |
| Daily rule | `src/systems/simulation/` | Simulation tests, ordering, invariants |
| Scheduled rule | `src/systems/strategic/` domain file | Strategic tests, feedback, snapshots |
| Cross-record transaction | Owning system or `src/systems/transactions.rs` | Typed errors, atomicity, stale-token tests |
| Read-only output | `src/projection.rs` | Projection/rendering tests |
| Save format | `src/persistence.rs` | Schema, validation, round trip, STATUS |
| Gameplay evaluation | `src/gameplay/` | Report schema, tests, `GAMEPLAY_HARNESS.md` |
| CLI syntax | `src/main.rs` | CLI smoke, README workflow if needed |
| Art primitive/subject/check | `src/art/*` | Determinism, review coverage, schema/STATUS for serialized output |

## Verification routing

`TESTING.md` owns lane selection; `ARCHITECTURE.md` extension map points to the narrowest proof per change class. Routine completion is `bash scripts/test.sh standard`; specialized surfaces add `soak` (long horizons), `adapters` (CLI), `gameplay`/`gameplay-audit` (harness matrices), and `docs` (link/doc consistency). Policy checks are `python tools/check_standards.py` and `python tools/check_no_github_actions.py` (no hosted CI; see `TESTING.md` § Completion gate).

Every lane is local-only and composes without hidden CI: `fast`/`standard` stay incremental, `soak`/`gameplay` run release-optimized but share the same assertions, and `standard` already reuses the debug CLI build across `docs`/`cli` sub-steps so an extra `fast` before `standard` adds no evidence (`AGENTS.md` completion guardrail).

BCA policy is `advisory` (see `AGENTS.md`); tooling lives in `scripts/test.sh` and `scripts/check_docs.py`.

No `unsafe` and no ambient mutable state: the crate declares no `unsafe` blocks and no `static mut`; `AppState.rng` is the sole randomness owner, and `HistoryLog` memo atomics are the only interior mutability — both documented in their owning modules (`rng.rs`, `history.rs`, `core/state.rs`).

Saves are bounded JSON (256 MiB) written atomically (same-directory temp → `persist` → optional parent-dir `sync_all` on Unix). `sync_save_directory` isolates the Unix fsync; failure degrades to `CommittedWithDegradedDurability` rather than silently switching semantics. No credentials or secret material is persisted.

## Public API

`src/lib.rs` defines the supported integration surface. Prefer stable operations there over exposing record internals. `PlayerCommand` in `src/systems/commands/` is the authoritative player mutation schema.
