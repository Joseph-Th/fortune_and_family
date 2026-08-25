# Project Status

This document defines the current implementation surface, schemas, runtime guarantees, and deliberate limits. Product intent belongs in `DESIGN.md`; implementation ownership belongs in `ARCHITECTURE.md`.

## Platform contracts

| Item | Current value |
|---|---|
| Crate version | `0.2.0` |
| Rust edition | 2024 |
| Minimum Rust version | 1.97 |
| Save schema | 29 |
| Supported save schemas | Current schema only |
| Maximum save file size | 256 MiB |
| Gameplay report schema | 68 |
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
| World | One detailed city with six districts, regional routes, seasonal pressure, and external disruption. Route health scales household regional earning power, so disrupted trade tightens household budgets directly. |
| Population | Dynasties, notable characters, grouped households, health, loyalty, family links, wards, education, councils, heirs, and succession. A collapsed head with no designated heir designates the most capable adult member as emergency heir so succession executes instead of leaving a headless house. An annual pass guarantees no business keeps an inactive manager, and a member whose health stays collapsed for three years passes away instead of lingering as inert population. |
| Businesses | Ownership, management, policy, wages, cash, inventory, production, quality, condition, distress, insolvency, closure, recovery, acquisition, recapitalization, and protected owner distributions. Recovery from distress requires a deeper cash cushion than distress onset, so borderline firms cannot flap between states. Operating and maintenance costs flow into the market clearing pool, so every business debit has a credited counterparty. |
| Markets | Scarce procurement, production, business and household demand, industrial tool demand, tool-constrained production and maintenance, spoilage, price formation, controls, and regional supply. Production-cost floors override the speculative ceiling when producer break-even demands it. Households prefer finished bread, falling back to cheaper staples only under scarcity or poverty; they economize on cloth priced above reference. Unowned property purchases fund the market clearing pool. |
| Contracts | Scheduled supply, payment, penalties, fulfillment, breach attribution, and termination. Breach attribution records the defendant from the first attributable miss, and partially paid penalties accumulate as recoverable breach debt capped at the contractual penalty. An inactive party's termination collects whatever penalty cash it holds first. A distressed seller stops protecting contracted stock: relying on a struggling supplier carries shortage risk. |
| Private finance | Loans, interest accrued as one 7-day share of the 360-day year's annual charge, repayment, delinquency, default, collateral, seizure, restructuring, and repayment records. An installment that cannot cover the week's interest counts as missed, so unsustainable terms reach default instead of compounding silently. A won judgment discharges only what it actually recovers; a settlement extinguishes the claim in full only when its payment covers the obligation. |
| Municipal finance | Authorizing laws, dynasty creditors, treasury proceeds, debt service, delinquency, default, and civic consequences. An installment that cannot cover the week's interest counts as missed, mirroring private loans, so unsustainable municipal terms reach default instead of compounding silently behind "successful" payments. |
| Property | Ownership, value drift with district conditions, monthly condition repair, tenancy, occupancy, rent scaled by the district rent index for vacancy income and sitting tenants alike, purchase, collateral, liquidation, lien settlement, and distressed civic guarantees. Closed or insolvent occupants are evicted at the weekly settlement; a recovered firm re-occupying rented premises restores its tenancy. Each district also seeds an affordable vacant workshop: an attainable first rung. |
| Labor | Employment agreements, player-set wage posture, wage fairness relative to the market reference wage, worker capacity, conditions, loyalty, disputes, suspension, recovery, and player responses. Wages below fair pay erode loyalty and can provoke disputes; generous wages build a loyal buffer, and stingy wages stall dispute recovery. |
| Institutions | Eleven guild, merchant, council, court, watch, treasury, charity, and market institutions with membership, budgets, legitimacy, member coalitions, powers, terms, endowments, and deterministic selection. Every trade answers to one chartered guild: guild-member managers sustain a higher quality target, institutional legitimacy scales office victory rewards, and entry restrictions reserve market access for members while surcharging outsiders. |
| Political office | Commercial and capability gates, patronage, nomination, office powers, directives, recurring duties funded into institutional budgets, monthly fees of office paid back from institutional budgets, administrative load, coalition response, withdrawal, forfeiture, and re-election limits. |
| Civic systems | Laws, differentiated public works, district conditions, grounded legal cases and settlements with filing fees funding the Civic Court, crisis response, municipal debt, and private funding of any unfinished public work — a dynasty's own project or another house's stalled one, with external contributions earning visible legitimacy and sponsor gratitude. |
| Relationships | Trust, fear, respect, obligation, resentment, memories, and interaction dates. |
| Information | Source, confidence, subject, summary, creation, expiry, passive reports, and paid market/district/counterparty intelligence. |
| AI | Deterministic objectives for property, supply, office, debt, legitimacy, liquidity, and rival pressure; monthly household upkeep with penalties for shortfalls; private-credit participation funding sound firms first, with punitive speculative credit for distressed or losing firms once a house carries a live loan, so repayment failure, seizure, and grounded legal claims arise organically; accumulation skims never strip a firm below its recapitalization target. |
| Crises | Grain, banking, fire, epidemic, guild, external-authority, and trade crises with detection, escalation, response, resolution, and recovery. Trade disruptions track their route condition: organized responses heal routes, a watch directive counts as a response, and the disruption resolves once every route heals below the detection threshold. A resolved banking panic raises the follow-up default bar for three years; resolved crises then leave state. Phantom demand moves prices only. |
| Observability | State summary, campaign projection, HTML dashboard, outbox, chronicle, audit history, validation, campaign progression, gameplay reports with causal feedback traces, and art review reports. |

## Player command surface

`PlayerCommand` in `src/systems/commands.rs` is authoritative. Command families include:

- Business capital, acquisition, investment, wage posture, and operating policy
- Supply contracts and private loans
- Property purchase and liquidation
- Laws, public works, legal filing, and legal settlement
- House governance, family council, heir designation, wards, and education. Charter changes, heir designation, and ward adoption cost family unity that a divided council cannot pay
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
- Save loads require paths that resolve to regular files and reject inputs larger than 256 MiB before parsing
- Release-mode validation of references, indexes, ownership, lifecycle, numeric ranges, accounting, histories, schedules, and ID allocation. Active weekly obligations must be settleable within the coming fortnight: schedules signed mid-week keep their nominal one-week due date and stay anchored to their signing rather than snapping onto the global weekly boundary.
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
