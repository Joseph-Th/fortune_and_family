# Project Status

This document describes the current implementation. Product targets are in `DESIGN.md`; implementation structure is in `ARCHITECTURE.md`.

## Current milestone

The repository implements the minimum coherent Rivergate game as a deterministic headless engine with:

- A Rust library API
- A command-line client
- Versioned JSON persistence
- Read-only JSON projections
- A self-contained HTML dashboard
- A deterministic player-agent gameplay harness

The engine supports multi-generation campaigns and preserves all consequential state required for deterministic continuation.

## Platform and contracts

| Item | Current value |
|---|---|
| Crate version | `0.2.0` |
| Rust edition | 2024 |
| Minimum Rust version | 1.97 |
| Save schema | 9 |
| Supported save migrations | Versions 0 through 8 |
| Gameplay report schema | 17 |
| Runtime services | None |
| Core randomness | Serializable state-owned deterministic RNG |
| Economic representation | Fixed-point `Money` and `Quantity` |

## Implemented domains

### World and population

- One detailed Rivergate city with six districts and abstract regional trade links.
- Eight major dynasties and grouped ordinary households.
- Typed persistent IDs and generated ID allocation.
- Characters, family links, heads, heirs, wards, councils, governance, focused player-funded education, marriage, health, loyalty, succession, and multi-generation continuity.

### Economy and business

- Ten goods and connected food, drink, textile, timber, fuel, metal, and tool chains.
- Businesses with ownership, management, capacity, policy, cash, inventory, condition, quality, distress, insolvency, closure, recovery, acquisition, and recapitalization.
- Scarce procurement, production, sales, household consumption, spoilage, maintenance, and causal price formation.
- Manager capabilities, administrative capacity, office-derived administrative burden, regional supply, and seasonal pressure.

### Contracts, finance, property, and labor

- Supply contracts with scheduled delivery, payment, penalties, fulfillment, breach, and termination.
- Loans with interest, repayment, delinquency, default, collateral, seizure, and repayment.
- Property ownership, tenancy, occupancy, rent, value, purchase, and collateral relationships.
- Employment agreements with worker capacity, wages, loyalty, conditions, disputes, suspension, recovery, and player responses.

### Institutions and civic systems

- Eleven guild, merchant, council, court, watch, treasury, charity, and market institutions.
- Membership, officeholders, budgets, legitimacy, powers, terms, institution-specific electoral competence, and deterministic elections.
- Monthly office duties funded from the officeholder dynasty, administrative strain from public service, standing penalties for shortfalls, and forced forfeiture after repeated failure.
- Laws affecting prices, imports, interest, tolls, fire safety, rents, and guild conditions.
- District employment, sanitation, safety, rent pressure, food satisfaction, unrest, and political support.
- Public works with budgets, spending, progress, completion, and persistent district effects.
- Legal cases with evidence, hearings, judgments, and damages.

### Relationships, information, AI, and crises

- Multidimensional dynasty relationships with trust, fear, respect, obligation, resentment, memories, and interaction dates.
- Quality and reliability reputation derived from economic behavior.
- Information reports with source, confidence, summary, creation, and expiry.
- Deterministic AI objectives for property, supply, office, debt, legitimacy, cash, and rival pressure.
- Regional routes with capacity, tolls, risk, disruption, and recovery.
- Grain, banking, fire, epidemic, guild, external-authority, and trade crises with detection, escalation, effects, response, and resolution.

### Adapters and observability

- CLI commands for `new`, `simulate`, `summary`, `inspect`, `dashboard`, `execute`, `validate`, and `playtest`.
- Adapter-facing read-only campaign projections for core and strategic records.
- Self-contained HTML rendering with escaped visible content and script-safe embedded JSON.
- Durable outbox notifications, chronicle entries, and audit records.
- Deterministic gameplay analysis with state-derived candidates, counterfactual branches, family-capacity metrics, scores, findings, and bounded traces.

## Public integration surface

The supported library facade is exported from `src/lib.rs`:

- `build_rivergate_registry`
- `build_new_game`
- `advance_days`
- `apply_player_command`
- `quote_business_acquisition`
- `build_campaign_projection`
- `render_campaign_html`
- `save_state` and `load_state`
- `run_gameplay_harness` and `render_gameplay_report`
- `validate_invariants`

The authoritative player command schema is `PlayerCommand` in `src/systems/commands.rs`.

## Persistence guarantees

Save and load support:

- Exact state round trips for the current schema.
- Deterministic migrations from schema versions 0 through 8.
- Release-mode validation of references, indexes, ownership, lifecycle, numeric ranges, accounting, histories, and ID allocation.
- Preservation of RNG state and all generated records required for deterministic continuation.
- Same-directory temporary writes, synchronization, and atomic replacement.

A serialized contract change requires a schema increment and a migration from the previous version.

## Runtime guarantees

The current architecture enforces:

- One canonical mutation path per operation class.
- Validation before mutation and unchanged state on failure.
- Revalidated consumed tokens for deferred cross-record commits.
- Stable result ordering and typed-ID tie-breaking.
- Fixed-point economic arithmetic with wide ratio intermediates.
- Synchronized record indexes and ownership relationships.
- Debug invariant checks after each simulated day.
- Release-mode state validation at persistence boundaries.
- Explicit daily, weekly, monthly, and annual execution order.

## Deliberate current limits

The following areas are outside the implemented contract or remain reserved for later expansion:

- `PublicDebtAuthorization` has no civic debt ledger and is rejected as an active law.
- External powers are represented through routes, demands, and crises rather than a full diplomatic simulation.
- Religion and charity are institutionally represented but do not yet implement the full doctrinal and faction scope described in `DESIGN.md`.
- The repository has no interactive graphical client. Current clients are the CLI, JSON projection, and HTML dashboard.
- Rivergate is the only detailed scenario. Additional settings require new registry content and bootstrap integration.
- Tactical combat, manual character movement, equal-detail multi-city simulation, routine crafting, and repetitive dialogue are not planned for the core engine.

## Verification status

Verified on August 4, 2026:

- 286 non-ignored library tests pass.
- 2 deterministic long-horizon soak tests pass.
- Documentation tests pass.
- Release-mode library tests pass.
- CLI creation, simulation, command execution, summary, inspection, dashboard, validation, playtest, and invalid-input smoke checks pass.
- Strict Clippy and rustdoc warning gates pass.
- The locked dependency graph contains 42 packages, no duplicate versions, and no RustSec advisories.

The authoritative release gate is documented in `TESTING.md`.

## Forward expansion areas

Expansion should add new institutional interactions rather than parallel systems. Suitable areas include:

- Additional Rivergate content and authored strategic variation
- New constitutions and office structures
- Deeper religion, charity, and legitimacy conflicts
- Municipal debt with a single explicit ledger
- Additional regional trade and external-power behavior
- New read-only or interactive clients built on the existing command and projection APIs
- New scenarios using the same Registry / AppState / System architecture
