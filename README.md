# Civic Dynasty

Deterministic Rust simulation of a merchant or artisan dynasty inside a living city economy. One detailed city, Rivergate, combines businesses, households, markets, credit, property, institutions, law, public works, crises, family governance, and succession in a single persistent campaign state.

Core arc:

```text
productive work -> commercial standing -> institutional access -> civic power -> dynastic continuity
```

## Documentation map

Each question is answered by one owned document. Link to the owner instead of restating its contract.

| Document | Owns |
|---|---|
| `AGENTS.md` | Change procedure, module ownership, repository rules |
| `ARCHITECTURE.md` | Code structure, dependency direction, mutation flows, execution order |
| `STATUS.md` | Current capability, schemas, runtime guarantees, deliberate limits |
| `DESIGN.md` | Product intent, player fantasy, campaign arc, scope |
| `TESTING.md` | Test tiers, assertion standards, completion gates |
| `GAMEPLAY_HARNESS.md` | Harness mechanics, report semantics |

Profiles: **Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation**. Root `../AGENTS.md` owns workspace coordination, task leases, and filesystem hygiene. BCA policy is `advisory` (see `AGENTS.md`).

## Cold start

1. Read this map, then `AGENTS.md` for the change procedure and required reading order.
2. Read `STATUS.md` for current capability (schemas, guarantees, limits) and `ARCHITECTURE.md` for the owning module.
3. Trace `src/lib.rs` to the canonical system, then its file header (`Purpose / Owns / Reads / Mutates / Does not own / Canonical operations / Relevant invariants / Focused tests`).
4. Identify the narrowest test that proves the behavior before editing.
5. Run the narrowest `bash scripts/test.sh <lane>` from `TESTING.md` — `fast <filter>` for iteration, `standard` for completion.

## Requirements

- Rust — minimum version in `Cargo.toml` (`rust-version`), pinned by `rust-toolchain.toml`. Edition 2024.
- Bash or PowerShell for repository scripts.
- Python for documentation and structured-output checks.
- `cargo-audit` for the security gate (`ci-gates`, `deep` lanes).

Verification is local-only. The scripted lanes in `scripts/test.sh` (or `scripts/test.ps1` on Windows) are the authoritative gate. Install optional local git hooks with `bash scripts/install_hooks.sh`.

## Common commands

```bash
bash scripts/test.sh fast <filter>   # focused edit-test loop
bash scripts/test.sh standard        # pre-commit gate
```

Create, advance, and inspect a campaign:

```bash
cargo run --locked -- new \
  --output saves/valeri.json \
  --seed 42 \
  --dynasty Valeri \
  --founder "Elian Valeri" \
  --background baker \
  --advance 30

cargo run --locked -- simulate saves/valeri.json --days 360
cargo run --locked -- summary saves/valeri.json
cargo run --locked -- dashboard saves/valeri.json --output saves/valeri.html
cargo run --locked -- validate saves/valeri.json
```

Apply a player command:

```bash
cargo run --locked -- execute saves/valeri.json \
  --command '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}'
```

Gameplay analysis and sprite review:

```bash
cargo run --release --locked -- playtest
bash scripts/test.sh playtest --days 360 --persona entrepreneur
cargo run --locked -- art --output target/sprite-review.html --seeds 2 --scale 6
```

`cargo run --locked -- --help` documents CLI syntax. `TESTING.md` owns test commands and completion gates.

## System model

```text
Registry definitions + AppState
            |
            v
     canonical systems
            |
            v
persistence / CLI / projections / HTML / gameplay analysis / art
```

- `Registry` holds immutable Rivergate definitions; records reference them through typed IDs.
- `AppState` holds every mutable value required for deterministic continuation.
- Systems validate and mutate state.
- Adapters serialize, render, inspect, or invoke systems; they own no domain rules.

Determinism contract: same registry (fingerprint-bound), state, seed, command sequence,
and day count produce bit-identical successor `AppState` via state-owned RNG,
`BTreeMap`-ordered iteration, typed-ID tie-breakers, and fixed-point
`Money`/`Quantity`. Full contract is in `ARCHITECTURE.md` § Determinism contract.

Failure semantics: consequential operations validate before mutation, preserve
state on rejection, and report typed `CommandError`/`SimulationError`/`PersistenceError`
variants with relevant fields.

## Repository map

```text
src/
  core/               Records, AppState, clock, stores, ID allocation
  registry/mod.rs     Immutable Rivergate definitions
  ids.rs              Typed persistent IDs
  money.rs            Fixed-point Money and Quantity
  rng.rs              Serializable deterministic RNG
  systems/
    bootstrap.rs      New campaign construction
    commands/         PlayerCommand schema and dispatch
    simulation.rs     Daily simulation pipeline
    strategic/        Scheduled cross-domain systems by domain
    legal.rs          Grounded legal claims
    progression.rs    Campaign progression milestones
    transactions.rs   Reusable validated transactions
    invariants.rs     Runtime invariant checks
  persistence.rs      Current-schema save/load and release validation
  projection.rs       Read-only projections and HTML dashboard
  gameplay/           Deterministic gameplay harness
  art/                Procedural sprite renderer and review harness
  main.rs             CLI adapter
  *_tests.rs          Sibling test suites
  test_support.rs     Shared fixtures and diagnostics
scripts/              Test runner, smoke groups, docs and gameplay checks, git hooks
```

`ARCHITECTURE.md` owns the complete ownership map.

## Determinism and failure contracts

- **Determinism:** same `Registry` fingerprint + serialized `AppState` (clock,
  RNG, allocators, records) + ordered explicit inputs (commands + day count)
  → bit-identical successor state. See `ARCHITECTURE.md` § Determinism contract
  for the full guarantee: state-owned `DeterministicRng`, ordered `BTreeMap`
  iteration, typed-ID tie-breakers, fixed-point `Money`/`Quantity` with `i128`
  intermediates, and no OS entropy / wall-clock dependency.
- **Failure:** every consequential operation validates before mutation, leaves
  state unchanged on rejection, and reports a typed
  `CommandError`/`SimulationError`/`PersistenceError` variant with relevant
  fields. Multi-record work resolves the complete result before commit or uses
  a consumed `Validated*` token with revalidation; stale tokens fail closed.

## Public library surface

`src/lib.rs` is the supported integration facade (`civic_dynasty`). Principal entry points:

| Operation | Entry point |
|---|---|
| Build definitions | `build_rivergate_registry` |
| Create campaign | `build_new_game` |
| Advance time | `advance_days` |
| Apply player action | `apply_player_command` |
| Quote strategic actions | `quote_business_acquisition`, `quote_property_liquidation` |
| Save or load | `save_state`, `load_state` |
| Build read models | `build_state_summary`, `build_campaign_projection` |
| Render dashboard | `render_campaign_html` |
| Run gameplay analysis | `run_gameplay_harness`, `render_gameplay_report` |
| Render and review sprites | `build_art_review`, `build_art_review_report`, `render_art_review_html` |
| Check invariants | `validate_invariants` |

`PlayerCommand` in `src/systems/commands/` is the authoritative player mutation schema.

## Working rule

Find the owner before changing behavior. Trace the public entry point to its canonical system, prove the change with the narrowest tests, and update the one owning document. `AGENTS.md` defines the procedure.

## Verification

All verification is local; no hosted CI. `bash scripts/test.sh <lane>` (or `.\scripts\test.ps1 <lane>` on Windows) is authoritative — see `TESTING.md` for lane selection. Current docs/tests must stay consistent (`bash scripts/test.sh docs`, `python scripts/check_docs.py`). Workspace policy:

```bash
python ../tools/check_no_github_actions.py   # must pass before completion
python ../tools/check_standards.py          # portfolio structural checks
```

- `fast` / `standard` are the routine developer gates; `soak` / `gameplay` / `adapters` are specialized and add only the contracts they own (see `TESTING.md` § Completion gate).
- `standard` already reuses the debug CLI build and includes `docs`; do not run an extra `fast` before it.
- No `unsafe`, no ambient mutable state, and no credentials/tokens are present; the only persistence side-effects are bounded saves and staged generated artifacts described in `ARCHITECTURE.md` and `STATUS.md`.
