# Civic Dynasty

Civic Dynasty is a deterministic dynasty, economic, and political simulation written in Rust. The repository contains the Rivergate simulation engine, a CLI, versioned JSON saves, read-only projections, a self-contained HTML dashboard, and a deterministic gameplay-analysis harness.

The central gameplay arc is:

```text
productive work -> commercial standing -> institutional access -> civic power -> dynastic continuity
```

## Start here

For a new contributor or agent:

1. Read this file for the project map and commands.
2. Read `ARCHITECTURE.md` before changing cross-domain behavior.
3. Read `AGENTS.md` before editing code.
4. Read the relevant section of `STATUS.md` to confirm current capability and limits.
5. Use `DESIGN.md`, `TESTING.md`, or `GAMEPLAY_HARNESS.md` only for the task they own.

Before editing, run:

```bash
git status --short
bash scripts/test.sh fast
```

Preserve unrelated working-tree changes.

## Documentation map

| Document | Authority |
|---|---|
| `README.md` | Setup, commands, repository navigation, and entry points. |
| `ARCHITECTURE.md` | State ownership, dependency direction, mutation flows, execution order, and extension points. |
| `AGENTS.md` | Repository rules and change procedures. |
| `DESIGN.md` | Product fantasy, gameplay loop, design constraints, and scope. |
| `STATUS.md` | Current schemas, implemented systems, public surface, and deliberate limits. |
| `TESTING.md` | Test tiers, test design, and completion gates. |
| `GAMEPLAY_HARNESS.md` | Player-agent analysis, report semantics, and harness integration. |

Do not duplicate a contract across documents. Link to the owning document instead.

## Requirements

- Rust 1.97 or newer
- Bash for repository scripts
- Python for CLI JSON smoke validation
- `cargo-audit` for the complete security gate

The crate uses Rust 2024 and has no runtime service dependency.

## Quick start

Run the fast library suite:

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

Apply a player command:

```bash
cargo run --locked -- execute saves/valeri.json \
  --command '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}'
```

Render and review sprites:

```bash
cargo run --locked -- art --output target/sprite-review.html --seeds 2 --scale 6
```

The review page is self-contained: it plays every clip, magnifies every frame, shows palettes, offers silhouette and pixel-grid toggles, and lists automated findings. Add `--json` for the machine-readable report and `--fail-on-critical` to gate on it.

Run `cargo run --locked -- --help` or a subcommand with `--help` for the authoritative CLI syntax. Starting backgrounds are `baker`, `cloth-trader`, and `blacksmith`.

## Core model

The codebase follows a Registry / AppState / Record / System model:

- `Registry` owns immutable Rivergate definitions.
- `AppState` owns all mutable and serializable campaign state.
- Records own identity, references, local values, and lifecycle state.
- Systems validate and perform canonical mutations.
- Persistence, CLI, projections, rendering, and gameplay analysis are boundary adapters.

The same registry, state, seed, command sequence, and day count must produce identical state.

## Repository map

```text
src/
  core/
    records.rs        Primary population and economic records
    extended.rs       Strategic, civic, family, finance, and relationship records
    state.rs          AppState, synchronized stores, clock, and ID allocation
  registry/mod.rs     Immutable Rivergate definitions
  systems/
    bootstrap.rs      New campaign construction
    commands.rs       Player command schema and dispatch
    simulation.rs     Daily economic pipeline
    strategic.rs      Weekly, monthly, annual, and cross-domain systems
    transactions.rs   Reusable validated transaction primitives
    invariants.rs     Debug runtime invariants
  persistence.rs      Save/load, migrations, and release validation
  projection.rs       Read-only projections and HTML rendering
  gameplay.rs         Deterministic gameplay harness
  art/
    color.rs          Color model, shading ramps, and palettes
    math.rs           Fixed-point angles and trigonometry
    canvas.rs         Indexed pixel buffers
    surface.rs        Material, light, and depth buffers
    shape.rs          Shaded rasterization primitives
    rig.rs            Skeletons, poses, and the humanoid rig
    anim.rs           Keyframed animation clips
    sprite.rs         Character specifications and sheet composition
    png.rs            Indexed PNG encoding
    lint.rs           Automated sprite review checks
    harness.rs        Visual review harness and HTML contact sheet
  main.rs             CLI adapter
  *_tests.rs          Large sibling test suites
  test_support.rs     Shared deterministic fixtures and diagnostics
scripts/
  check_docs.py       Documentation contract consistency checks
  test.sh             Test tier runner
  verify_cli.sh       End-to-end CLI smoke suite
```

## Primary entry points

The supported library facade is exported from `src/lib.rs`.

| Operation | Entry point |
|---|---|
| Build definitions | `build_rivergate_registry` |
| Create campaign | `build_new_game` |
| Advance time | `advance_days` |
| Apply player action | `apply_player_command` |
| Quote strategic acquisitions or liquidation | `quote_business_acquisition`, `quote_property_liquidation` |
| Save or load | `save_state`, `load_state` |
| Build read models | `build_state_summary`, `build_campaign_projection` |
| Render dashboard | `render_campaign_html` |
| Run gameplay analysis | `run_gameplay_harness`, `render_gameplay_report` |
| Render and review sprites | `build_art_review`, `render_art_review_html`, `build_art_review_report` |
| Check runtime invariants | `validate_invariants` |

`PlayerCommand` in `src/systems/commands.rs` is the authoritative player-command schema.

## Library example

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

## Verification

Use focused tests while editing and the complete gate before finishing cross-cutting work:

```bash
bash scripts/test.sh fast <filter>
bash scripts/test.sh docs
bash scripts/test.sh all
```

`TESTING.md` defines the full workflow. `GAMEPLAY_HARNESS.md` defines when systemic player-agent analysis is required.
