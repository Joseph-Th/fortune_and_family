# Project Status

This document defines the current implementation surface, schemas, runtime guarantees, and deliberate limits. Product intent belongs in `DESIGN.md`; implementation ownership belongs in `ARCHITECTURE.md`.

## Platform contracts

| Item | Current value |
|---|---|
| Crate version | `0.2.0` |
| Rust edition | 2024 |
| Minimum Rust version | 1.97 |
| Save schema | 21 |
| Supported save schemas | Current schema only |
| Gameplay report schema | 52 |
| Art review report schema | 1 |
| Runtime services | None |
| Core randomness | Serializable state-owned deterministic RNG |
| Economic representation | Fixed-point `Money` and `Quantity` |

## Product surface

The repository provides:

- Rust library API
- Command-line client
- Versioned JSON persistence
- Read-only JSON projections
- Self-contained HTML campaign dashboard
- Deterministic gameplay-analysis harness
- Deterministic procedural sprite renderer and review harness

Civic Dynasty campaigns support deterministic continuation across multiple generations.

## Implemented domains

| Domain | Current capability |
|---|---|
| World | One detailed city with six districts, regional routes, seasonal pressure, and external disruption. |
| Population | Dynasties, notable characters, grouped households, health, loyalty, family links, wards, education, councils, heirs, and succession. |
| Businesses | Ownership, management, policy, cash, inventory, production, quality, condition, distress, insolvency, closure, recovery, acquisition, recapitalization, and protected owner distributions. |
| Markets | Scarce procurement, production, business and household demand, industrial tool demand, spoilage, maintenance, price formation, controls, and regional supply. |
| Contracts | Scheduled supply, payment, penalties, fulfillment, attribution, breach, and termination. |
| Private finance | Loans, interest, repayment, delinquency, default, collateral, seizure, restructuring, and repayment records. |
| Municipal finance | Authorizing laws, dynasty creditors, treasury proceeds, debt service, delinquency, default, and civic consequences. |
| Property | Ownership, value, tenancy, occupancy, rent, purchase, collateral, liquidation, lien settlement, and distressed civic guarantees. |
| Labor | Employment agreements, wages, worker capacity, conditions, loyalty, disputes, suspension, recovery, and player responses. |
| Institutions | Eleven guild, merchant, council, court, watch, treasury, charity, and market institutions with membership, budgets, legitimacy, member coalitions, powers, terms, endowments, and deterministic selection. |
| Political office | Commercial and capability gates, patronage, nomination, office powers, directives, recurring duties, administrative load, coalition response, withdrawal, forfeiture, and re-election limits. |
| Civic systems | Laws, differentiated public works, district conditions, grounded legal cases and settlements, crisis response, municipal debt, and private funding of sponsored public works. |
| Relationships | Trust, fear, respect, obligation, resentment, memories, and interaction dates. |
| Information | Source, confidence, subject, summary, creation, expiry, passive reports, and paid market/district/counterparty intelligence. |
| AI | Deterministic objectives for property, supply, office, debt, legitimacy, liquidity, and rival pressure; monthly household upkeep for family and portfolio; recapitalization only for lifetime-profitable businesses; grounded legal filing; and institution selection. |
| Crises | Grain, banking, fire, epidemic, guild, external-authority, and trade crises with detection, escalation, response, recovery, effects, and resolution. |
| Observability | State summary, campaign projection, HTML dashboard, outbox, chronicle, audit history, validation, campaign progression, gameplay reports, and art review reports. |

## Player command surface

`PlayerCommand` in `src/systems/commands.rs` is authoritative. Command families include:

- Business capital, acquisition, investment, and operating policy
- Supply contracts and private loans
- Property purchase and liquidation
- Laws, public works, legal filing, and legal settlement
- House governance, family council, heir designation, wards, and education
- Institutional patronage, endowment, nomination, office directives, and withdrawal
- Crisis and labor responses
- Information commissioning and leverage
- Notification acknowledgement

All callers use the same command validation and mutation paths.

## Public library surface

`src/lib.rs` defines the supported integration facade. It exports campaign construction and advancement, player commands, strategic quotes, persistence, projections, HTML rendering, gameplay analysis, art review, and invariant validation.

## Persistence guarantees

- Exact current-schema round trips
- Explicit rejection of older, future, and missing save schema versions
- Release-mode validation of references, indexes, ownership, lifecycle, numeric ranges, accounting, histories, schedules, and ID allocation
- Preservation of RNG state and generated records required for deterministic continuation
- Same-directory synchronized temporary writes followed by atomic replacement

A serialized contract change requires a schema increment. Existing saves from earlier schemas are intentionally unsupported.

## Runtime guarantees

- One canonical mutation path per operation class
- Validation before mutation
- Unchanged state on failure
- Revalidation for deferred commits
- Stable ordering and typed-ID tie-breaking
- Fixed-point economic arithmetic with wide ratio intermediates
- Synchronized records, indexes, ownership, occupancy, collateral, and lifecycle state
- Campaign phases derived from durable commercial, institutional, civic, and succession milestones
- Explicit daily, weekly, monthly, and annual execution order
- Runtime invariant checks during simulation
- Release validation at persistence boundaries

## Deliberate limits

- Rivergate is the only detailed scenario.
- External powers act through routes, demands, privileges, and crises rather than a full diplomacy simulation.
- Religion and charity are institutionally represented without deep doctrine or faction simulation.
- There is no interactive graphical campaign client; current clients are the CLI, JSON projection, and HTML dashboard.
- Tactical combat, manual character movement, equal-detail multi-city simulation, routine crafting, repetitive dialogue, and non-systemic decorative interiors are outside scope.

Use `TESTING.md` for the authoritative validation workflow.
