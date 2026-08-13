# Agent Guide

This file defines how to modify the repository safely. It is an execution contract. Product intent belongs in `DESIGN.md`; implementation ownership belongs in `ARCHITECTURE.md`; current capability belongs in `STATUS.md`; test policy belongs in `TESTING.md`.

## Cold start

Before editing:

1. Run `git status --short`; preserve unrelated changes.
2. Read `README.md` and the relevant section of `ARCHITECTURE.md`.
3. Identify the owning source module and sibling test suite.
4. Trace the canonical path from public entry point to mutation and invariant validation.
5. Run the narrowest relevant test before changing behavior.

Primary owners:

| Concern | Owner |
|---|---|
| Campaign construction | `src/systems/bootstrap.rs` |
| Player commands | `src/systems/commands.rs` |
| Daily simulation | `src/systems/simulation.rs` |
| Scheduled/cross-domain systems | `src/systems/strategic.rs` |
| Legal claim grounding | `src/systems/legal.rs` |
| Campaign progression | `src/systems/progression.rs` |
| Persistence and schema validation | `src/persistence.rs` |
| Read models and HTML | `src/projection.rs` |
| Gameplay analysis | `src/gameplay.rs` |
| Procedural art | `src/art/*` |

## Non-negotiable contracts

### Ownership

- `Registry` owns immutable authored definitions.
- `AppState` owns mutable, serializable campaign state.
- Records own identity, references, local values, and lifecycle state.
- Systems own validation and mutation.
- Adapters translate external input and output only.

Do not put mutable runtime state in the registry. Do not put domain rules in the CLI, persistence, projections, rendering, tests, or report formatting.

### Canonical mutation

Consequential operations follow one path:

```text
input -> validation -> resolution -> atomic commit -> durable feedback -> invariant check
```

Validate references, ownership, permissions, lifecycle, capacities, ranges, and arithmetic before mutation. Failed operations must leave state unchanged.

For multi-record operations, calculate the complete result before committing. If validation and commit are separated, use a consumed `Validated*` value and revalidate current state during commit.

### Determinism

The same registry, state, seed, command sequence, and day count must produce identical state.

- Use the state-owned RNG for simulation randomness.
- Use ordered collections, explicit sorting, and typed-ID tie-breakers for result-affecting iteration.
- Do not use wall-clock time, sleeps, environment-dependent values, external services, or unordered filesystem enumeration in core systems.

### Arithmetic and time

- Economic state uses `Money` and `Quantity` from `src/money.rs`.
- Do not use floating point for economic state.
- Use wide intermediates for multiply-then-divide calculations.
- Reject overflow before player-initiated and cross-record transfers.
- Saturation is not a substitute for a domain bound.
- Use the shared checked scheduling helpers for future runtime dates.

### Identity and derived state

- Persistent references use typed IDs from `src/ids.rs`.
- Generated IDs come from `AppState` allocators.
- Optional references use `Option`.
- Raw strings are for authored keys and user-facing text, not runtime identity.
- Update authoritative records and every owned derived index in the same atomic operation.
- Related lifecycle state must remain synchronized across ownership, occupancy, collateral, employment, office, debt, and succession records.

### Boundaries

Core systems perform no implicit IO. Persistence, CLI, projections, HTML, gameplay reports, and art are boundary layers. Durable external work must first be represented in state.

## Change map

| Change | Required work |
|---|---|
| Persistent state | Record/store ownership, IDs if needed, bootstrap, persistence validation, invariants, projection if visible, round-trip and invalid-state tests, schema increment when serialized shape changes. |
| Player command | Exhaustive `PlayerCommand` variant, typed errors, complete preflight, canonical commit, durable feedback when consequential, success/rejection/atomicity tests, projection and gameplay integration when observable. |
| Simulation behavior | Explicit cadence, canonical owner, causal ordering, focused tests, durable feedback when player-relevant, invariants, soak coverage for accumulating effects. |
| Persistence contract | Schema increment, current-schema enforcement, current round trip, release validation, atomic write behavior. |
| Projection/report field | Immutable derivation only, schema update when serialized contract changes, focused output tests. |
| Gameplay behavior | Candidate generation/classification, snapshots, consequence attribution, findings/traces when applicable, harness tests, `GAMEPLAY_HARNESS.md` if the harness contract changes. |
| Art behavior | Integer/fixed-point rendering, deterministic coverage, lint rule for automatable defect classes, art report schema update when serialized shape changes. |
| CLI syntax | `src/main.rs`, CLI smoke coverage, `README.md` when the common workflow changes. |

## Code conventions

- Match project-owned enums exhaustively; avoid wildcard arms.
- Keep consequential fields private and mutate them through systems.
- Prefer concrete records and functions over dynamic dispatch in core domains.
- Keep top-level execution order visible.
- Pass the narrowest mutable context required.
- Delete superseded paths instead of keeping inactive compatibility layers.
- Give every source file a concise `//!` module description.
- Use dedicated error enums with contextual variants; avoid stringly typed domain errors.

Naming:

| Intent | Form |
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
| Deferred validation | `validate_*` returning `Validated*` |

Test fixtures may use `make_test_*`.

## Tests

Use sibling `*_tests.rs` files for large suites. Test canonical behavior, not helper reachability. `TESTING.md` owns tier selection, assertion standards, and completion gates.

During development:

```bash
bash scripts/test.sh fast <filter>
```

Before a normal commit:

```bash
bash scripts/test.sh standard
```

Run the complete scripted test tier for cross-cutting changes:

```bash
bash scripts/test.sh all
```

Then follow the full completion gate in `TESTING.md`, which also includes formatting, all-target checks, Clippy, rustdoc warnings, release tests, security audit, and diff hygiene.

## Documentation

Update documentation in the same change when architecture, behavior, schemas, public APIs, commands, test workflow, harness semantics, or deliberate scope changes.

Keep documentation current-state and forward-facing. Describe the contract that exists, not the sequence of fixes that produced it. Put each contract in its owning document and link to it from other documents.
