# Civic Dynasty

Civic Dynasty is a deterministic dynasty, economic, political, and social strategy simulation written in Rust. The repository contains a complete headless Rivergate game engine, a command-line client, a read-only projection API, a self-contained HTML campaign dashboard, and a deterministic gameplay-testing harness.

The implementation follows the Registry / AppState / Record / System architecture defined in `AGENTS.md`.

## What is implemented

The Rivergate campaign includes:

- Six persistent urban districts and grouped ordinary households.
- Eight competing dynasties with heads, heirs, family councils, governance, education, marriage links, succession, reputation, legitimacy, and administrative capacity.
- Ten goods and connected grain, brewing, textile, timber, fuel, iron, and tool production chains.
- Businesses with ownership, managers, employees, policies, cash, inventory, condition, quality, maintenance, distress, insolvency, and recovery.
- Explicit employment agreements, wages, loyalty, workplace conditions, disputes, and player responses.
- Property ownership separated from enterprise ownership, including residences, workshops, warehouses, rents, occupancy, value, condition, purchases, and collateral.
- Supply contracts with quantities, prices, schedules, fulfillment, breach, penalties, and termination.
- Loans with principal, interest, repayment schedules, delinquency, default, collateral seizure, and repayment.
- Eleven civic, guild, legal, security, market, treasury, and religious institutions with membership, powers, budgets, legitimacy, officeholders, and deterministic elections.
- Persistent enacted laws that alter price ceilings, imports, interest limits, and other economic conditions.
- Multidimensional dynasty relationships, obligations, resentment, memories, and marriage effects.
- Information reports with source, confidence, creation, expiry, and causal summaries.
- Traceable AI objectives covering property, supply, office, legitimacy, debt, cash, and rival containment.
- District employment, sanitation, safety, rent pressure, food satisfaction, unrest, and public works.
- Legal cases with parties, evidence, hearings, damages, and deterministic judgments.
- Regional trade routes with capacity, tolls, risk, disruption, and market supply.
- Systemic grain, banking, fire, epidemic, trade, guild, and external-authority crises.
- Durable player notifications, chronicle records, and audit records.
- A complete read-only campaign projection for dynasty, relationship, district, market, contract, finance, property, institution, law, court, crisis, information, and notification views.
- A self-contained dashboard that HTML-escapes visible content and safely encodes embedded JSON against script-block breakout.

## Deterministic simulation

All consequential state lives in `AppState` and is serializable. All randomness comes from the state-owned deterministic RNG. Stable typed-ID ordering resolves ties and scarce allocations.

Each simulated day follows one canonical sequence:

1. Reset market flows.
2. Apply regional route supply, active laws, and crisis effects.
3. Validate and commit business procurement.
4. Decide and apply production.
5. Decide and apply business sales.
6. Resolve household consumption and food satisfaction.
7. Resolve maintenance, deterioration, and spoilage.
8. Recalculate prices with causal explanations.
9. Update business lifecycle state.
10. Process weekly contracts, loans, rents, employment, public works, relationships, and law effects.
11. Process monthly districts, elections, AI, information, courts, routes, and crises.
12. Process annual campaign phases, succession, education, marriages, and family governance.
13. Append durable audit records.
14. Validate runtime invariants.

The same seed, state, inputs, and commands produce the same result.

## Persistence

Campaigns are stored as human-readable JSON. Schema version 6 preserves every generated record, ID, relationship, index, RNG value, objective, report, notification, and strategic obligation required for deterministic continuation. Version 4 saves migrate deterministically by retaining at most one office per character, and version 5 saves restore tenant assignments when enterprise ownership differs from separately owned premises. Saves are serialized to a same-directory temporary file, synchronized, and atomically persisted over the destination so an interrupted write does not truncate the previous campaign.

Explicit migrations are provided for schema versions 0 through 5. Version 1 Rivergate saves are deterministically hydrated with strategic records, version 2 saves consolidate institution runtime and remove redundant staffing fields, version 3 saves remove the unused parallel business-debt aggregate in favor of explicit loan records, version 4 saves resolve duplicate simultaneous officeholders in stable institution order, and version 5 saves synchronize occupied-property tenancy with business ownership.

## CLI

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

Advance a campaign:

```bash
cargo run --locked -- simulate saves/valeri.json --days 360
```

Inspect compact or complete projections:

```bash
cargo run --locked -- summary saves/valeri.json
cargo run --locked -- summary saves/valeri.json --json
cargo run --locked -- inspect saves/valeri.json
```

Generate a self-contained HTML dashboard:

```bash
cargo run --locked -- dashboard saves/valeri.json --output saves/valeri.html
```

Validate a save:

```bash
cargo run --locked -- validate saves/valeri.json
```

Apply a canonical player command. Commands use Serde's externally tagged JSON representation:

```bash
cargo run --locked -- execute saves/valeri.json \
  --command '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}'
```

Other command variants support direct dynasty investment in owned businesses, distressed-business
acquisition and recapitalization, business policy, cash transfer, contracts, loans, property
acquisition, public works, court cases, family governance, office nomination, crisis response,
labor disputes, and notification acknowledgement.
Law and public-work sponsorship require the player dynasty to hold political office.
Run `cargo run --locked -- --help` for command syntax. The complete command schema is represented by
`PlayerCommand` in `src/systems/commands.rs`.

Available starting backgrounds are `baker`, `cloth-trader`, and `blacksmith`.

Run deterministic player agents across the full command surface:

```bash
cargo run --release --locked -- playtest
```

Focused and structured reports are also supported:

```bash
cargo run --release --locked -- playtest \
  --days 360 \
  --persona entrepreneur \
  --background baker

cargo run --release --locked -- playtest \
  --seeds 10 \
  --json \
  --output gameplay-report.json
```

The harness validates state-derived candidates through the canonical command API, records real activation opportunities for reactive legal, crisis, and labor routes, advances both an action branch and a no-action counterfactual branch, and reports immediate, delayed, and ambient system changes separately. Relationship changes, earned intelligence, and notification feedback are measured as distinct domains. Reports distinguish securing inputs from selling outputs and borrowing from extending credit, measure both concrete choice depth and cross-direction breadth, and record when each campaign earns commercial standing, campaigns for office, gains office, reshapes the city, encounters player labor conflict, and reaches succession. This exposes whether the intended dynasty arc is present, incomplete, misordered, overly synchronized, or strategically repetitive. Every report states the questions that still require human playtesting. See `GAMEPLAY_HARNESS.md` for personas, scores, causal attribution, findings, traces, limitations, and performance guidance.

CI runs can use `--minimum-overall <score>` or `--fail-on-critical`; the report is still written before the command returns a failing status.

## Library API

The crate exposes:

- `build_rivergate_registry`
- `build_new_game`
- `advance_days`
- `apply_player_command`
- `quote_business_acquisition`
- `build_campaign_projection`
- `render_campaign_html`
- `run_gameplay_harness` and `render_gameplay_report`
- `save_state` and `load_state`
- `validate_invariants`

`build_new_game` returns a dedicated `NewGameError` for invalid user-authored names. Adapters receive immutable projections and submit explicit commands. They do not mutate records or own business rules.

## Verification

```bash
cargo fmt --all -- --check
cargo check --all-targets --locked
bash scripts/test.sh fast
cargo test --quiet --locked --doc
bash scripts/test.sh soak
cargo clippy --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
bash scripts/verify_cli.sh
```

`bash scripts/test.sh fast` runs only the non-ignored library tests, avoiding binary and documentation builds during ordinary edit-test cycles. An optional substring filter supports focused runs such as `bash scripts/test.sh fast loans`. `bash scripts/test.sh list laws` lists matching tests, and `bash scripts/test.sh exact <fully-qualified-name>` runs one test even when it belongs to the ignored soak tier. Long deterministic simulations are explicitly ignored by ordinary test runs and are collected under `bash scripts/test.sh soak`. `bash scripts/test.sh all` runs shell syntax checks, library tests, documentation tests, soak tests, and the CLI smoke suite in sequence.

Tests share one immutable Rivergate registry but build a fresh campaign state for every case. Large suites are separated from production modules and grouped by stable domains such as contracts, loans, laws, crises, migrations, validation, and gameplay. The suite includes deterministic replay, transaction rollback, stale-token revalidation, command rollback, bounded input validation, registry validation, schema migration, atomic save replacement, exact save/load equality, projection rendering, deterministic player-agent reports, a 3,000-day invariant soak, and a 7,200-day strategic soak spanning multiple generations. The CLI smoke script validates campaign creation, simulation, structured summaries, complete projections, commands, dashboard generation, save validation, focused gameplay-harness output, and rejected input. See `TESTING.md` for test tiers, naming, layout, and assertion guidance.

## Repository structure

```text
src/
  core/          Persistent records and AppState ownership
  registry/      Immutable Rivergate definitions
  systems/       Bootstrap, commands, simulation, strategic systems, transactions, invariants
  gameplay.rs    Player agents, counterfactual analysis, scores, traces, and findings
  projection.rs  Read-only campaign projections and HTML rendering
  persistence.rs Versioned JSON adapter and migrations
  *_tests.rs    Larger domain-organized unit-test suites
  test_support.rs Shared deterministic fixtures and diagnostic assertions
  main.rs        CLI adapter
```

The design document deliberately excludes tactical combat, manual movement of every character, equal-detail simulation of multiple cities, and repetitive crafting or dialogue minigames. The engine preserves those boundaries.