# Project Status

This document describes the current implementation contract. It does not record verification history or completed repair work.

Use `DESIGN.md` for intended product behavior and `ARCHITECTURE.md` for code ownership.

## Platform contracts

| Item | Current value |
|---|---|
| Crate version | `0.2.0` |
| Rust edition | 2024 |
| Minimum Rust version | 1.97 |
| Save schema | 15 |
| Supported save migrations | Versions 0 through 14 |
| Gameplay report schema | 42 |
| Runtime services | None |
| Core randomness | Serializable state-owned deterministic RNG |
| Economic representation | Fixed-point `Money` and `Quantity` |

## Product surface

The repository provides:

- A Rust library API
- A command-line client
- Versioned JSON persistence
- Read-only JSON projections
- A self-contained HTML dashboard
- A deterministic gameplay-analysis harness

The Rivergate campaign supports multi-generation deterministic continuation.

## Implemented systems

| Domain | Current capability |
|---|---|
| World | One detailed city with six districts, regional routes, seasonal pressure, and external disruption. |
| Population | Major dynasties, notable characters, grouped households, family links, health, loyalty, wards, education, councils, heirs, and succession. |
| Businesses | Ownership, management, policy, inventory, cash, production, quality, condition, distress, insolvency, closure, recovery, acquisition, and recapitalization. |
| Markets | Scarce procurement, production, sales, household consumption, spoilage, maintenance, price formation, controls, and regional supply. |
| Contracts | Scheduled supply, payment, penalties, fulfillment, durable dynasty attribution, breach, and termination. |
| Private finance | Loans, interest, repayment, delinquency, default, collateral, seizure, restructuring, and repayment history. |
| Municipal finance | Authorizing laws, dynasty creditors, treasury proceeds, scheduled service, delinquency, default, and civic consequences. |
| Property | Ownership, value, tenancy, occupancy, rent, purchase, collateral, voluntary liquidation, lien settlement, and distressed civic guarantees. |
| Labor | Employment agreements, wages, worker capacity, conditions, loyalty, disputes, suspension, recovery, and player responses. |
| Institutions | Eleven guild, merchant, council, court, watch, treasury, charity, and market institutions with earned patronage-based membership, budgets, legitimacy, powers, terms, and deterministic selection. |
| Political office | Commercial eligibility, paid institutional patronage, a 180-day support-establishment period, nomination with a 120-day campaign resolution, office-specific competence, a 120-day power-establishment period within a 360-day term, recurring duties with escalating multi-office portfolio overhead, coalition backlash from concentrated office control, administrative load, voluntary withdrawal with political recovery before re-entry, forfeiture, and re-election limits. |
| Civic systems | Laws, differentiated public works, district conditions, legal cases, crisis response, and municipal debt. Completed infrastructure persists through district recomputation, while food access, employment, sanitation, safety, and rent pressure all feed district unrest and therefore public response. |
| Relationships | Trust, fear, respect, obligation, resentment, memories, and interaction dates. |
| Information | Source, confidence, summary, creation, expiry, passive reports, and paid player-directed market, district, and counterparty intelligence. |
| AI | Deterministic objectives for property, supply, office, debt, legitimacy, liquidity, and rival pressure; sustained containment can worsen the commercial terms the player receives from that house. |
| Crises | Grain, banking, fire, epidemic, guild, external-authority, and trade crises with detection, monthly escalation while unaddressed, one-time exploitation followed by optional containment, response-driven recovery, effects, and resolution. |
| Observability | Summary, projection, HTML dashboard, outbox, chronicle, audit history, validation, and gameplay reports. |

## Player commands

`PlayerCommand` in `src/systems/commands.rs` is authoritative. The current command surface includes:

- Business cash transfer, acquisition, investment, and operating policy
- Supply contracts and private loans
- Property purchase and sale
- Laws, public works, and legal cases
- House governance, ward adoption, and family education
- Institutional patronage, office nomination, and institutional withdrawal
- Crisis and labor-dispute responses
- Commissioned market, district, and counterparty intelligence
- Notification acknowledgement

Commands use the same canonical validation and mutation paths from the library, CLI, tests, and gameplay harness.

## Public library surface

`src/lib.rs` exports the supported integration API:

- `build_rivergate_registry`
- `build_new_game`
- `advance_days`
- `apply_player_command`
- `quote_business_acquisition`
- `quote_property_liquidation`
- `build_state_summary`
- `build_campaign_projection`
- `render_campaign_html`
- `save_state` and `load_state`
- `run_gameplay_harness` and `render_gameplay_report`
- `validate_invariants`

## Persistence guarantees

- Exact round trips for the current schema
- Deterministic migrations from supported older schemas
- Release-mode validation of references, indexes, ownership, lifecycle, numeric ranges, accounting, histories, and ID allocation
- Preservation of RNG state and generated records required for deterministic continuation
- Synchronized same-directory temporary writes followed by atomic replacement

A serialized contract change requires a schema increment and one migration from the previous version.

## Runtime guarantees

- One canonical mutation path per operation class
- Validation before mutation
- Unchanged state on failure
- Revalidated consumed tokens for deferred commits
- Stable ordering and typed-ID tie-breaking
- Fixed-point economic arithmetic with wide ratio intermediates
- Synchronized records, indexes, ownership, occupancy, and collateral
- Explicit daily, weekly, monthly, and annual execution order
- Debug invariant checks during simulation
- Release-mode validation at persistence boundaries

## Deliberate limits

- Rivergate is the only detailed scenario.
- External powers operate through routes, demands, privileges, and crises rather than a full diplomacy simulation.
- Religion and charity are represented institutionally but do not yet implement deep doctrine or faction systems.
- The repository has no interactive graphical client. Current clients are the CLI, JSON projection, and HTML dashboard.
- Tactical combat, manual character movement, equal-detail multi-city simulation, routine crafting, repetitive dialogue, and decorative interiors without systemic effects are outside scope.

## Extension priorities

New work should deepen the shared economy, institution, family, and city model rather than add parallel progression systems. Suitable areas include:

- Additional Rivergate content and strategic variation
- New constitutions and office structures
- Deeper religion, charity, and legitimacy conflict
- Additional regional trade and external-pressure behavior
- New clients built on the command and projection APIs
- New scenarios using the existing Registry / AppState / System architecture

The authoritative validation workflow is in `TESTING.md`.
