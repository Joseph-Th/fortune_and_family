# Civic Dynasty

Civic Dynasty is a deterministic Rust simulation of a merchant or artisan dynasty operating inside a living city economy. Rivergate combines businesses, households, markets, credit, property, institutions, law, public works, crises, family governance, and succession in one persistent campaign state.

The core arc is:

```text
productive work -> commercial standing -> institutional access -> civic power -> dynastic continuity
```

## Start here

For a new contributor or agent, use this reading order:

| Need | Read |
|---|---|
| Repository orientation and commands | `README.md` |
| Ownership, dependency direction, mutation paths, execution order | `ARCHITECTURE.md` |
| Safe change procedure and repository rules | `AGENTS.md` |
| Product intent and design constraints | `DESIGN.md` |
| Current schemas, capabilities, API surface, and limits | `STATUS.md` |
| Test tiers and assertion standards | `TESTING.md` |
| Gameplay-agent analysis and report semantics | `GAMEPLAY_HARNESS.md` |

Each contract has one owner. Link to the owning document instead of restating it elsewhere.

Before editing:

```bash
git status --short
bash scripts/test.sh fast
```

Preserve unrelated working-tree changes.

## Mental model

```text
Registry definitions + AppState
            |
            v
       canonical systems
            |
            v
persistence / CLI / projections / HTML / gameplay analysis / art
```

- `Registry` contains immutable Rivergate definitions.
- `AppState` contains all mutable state required for deterministic continuation.
- Records contain identity, references, local values, and lifecycle state.
- Systems validate and perform canonical mutations.
- Adapters serialize, render, inspect, or invoke systems. They do not own domain rules.

Given the same registry, state, seed, command sequence, and day count, the simulation must produce identical state.

## Requirements

- Rust 1.97 or newer
- Bash for repository scripts
- Python for documentation and CLI structured-output checks
- `cargo-audit` for the complete security gate

The crate uses Rust 2024 and has no runtime service dependency.

## Common commands

Fast edit-test loop:

```bash
bash scripts/test.sh fast <filter>
```

Normal pre-commit loop:

```bash
bash scripts/test.sh standard
```

Create and inspect a campaign:

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
cargo run --locked -- inspect saves/valeri.json
cargo run --locked -- dashboard saves/valeri.json --output saves/valeri.html
cargo run --locked -- validate saves/valeri.json
```

Apply a player command:

```bash
cargo run --locked -- execute saves/valeri.json \
  --command '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}'
```

Run gameplay analysis or sprite review:

```bash
cargo run --release --locked -- playtest
cargo run --locked -- art --output target/sprite-review.html --seeds 2 --scale 6
```

Use `cargo run --locked -- --help` or a subcommand with `--help` for CLI syntax. `TESTING.md` owns test commands and completion gates.

## Repository map

```text
src/
  core/
    records.rs        Core population and economic records
    extended.rs       Strategic, civic, family, finance, and relationship records
    state.rs          AppState, clock, synchronized stores, ID allocation
  registry/mod.rs     Immutable Rivergate definitions
  ids.rs              Typed persistent IDs
  money.rs            Fixed-point Money and Quantity
  rng.rs              Serializable deterministic RNG
  systems/
    bootstrap.rs      New campaign construction
    commands.rs       PlayerCommand schema and dispatch
    legal.rs          Grounded legal claims
    progression.rs    Campaign progression milestones
    simulation.rs     Daily simulation pipeline
    strategic.rs      Scheduled and cross-domain systems
    transactions.rs   Reusable validated transactions
    invariants.rs     Runtime invariant checks
  persistence.rs      Current-schema save/load and release validation
  projection.rs       Read-only projections and HTML dashboard
  gameplay.rs         Deterministic gameplay harness
  art/                Procedural sprite renderer and review harness
  main.rs             CLI adapter
  *_tests.rs          Large sibling test suites
  test_support.rs     Shared deterministic fixtures and diagnostics
scripts/
  check_docs.py       Documentation consistency checks
  test.sh             Test tier runner
  verify_cli.sh       CLI smoke groups
```

## Supported library entry points

`src/lib.rs` is the authoritative public facade. The main operations are:

| Operation | Entry point |
|---|---|
| Build Rivergate definitions | `build_rivergate_registry` |
| Create a campaign | `build_new_game` |
| Advance time | `advance_days` |
| Apply a player action | `apply_player_command` |
| Quote supported strategic actions | `quote_business_acquisition`, `quote_property_liquidation` |
| Save or load | `save_state`, `load_state` |
| Build read models | `build_state_summary`, `build_campaign_projection` |
| Render dashboard | `render_campaign_html` |
| Run gameplay analysis | `run_gameplay_harness`, `render_gameplay_report` |
| Render and review sprites | `build_art_review`, `build_art_review_report`, `render_art_review_html` |
| Check runtime invariants | `validate_invariants` |

`PlayerCommand` in `src/systems/commands.rs` is the authoritative player mutation schema.

## Working rule

Find the owner before changing behavior. Trace the public entry point to the canonical system, identify the state and invariants it owns, run the narrowest relevant tests, and update the one document that owns any changed contract.
