# Civic Dynasty

Civic Dynasty is a deterministic Rust simulation of a merchant or artisan dynasty inside a living city economy. One detailed city scenario, Rivergate, combines businesses, households, markets, credit, property, institutions, law, public works, crises, family governance, and succession in a single persistent campaign state.

The core arc is:

```text
productive work -> commercial standing -> institutional access -> civic power -> dynastic continuity
```

## Documentation map

Each question is answered by exactly one owned document. Link to an owner instead of restating its contract.

| Document | Owns |
|---|---|
| `AGENTS.md` | Change procedure, module ownership, repository rules |
| `ARCHITECTURE.md` | Code structure, dependency direction, mutation flows, execution order |
| `STATUS.md` | Current capability, schemas, runtime guarantees, deliberate limits |
| `DESIGN.md` | Product intent, player fantasy, campaign arc, scope |
| `TESTING.md` | Test tiers, assertion standards, completion gates |
| `GAMEPLAY_HARNESS.md` | Harness mechanics, gameplay-report semantics |

Profiles implemented: **Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation**. Root `../AGENTS.md` owns workspace task leases and hygiene.

## Requirements

- Rust — the authoritative minimum version is `Cargo.toml` (`rust-version`), pinned for local development by `rust-toolchain.toml`. Edition 2024.
- Bash or PowerShell for repository scripts.
- Python for documentation and CLI structured-output checks.
- `cargo-audit` for the security gate (`ci-gates`, `deep` lanes only).

Verification is local-only; there are no hosted CI runners. The scripted lanes in `scripts/test.sh` (or `scripts/test.ps1` on Windows) are the authoritative gate. Optional local git hooks install with `bash scripts/install_hooks.sh`.

## Common commands

Test loop:

```bash
bash scripts/test.sh fast <filter>   # focused edit-test iteration
bash scripts/test.sh standard        # normal pre-commit gate
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
- Systems validate operations and perform canonical mutations.
- Adapters serialize, render, inspect, or invoke systems; they own no domain rules.

Determinism contract: given the same registry, state, seed, command sequence, and day count, execution produces identical state. `ARCHITECTURE.md` owns the full contract.

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
    commands/         PlayerCommand schema and dispatch by family
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

`ARCHITECTURE.md` owns the complete ownership map, including submodule-level responsibilities.

## Public library surface

`src/lib.rs` is the supported integration facade; the crate imports as `civic_dynasty`. Principal entry points:

| Operation | Entry point |
|---|---|
| Build Rivergate definitions | `build_rivergate_registry` |
| Create a campaign | `build_new_game` |
| Advance time | `advance_days` |
| Apply a player action | `apply_player_command` |
| Quote strategic actions | `quote_business_acquisition`, `quote_property_liquidation` |
| Save or load | `save_state`, `load_state` |
| Build read models | `build_state_summary`, `build_campaign_projection` |
| Render dashboard | `render_campaign_html` |
| Run gameplay analysis | `run_gameplay_harness`, `render_gameplay_report` |
| Render and review sprites | `build_art_review`, `build_art_review_report`, `render_art_review_html` |
| Check runtime invariants | `validate_invariants` |

`PlayerCommand` in `src/systems/commands/` is the authoritative player mutation schema.

## Working rule

Find the owner before changing behavior. Trace the public entry point to the canonical system, identify the state and invariants it owns, prove the change with the narrowest relevant tests, and update the one document that owns any changed contract. `AGENTS.md` defines the full procedure.
