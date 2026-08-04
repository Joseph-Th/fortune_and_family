# Civic Dynasty

Civic Dynasty is a deterministic dynasty and political-economy simulation written in Rust. The repository contains a complete headless Rivergate campaign engine, a CLI, versioned JSON persistence, read-only projections, a self-contained HTML dashboard, and a deterministic gameplay-testing harness.

## Documentation map

| Document | Use it for |
|---|---|
| `README.md` | Build, run, inspect, and navigate the repository. |
| `ARCHITECTURE.md` | Understand state ownership, module boundaries, canonical flows, and extension points. |
| `AGENTS.md` | Follow repository rules when changing code. |
| `DESIGN.md` | Understand product intent, player fantasy, scope, and design constraints. |
| `STATUS.md` | Check current implementation coverage, supported schemas, and deliberate gaps. |
| `TESTING.md` | Select the correct test tier and write maintainable tests. |
| `GAMEPLAY_HARNESS.md` | Run and interpret deterministic player-agent analysis. |

A cold implementation agent should read `ARCHITECTURE.md`, `AGENTS.md`, and the relevant section of `STATUS.md` before editing.

## Requirements

- Rust 1.97 or newer
- Bash for the repository test scripts
- Python for CLI JSON smoke validation
- `cargo-audit` for the full security gate

The crate uses Rust 2024 and has no runtime service dependencies.

## Quick start

Run the test suite:

```bash
bash scripts/test.sh fast
```

Create a campaign:

```bash
cargo run --locked -- new \
  --output saves/valeri.json \
  --seed 42 \
  --dynasty Valeri \
  --founder "Elian Valeri" \
  --background baker \
  --advance 30
```

Advance and inspect it:

```bash
cargo run --locked -- simulate saves/valeri.json --days 360
cargo run --locked -- summary saves/valeri.json
cargo run --locked -- inspect saves/valeri.json
cargo run --locked -- dashboard saves/valeri.json --output saves/valeri.html
cargo run --locked -- validate saves/valeri.json
```

Starting backgrounds are `baker`, `cloth-trader`, and `blacksmith`.

## Library use

```rust
use civic_dynasty::{
    NewGameConfig, advance_days, build_campaign_projection, build_new_game,
    build_rivergate_registry,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = build_rivergate_registry();
    let mut state = build_new_game(&registry, NewGameConfig::default())?;
    advance_days(&registry, &mut state, 30)?;
    let projection = build_campaign_projection(&registry, &state);

    println!("{}", projection.scenario.name);
    Ok(())
}
```

The supported library facade is defined in `src/lib.rs`. Primary entry points are:

- `build_rivergate_registry`
- `build_new_game`
- `advance_days`
- `apply_player_command`
- `quote_business_acquisition`
- `build_state_summary`
- `build_campaign_projection`
- `render_campaign_html`
- `save_state` and `load_state`
- `run_gameplay_harness` and `render_gameplay_report`
- `validate_invariants`

## Player commands

Commands use Serde's externally tagged JSON representation:

```bash
cargo run --locked -- execute saves/valeri.json \
  --command '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}'
```

`PlayerCommand` in `src/systems/commands.rs` is the authoritative command schema. It covers business capitalization and acquisition, operating policy, cash transfers, contracts, private loans, municipal debt authorization, property, civic actions, legal cases, family governance, ward adoption, focused family education, office nomination, crisis response, labor disputes, and notification acknowledgement.

Run `cargo run --locked -- --help` for CLI syntax.

## Determinism and persistence

All consequential runtime state is owned by serializable `AppState`. Simulation randomness comes from the state-owned deterministic RNG. Ordered collections and typed-ID tie-breakers make result-affecting iteration stable.

Given the same registry, state, seed, command sequence, and day count, the engine must produce identical state.

Campaigns are human-readable JSON. The current save schema and supported migrations are listed in `STATUS.md`. Saves are validated before writing and after loading. Writes use a synchronized same-directory temporary file followed by atomic replacement.

## Architecture summary

The repository follows a Registry / AppState / Record / System model:

- `Registry` contains immutable Rivergate definitions.
- `AppState` contains mutable campaign state, indexes, RNG state, IDs, and histories.
- Records contain identity, references, local values, and lifecycle state.
- Systems validate operations and perform canonical mutations.
- Persistence, CLI, projections, rendering, and gameplay analysis are adapters around the core systems.

See `ARCHITECTURE.md` for the full module map and execution flows.

## Repository map

```text
src/
  core/
    records.rs        Primary runtime records
    extended.rs       Strategic and civic records
    state.rs          AppState, stores, clock, and ID allocation
  registry/mod.rs     Immutable Rivergate definitions
  systems/
    bootstrap.rs      New campaign construction
    commands.rs       Player command API
    simulation.rs     Daily economic pipeline
    strategic.rs      Weekly, monthly, and annual systems
    transactions.rs   Validated transfer primitives and errors
    invariants.rs     Debug runtime invariants
  persistence.rs      Versioned JSON save/load and migrations
  projection.rs       Read-only projections and HTML rendering
  gameplay.rs         Deterministic player-agent harness
  main.rs             CLI adapter
  *_tests.rs          Large sibling test suites
  test_support.rs     Shared deterministic fixtures
scripts/
  test.sh             Test tier runner
  verify_cli.sh       End-to-end CLI smoke suite
```

## Gameplay analysis

Run the default release-mode harness:

```bash
cargo run --release --locked -- playtest
```

Run a focused campaign:

```bash
cargo run --release --locked -- playtest \
  --days 360 \
  --persona entrepreneur \
  --background baker \
  --trace-limit 20
```

Run a multi-seed JSON report:

```bash
cargo run --release --locked -- playtest \
  --start-seed 1 \
  --seeds 10 \
  --days 1080 \
  --json \
  --output gameplay-report.json
```

The harness selects state-derived commands through the same command API used by the CLI, compares action and no-action branches, and reports reachability, consequences, feedback, resilience, traces, and explicit findings. See `GAMEPLAY_HARNESS.md`.

## Verification

Use the fast tier while editing and the complete scripted tier before broad review:

```bash
bash scripts/test.sh fast
bash scripts/test.sh all
```

`TESTING.md` is the authoritative reference for filters, exact test selection, soak coverage, CLI validation, test layout, and the full completion gate.

## Product scope

The engine focuses on one deeply simulated city and abstract regional connections. Its core loop converts productive competence into commercial leverage, social standing, political office, institutional power, and dynastic continuity. Officeholding is not a free upgrade: public service consumes administrative capacity, requires recurring private civic contributions, and can be forfeited when a dynasty repeatedly fails its duties.

Tactical combat, manual movement of every character, equal-detail multi-city simulation, repetitive crafting, routine dialogue trees, and decorative interiors without systemic effects are outside the project scope. See `DESIGN.md` for the product contract.
