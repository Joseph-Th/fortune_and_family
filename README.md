# Civic Dynasty

Deterministic Rust simulation of a merchant or artisan dynasty inside a living city economy. One city — Rivergate — combines businesses, households, markets, credit, property, institutions, law, public works, crises, family governance, and succession in a single persistent campaign state.

Core arc:

```text
productive work -> commercial standing -> institutional access -> civic power -> dynastic continuity
```

## Documentation map

Each question has one owning document. Link to the owner; do not duplicate its contract.

| Document | Owns |
|---|---|
| `AGENTS.md` | Change procedure, module ownership, repository rules |
| `ARCHITECTURE.md` | Code structure, dependency direction, mutation flows, execution order |
| `STATUS.md` | Current capability, schemas, runtime guarantees, deliberate limits |
| `DESIGN.md` | Product intent, player fantasy, campaign arc, scope |
| `TESTING.md` | Test tiers, assertion standards, completion gates |
| `GAMEPLAY_HARNESS.md` | Harness mechanics and report semantics |

Profiles: **Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation**. Root `../AGENTS.md` owns workspace coordination. BCA policy is `advisory`.

## Cold start

1. Read this map, then `AGENTS.md` for procedure and reading order.
2. Read `STATUS.md` for capability and schemas; read `ARCHITECTURE.md` for the owning module.
3. Trace `src/lib.rs` to the canonical system, then its file header (`Purpose / Owns / Reads / Mutates / Does not own / Canonical operations / Relevant invariants / Focused tests`).
4. Identify the narrowest test that proves the behavior before editing.
5. Run the narrowest `bash scripts/test.sh <lane>` from `TESTING.md` — `fast <filter>` to iterate, `standard` to complete.

## Requirements

- Rust `1.97` minimum (`Cargo.toml` `rust-version`, pinned by `rust-toolchain.toml`), edition 2024.
- Bash or PowerShell for `scripts/test.sh` / `scripts/test.ps1`.
- Python for doc and structured-output checks; `cargo-audit` for `ci-gates`/`deep` lanes.

Verification is local-only. `scripts/test.sh` is authoritative; `bash scripts/install_hooks.sh` installs optional git hooks.

## Common commands

```bash
bash scripts/test.sh fast <filter>   # focused edit-test loop
bash scripts/test.sh standard        # pre-commit gate
```

Campaign lifecycle:

```bash
cargo run --locked -- new --output saves/valeri.json --seed 42 --dynasty Valeri --founder "Elian Valeri" --background baker --advance 30
cargo run --locked -- simulate saves/valeri.json --days 360
cargo run --locked -- summary saves/valeri.json
cargo run --locked -- dashboard saves/valeri.json --output saves/valeri.html
cargo run --locked -- validate saves/valeri.json
cargo run --locked -- execute saves/valeri.json --command '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}'
```

Analysis and art:

```bash
cargo run --release --locked -- playtest
bash scripts/test.sh playtest --days 360 --persona entrepreneur
cargo run --locked -- art --output target/sprite-review.html --seeds 2 --scale 6
```

`cargo run --locked -- --help` documents CLI syntax. `TESTING.md` owns test gates.

## System model

```text
Registry definitions + AppState  ->  canonical systems  ->  adapters
```

- `Registry` — immutable Rivergate definitions, referenced by typed IDs.
- `AppState` — every mutable value required for deterministic continuation.
- Systems — validate and mutate state.
- Adapters — serialize, render, inspect, or invoke systems; own no domain rules.

Determinism: same `Registry` (fingerprint-bound) + serialized `AppState` (clock, RNG, allocators, records) + ordered explicit inputs (commands, day count) produces bit-identical successor state via state-owned RNG, `BTreeMap`-ordered iteration, typed-ID tie-breakers, and fixed-point `Money`/`Quantity`. Full contract: `ARCHITECTURE.md` § Determinism contract.

Failure: consequential operations validate before mutation, preserve state on rejection, and report typed `CommandError` / `SimulationError` / `PersistenceError` variants. Multi-record work resolves the full result before commit or uses a consumed `Validated*` token with revalidation; stale tokens fail closed.

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
scripts/              Test runner, smoke groups, doc and gameplay checks, git hooks
```

`ARCHITECTURE.md` owns the complete map.

## Public library surface

`src/lib.rs` is the integration facade (`civic_dynasty`):

| Operation | Entry point |
|---|---|
| Build definitions | `build_rivergate_registry` |
| Create campaign | `build_new_game` |
| Advance time | `advance_days` |
| Apply player action | `apply_player_command` |
| Quote strategic actions | `quote_business_acquisition`, `quote_property_liquidation` |
| Save / load | `save_state`, `load_state` |
| Read models | `build_state_summary`, `build_campaign_projection` |
| Render dashboard | `render_campaign_html` |
| Gameplay analysis | `run_gameplay_harness`, `render_gameplay_report` |
| Sprite review | `build_art_review`, `render_art_review_html` |
| Check invariants | `validate_invariants` |

`PlayerCommand` in `src/systems/commands/` is the authoritative player mutation schema.

## Working rule

Find the owner before changing behavior. Trace the public entry point to its canonical system, prove the change with the narrowest tests, and update the one owning document. `AGENTS.md` defines the procedure.

## Verification

Local only; no hosted CI. `bash scripts/test.sh <lane>` (or `.\scripts\test.ps1 <lane>` on Windows) is authoritative — see `TESTING.md` for lane selection. Current docs and tests must stay consistent (`bash scripts/test.sh docs`, `python scripts/check_docs.py`). Workspace policy:

```bash
python ../tools/check_no_github_actions.py   # must pass before completion
python ../tools/check_standards.py          # portfolio structural checks
```

- `fast` / `standard` are routine gates; `soak` / `gameplay` / `adapters` add only their owned contracts (see `TESTING.md` § Completion gate).
- `standard` reuses the debug CLI build and includes `docs`; no extra `fast` before it.
- No `unsafe`, no ambient mutable state, no credentials. Persistence side-effects are bounded saves and staged artifacts described in `ARCHITECTURE.md` and `STATUS.md`.
