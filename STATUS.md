# Project Status

This document defines the current implementation surface, schemas, runtime guarantees, and deliberate limits. Product intent belongs in `DESIGN.md`; implementation ownership belongs in `ARCHITECTURE.md`.

## Platform contracts

| Item | Current value |
|---|---|
| Crate version | `0.2.0` |
| Rust edition | 2024 |
| Minimum Rust version | 1.97 |
| Save schema | 30 |
| Supported save schemas | Current schema only |
| Maximum save file size | 256 MiB |
| Gameplay report schema | 70 |
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
| Population | Dynasties, notable characters, grouped households, health, loyalty, family links, wards, education, councils, heirs, and succession. A collapsed head with no heir designates the most capable adult member as emergency heir so succession executes; a head whose house has no other active member is pinned at a survivable health floor instead of lingering as an invalid record. An annual pass guarantees no business keeps an inactive manager; three collapsed years end a member's life. |
| Succession pacing | Founder succession pressure ramps so the median first transition lands in the middle of the third campaign year — late enough that an ascending founder has offices and memberships worth testing when the transition arrives, early enough that dynastic continuity is ordinary play rather than only generation-length content. |
| Businesses | Ownership, management, policy, wages, cash, inventory, production, quality, condition, distress, insolvency, closure, recovery, acquisition, recapitalization, and protected owner distributions. Recovery requires a deeper cash cushion than onset, acquisitions land on the status their recapitalized cash sustains, and quality mean-reverts to its target. Operating and maintenance costs flow into the market clearing pool, so every business debit has a credited counterparty. |
| Business transfer | Failing trades change hands at a discount to book value; a going concern is quotable at a 140% controlling premium, so portfolio growth converts wealth into capacity and administrative load at a real cost while the seller ends the exchange richer than before. |
| Markets | Scarce procurement, production, business and household demand, industrial tool demand, tool-constrained production and maintenance, spoilage, price formation, controls, and regional supply. Production-cost floors override the speculative ceiling when producer break-even demands it. Households prefer finished bread, falling back to cheaper staples only under scarcity or poverty; they economize on cloth priced above reference. Unowned property purchases fund the market clearing pool. |
| Regional coupling | Input-less import trades scale their daily output with regional route availability (with a small blockade floor), so sustained disruption reaches the staple chains downstream — grain stores thin, prices spike, and the food system shares the risk households already feel — instead of stopping at the gatehouses. |
| Contracts | Scheduled supply, payment, penalties, fulfillment, breach attribution, and termination. Breach attribution records the defendant from the first attributable miss; partially paid penalties accumulate as recoverable breach debt capped at the contractual penalty. A business that loses active standing terminates its active contracts the same day. Termination collects whatever penalty cash an inactive party holds first, and a distressed seller stops protecting contracted stock. |
| Private finance | Loans, interest accrued as one 7-day share of the 360-day year annual charge, repayment, delinquency, default, collateral, seizure, restructuring, and repayment records. An installment that cannot reduce the balance below its capitalized interest counts as missed, so unsustainable terms reach default instead of servicing forever. A judgment discharges only what it recovers; a settlement extinguishes the claim only when fully covered. |
| Municipal finance | Authorizing laws, dynasty creditors, treasury proceeds, debt service, delinquency, default, and civic consequences. An installment that cannot reduce the balance below its capitalized interest counts as missed, mirroring private loans, so unsustainable municipal terms reach default instead of compounding behind "successful" payments. |
| Property | Ownership, value drift with district conditions, monthly condition repair, tenancy, occupancy, rent scaled by the district rent index for vacancy income and sitting tenants alike, purchase, collateral, liquidation, lien settlement, and distressed civic guarantees. Closed or insolvent occupants are evicted at the weekly settlement; a recovered firm re-occupying rented premises restores its tenancy. Each district also seeds an affordable vacant workshop: an attainable first rung. |
| Labor | Employment agreements, player-set wage posture, wage fairness relative to the market reference wage, worker capacity, conditions, loyalty, disputes, suspension, recovery, and player responses. Wages below fair pay erode loyalty and can provoke disputes; generous wages build a loyal buffer; stingy wages stall dispute recovery entirely rather than rewarding it. Employers retain a week of operating cover during settlement; closure returns a firms workers to the household pool. |
| Institutions | Eleven guild, merchant, council, court, watch, treasury, charity, and market institutions with membership, budgets, legitimacy, member coalitions, powers, terms, endowments, and deterministic selection. Every trade answers to one chartered guild: guild-member managers sustain a higher quality target, institutional legitimacy scales office victory rewards, and entry restrictions reserve market access for members while surcharging outsiders. |
| Political office | Commercial and capability gates, patronage, nomination, office powers, directives, recurring duties funded into institutional budgets, monthly fees of office paid back from institutional budgets, administrative load, coalition response, withdrawal, forfeiture, and re-election limits. |
| Civic systems | Laws, differentiated public works, district conditions, grounded legal cases and settlements with filing fees funding the Civic Court, crisis response, municipal debt, and private funding of any unfinished public work — a dynasty's own project or another house's stalled one, with external contributions earning visible legitimacy and sponsor gratitude. |
| Relationships | Trust, fear, respect, obligation, resentment, memories, and interaction dates. |
| Information | Source, confidence, subject, summary, creation, expiry, passive reports, and paid market/district/counterparty intelligence. |
| AI | Deterministic objectives for property, supply, office, debt, legitimacy, liquidity, and rival pressure; monthly household upkeep with great-house wealth stewardship penalizing hoards; private-credit participation funding sound firms first, with punitive speculative credit reserved for liquidity-strained houses or structurally losing firms. Rival supply contracts commit near real weekly input need at penalties scaled to scheduled value. |
| Crises | Seven kinds with detection, escalation, response, resolution, and recovery: grain, banking, fire, epidemic, guild revolt, noble demand, and trade disruption. A response counts as containment for a bounded window; afterwards persistent crises may be answered again. Trade disruption resolves when every route heals below detection; paid responses never inflate severity. Route spikes outweigh calm healing and the levy is annual. A resolved panic raises the default bar for three years. |
| Crisis standing | Grain shortage detects a building squeeze — staple stores thinning against target stock while regional access collapses — before shelves empty, so response routes still have something to protect. Crisis service earns standing with diminishing returns inside a year (full, half, quarter, one-eighth): the city rewards fresh service rather than a house that lives from crisis to crisis. Material relief is never diminished — only the legitimacy credit. |
| Observability | State summary, campaign projection, HTML dashboard, outbox, chronicle, audit history, validation, campaign progression, gameplay reports with causal feedback traces, and art review reports. |

## Player command surface

`PlayerCommand` in `src/systems/commands/` is authoritative. Command families include:

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
