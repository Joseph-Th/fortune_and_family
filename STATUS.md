# STATUS.md

## Current milestone

Complete minimum coherent Rivergate game engine.

The repository now implements the full architectural foundation and a broad, integrated version of every major system required by the minimum coherent game in `DESIGN.md`. It is a deterministic headless game with a CLI, command API, projection API, and HTML dashboard rather than a narrow vertical slice.

## Architecture

- Rust 2024 library and CLI binary.
- Registry / AppState / Record / System ownership model.
- Code-owned immutable scenario definitions.
- Typed IDs for registry and runtime references.
- Fixed-point `Money` and `Quantity` values.
- Serializable state-owned deterministic RNG.
- Private synchronized character, business, and household stores.
- Canonical validate / resolve / commit mutation paths.
- Revalidated commit tokens for cross-record contracts and loans.
- Dedicated error enums for persistence, simulation, strategic operations, and commands.
- Fallible, bounded, whitespace-normalizing new-campaign input handling.
- Explicit daily, weekly, monthly, and annual execution order.
- Debug invariant validation after every simulated day and every CLI command.
- No business rules in CLI, persistence, projection, or rendering adapters.

## Rivergate content

- Six districts.
- Ten goods.
- Ten production and regional-trade recipes.
- Eleven institutions.
- Eight major dynasties.
- Thirty-six grouped household records.
- Grain, brewing, textile, timber, fuel, iron, and tool chains.
- Baker, cloth trader, and blacksmith starts.
- Four regional trade routes.
- Persistent opening laws, public works, information, contracts, loans, employment, property, relationships, objectives, and a court case.

## Integrated systems

### Economy and business

- Scarce market procurement.
- Deterministic production and output sale.
- Household demand and food satisfaction.
- Fixed-point price formation with causal records.
- Seasonal pressure, spoilage, and regional supply.
- Business policies, maintenance, condition, quality, distress, insolvency, and recovery.
- Administrative-capacity penalties.
- Canonical inter-business cash transfers.

### Contracts, finance, and property

- Explicit supply contracts.
- Weekly delivery, payment, breach, penalty, fulfillment, and termination.
- Explicit loans with interest, weekly payment, delinquency, default, repayment, and restructuring state.
- Property collateral and deterministic seizure.
- Residences, workshops, warehouses, occupants, owners, tenants, values, conditions, rents, and purchases.
- Weekly rent settlement.

### Labor and households

- Employment agreements connecting household labor pools to businesses.
- Worker counts, wages, loyalty, conditions, disputes, and replacement.
- Weekly wage settlement.
- Player responses through investment, negotiation, or replacement.
- District-level employment and material-condition effects.

### Dynasty and family

- Heads, heirs, roles, capabilities, health, loyalty, and lifecycle.
- Family links and parent-child records.
- Family councils, unity, charters, and governance models.
- Education progression.
- Dynastic marriages and relationship consequences.
- Annual mortality and succession.
- Multi-generation continuation.

### Institutions, politics, and law

- Guild, merchant, council, court, watch, treasury, charity, and market institutions.
- Membership, powers, budgets, legitimacy, terms, and officeholders.
- Deterministic elections using capability, legitimacy, and stable tie-breaking.
- Persistent laws and supersession.
- Bread ceilings, emergency imports, interest limits, tolls, fire codes, rent rules, and guild rules.
- Public-debt authorization remains a reserved law kind and is rejected before cost or mutation until a civic debt ledger exists.
- Player law sponsorship and office nominations.

### Relationships, information, and AI

- Trust, fear, respect, obligation, resentment, memories, and interaction dates.
- Information reports with source, confidence, summary, and expiry.
- Monthly causal market reports.
- Traceable AI objectives with priorities and rationale.
- AI actions for property acquisition, office pursuit, supply security, debt reduction, legitimacy, cash accumulation, and rival containment.
- Objective completion and deterministic objective replacement.

### Districts, public works, courts, and crises

- District rent, employment, sanitation, safety, unrest, support, and food conditions.
- Public works with budgets, spending, progress, completion, and permanent district effects.
- Legal cases with parties, evidence, attention, hearings, damages, and judgments.
- External route capacity, tolls, risk, disruption, and recovery.
- Grain shortage, banking panic, urban fire, guild revolt, noble demand, epidemic, and trade disruption records.
- Crisis detection, escalation, daily effects, natural recovery, and player responses.

### Adapters and presentation

- Human-readable JSON persistence with synchronized same-directory temporary writes and atomic replacement.
- Schema version 2.
- Explicit migrations from versions 0 and 1.
- Deterministic strategic hydration for version 1 Rivergate saves.
- Release-mode load validation for registry alignment, references, synchronized indexes, numeric ranges, reciprocal collateral, and next-ID allocators.
- Durable outbox notifications.
- Compact state summary.
- Complete campaign projection.
- Self-contained HTML dashboard with escaped visible content and script-safe embedded JSON.
- CLI commands for new, simulate, summary, inspect, dashboard, execute, and validate.
- JSON `PlayerCommand` API for every consequential player mutation currently exposed.

## Invariants

Runtime validation covers:

- Registry and record reference validity.
- Store index completeness and uniqueness.
- Ownership and occupancy exclusivity.
- Character, manager, head, heir, officeholder, family-member, and contract-party consistency.
- Contract production-chain compatibility.
- Loan, collateral, property, rent, employment, and public-work accounting.
- Lifecycle and basis-point bounds.
- Administrative-load derivation.
- Institution legacy/strategic synchronization.
- Family councils and relationship pairs.
- Information, AI objective, court, route, crisis, outbox, chronicle, and audit dates.
- Deterministic chronological ordering.
- Save/load serialization completeness.

## Verification status

The release gate requires all of the following to pass:

- `cargo fmt --all -- --check`
- `cargo check --all-targets --locked`
- `bash scripts/test.sh fast`
- `bash scripts/test.sh soak`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked`
- `bash scripts/verify_cli.sh`
- CLI create, simulate, summary, inspect, execute, dashboard, validate, and invalid-input smoke tests

The fast suite uses a shared immutable registry fixture and fresh per-test campaign state. Long-running deterministic checks are co-located in `src/core/state.rs` and explicitly invoked by the soak test mode. The automated suite includes failure-path rollback coverage, atomic save replacement, a 3,000-day core invariant soak, and a 7,200-day strategic soak.

## Deliberate boundaries

The implementation follows the exclusions in `DESIGN.md`:

- No tactical battlefield mode.
- No manual movement of every character.
- No several-city simulation at Rivergate's detail level.
- No crafting minigames.
- No repetitive routine dialogue trees.
- No decorative interiors without systemic effects.

Future content expansion can add more authored industries, constitutions, cultures, scenarios, religious structures, external powers, and presentation clients without replacing the current state model or canonical pipelines.