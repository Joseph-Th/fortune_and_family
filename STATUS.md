# Project Status

This document describes the current implementation contract. It does not record verification history or completed repair work.

Use `DESIGN.md` for intended product behavior and `ARCHITECTURE.md` for code ownership.

## Platform contracts

| Item | Current value |
|---|---|
| Crate version | `0.2.0` |
| Rust edition | 2024 |
| Minimum Rust version | 1.97 |
| Save schema | 18 |
| Supported save migrations | Versions 0 through 17 |
| Gameplay report schema | 49 |
| Art review report schema | 1 |
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
- A deterministic procedural sprite renderer and visual review harness

The Rivergate campaign supports multi-generation deterministic continuation.

## Implemented systems

| Domain | Current capability |
|---|---|
| World | One detailed city with six districts, regional routes, seasonal pressure, and external disruption. |
| Population | Major dynasties, notable characters, grouped households, family links, health, loyalty, wards, education, councils, heirs, and succession. |
| Businesses | Ownership, management, policy, inventory, cash, production, quality, condition, distress, insolvency, closure, recovery, acquisition, recapitalization, and owner distributions protected by the same operating floor as automatic dividends. |
| Markets | Scarce procurement, production, sales, household and industrial consumption, spoilage, maintenance, price formation, controls, and regional supply. Replacement tools are purchased from the market out of existing production and maintenance overhead, giving toolmaking a citywide productive demand loop without double-charging operating businesses. |
| Contracts | Scheduled supply, payment, penalties, fulfillment, durable dynasty attribution, breach, and termination. |
| Private finance | Loans, interest, repayment, delinquency, default, collateral, seizure, restructuring, and repayment history. |
| Municipal finance | Authorizing laws, dynasty creditors, treasury proceeds, scheduled service, delinquency, default, and civic consequences. |
| Property | Ownership, value, tenancy, occupancy, district-indexed rent, purchase, collateral, voluntary liquidation, lien settlement, and distressed civic guarantees. Vacant warehouse yields are calibrated as a competing investment rather than an automatic high-return wealth step. |
| Labor | Employment agreements, wages, worker capacity, conditions, loyalty, disputes, suspension, recovery, and player responses. |
| Institutions | Eleven guild, merchant, council, court, watch, treasury, charity, and market institutions with earned patronage-based membership, annual dynasty endowments after support matures, budgets, legitimacy, member-house coalition effects, powers, terms, and deterministic selection. |
| Political office | Commercial eligibility, capability-sensitive patronage preparation, paid institutional patronage, a 180-day support-establishment period, nomination with a 120-day campaign resolution, office-specific competence, a 120-day power-establishment period within a 360-day term, recurring duties with escalating multi-office portfolio overhead, coalition backlash from concentrated office control, administrative load, voluntary withdrawal with political recovery before re-entry, forfeiture, and re-election limits. |
| Civic systems | Laws, differentiated public works, district conditions, claim-sourced legal cases, crisis response, and municipal debt. A sponsoring dynasty can directly fund its own unfinished public works, allowing accumulated private wealth to rescue or accelerate civic commitments while recording the contribution as civic patronage. Municipal construction consumes tools from the shared market as part of its existing project spend, linking civic development to industrial demand. Player and rival litigation are grounded in exact distressed loans or attributable breached contracts with unpaid terminal penalties; non-player creditors can autonomously file the same grounded claims and notify the player when the dynasty is sued. A player defendant may negotiate an early settlement of a grounded unresolved claim, with the settlement discount shrinking as the plaintiff's evidence strengthens; payment closes the exact underlying obligation and the case remains auditable as settled. Winning grounded judgments likewise settle the source loan or unpaid breach balance; immediate recovery is limited by defendant cash and any uncollectible remainder is written off instead of surviving as a second collectible path. Completed infrastructure persists through district recomputation, while food access, employment, sanitation, safety, and rent pressure all feed district unrest and therefore public response. |
| Relationships | Trust, fear, respect, obligation, resentment, memories, and interaction dates. |
| Information | Source, confidence, summary, creation, expiry, passive reports, and paid player-directed market, district, and counterparty intelligence. |
| AI | Deterministic objectives for property, supply, office, debt, legitimacy, liquidity, and rival pressure; sustained containment can worsen the commercial terms the player receives from that house. |
| Crises | Grain, banking, fire, epidemic, guild, external-authority, and trade crises with detection, monthly escalation while unaddressed, one-time exploitation followed by optional containment, response-driven recovery, effects, and resolution. |
| Observability | Summary, projection, HTML dashboard, outbox, chronicle, audit history, validation, progression-aligned campaign phases, and gameplay reports with separate player borrowing/lending distress, direct player-defendant legal pressure, exact before/after values for measured command consequences, first-succession transition profiles for family unity, legitimacy, offices, memberships, and represented institutions, plus mature-liquidity and starting-trade balance diagnostics so high resilience cannot hide an economically solved dynasty or a hidden background difficulty mode. |

## Player commands

`PlayerCommand` in `src/systems/commands.rs` is authoritative. The current command surface includes:

- Business-to-business cash transfer, protected owner distributions to dynasty treasury, acquisition, investment, and operating policy
- Supply contracts and private loans
- Property purchase and sale
- Laws, public-work sponsorship and direct funding, legal filing, and negotiated settlement of grounded claims against the dynasty
- House governance, ward adoption, and family education
- Institutional patronage, mature-member endowment, office nomination, and institutional withdrawal
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
- Campaign phases advance from durable commercial, institutional, civic, and succession milestones rather than elapsed calendar age
- Time-limited reports and office directives expire on the first daily boundary after their inclusive expiry day
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
