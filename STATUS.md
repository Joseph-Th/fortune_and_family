# Project Status

This document defines the current implementation surface, schemas, runtime guarantees, and deliberate limits. Product intent belongs in `DESIGN.md`; implementation ownership belongs in `ARCHITECTURE.md`. Profiles: **Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation** — see [AGENTS.md](AGENTS.md) and [ARCHITECTURE.md](ARCHITECTURE.md) for routing.

## Platform contracts

| Item | Current value |
|---|---|
| Crate version | `0.2.0` |
| Rust edition | 2024 |
| Minimum Rust version | 1.97 |
| Save schema | 31 |
| Supported save schemas | Current schema only |
| Maximum save file size | 256 MiB |
| Gameplay report schema | 71 |
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
| World | One detailed city with six districts, regional routes, seasonal pressure, and external disruption. Route health scales household earning power and import-trade output via capacity-weighted disruption, so shocks tighten budgets and supply coherently. Living costs scale with district and class, so cash-rich groups face pressure and shortfalls erode satisfaction. |
| Population | Dynasties, notable characters, grouped households, health, loyalty, family links, wards, education, councils, heirs, and succession. A collapsed head with no heir designates the most capable adult as emergency heir; a sole collapsed head generates a successor so succession executes. An annual pass guarantees no business keeps an inactive manager; three collapsed years end a member's life. |
| Succession pacing | Founder succession pressure targets a median first transition in the middle of the third campaign year — late enough that an ascending founder has offices and memberships worth testing, early enough that dynastic continuity is ordinary play. |
| Businesses | Ownership, management, policy, wages, cash, inventory, production, quality, condition, distress, insolvency, closure, recovery, acquisition, recapitalization, and protected owner distributions. Recovery requires a deeper cash cushion than onset; quality mean-reverts to its target; operating and maintenance costs flow into the market clearing pool, so every business debit has a credited counterparty. |
| Business transfer | Failing trades change hands at a discount to book value; a going concern quotes at a 140% controlling premium. Portfolio growth converts wealth into capacity and administrative load at a real cost while the seller ends the exchange richer. |
| Markets | Scarce procurement, production, business and household demand, industrial tool demand, tool-constrained production and maintenance, spoilage, price formation, controls, and regional supply. Production-cost floors override the speculative ceiling at producer break-even. Households prefer finished bread, fall back to cheaper staples under scarcity or poverty, and economize on cloth priced above reference. Unowned property purchases fund the market clearing pool. |
| Contracts | Scheduled supply, payment, penalties, fulfillment, breach attribution, and termination. Breach attribution records the defendant from the first attributable miss; partially paid penalties accumulate as recoverable breach debt capped at the contractual penalty. A business that loses active standing terminates its active contracts the same day. Termination collects whatever penalty cash an inactive party holds first; a distressed seller stops protecting contracted stock. |
| Private finance | Loans, interest as one 7-day share of the 360-day annual charge, repayment, delinquency, default, collateral, seizure, restructuring, and write-off. Unproductive installments count as missed so unsustainable terms reach default. Unresolved defaults block fresh unrelated credit and can be worked out with the existing creditor. After final execution leaves a borrower with no collectible assets, the deficiency becomes an explicit lender write-off with severe standing damage. |
| Municipal finance | Authorizing laws, dynasty creditors, treasury proceeds, debt service, delinquency, default, and civic consequences. Missed-installment semantics mirror private loans, so unsustainable municipal terms reach default instead of compounding behind nominal payments. |
| Property | Ownership, value drift with district conditions, monthly 180 bp condition repair, tenancy, occupancy, rent scaled by district rent index (vacancy and sitting tenants) and discounted for fire-damaged premises, purchase, collateral, liquidation, lien settlement, and distressed civic guarantees. Closed or insolvent occupants are evicted weekly; recovered firms re-occupy rented premises. Each district seeds an affordable vacant workshop. |
| Labor | Employment agreements, player-set wage posture, wage fairness relative to the market reference wage, worker capacity, conditions, loyalty, disputes, suspension, recovery, and player responses. Sub-fair wages erode loyalty toward dispute; generous wages build a loyal buffer; stingy pay stalls dispute recovery. Employers retain a week of operating cover during settlement; closure returns workers to the household pool. |
| Institutions | Eleven guild, merchant, council, court, watch, treasury, charity, and market institutions with membership, budgets, legitimacy, member coalitions, powers, terms, endowments, and deterministic selection. Every trade answers to one chartered guild: guild-member managers sustain a higher quality target, institutional legitimacy scales office victory rewards, and entry restrictions reserve market access for members while surcharging outsiders. |
| Political office | Commercial and capability gates, patronage, nomination, office powers, directives, recurring duties funded into institutional budgets, monthly fees of office repaid from institutional budgets, administrative load, coalition response, withdrawal, forfeiture, and re-election limits. |
| Civic systems | Laws, differentiated public works, district conditions, grounded legal cases and settlements with filing fees funding the Civic Court, crisis response, municipal debt, and private funding of any unfinished public work — a dynasty's own project or another house's stalled one — with external contributions earning visible legitimacy and sponsor gratitude. |
| Relationships | Trust, fear, respect, obligation, resentment, memories, and interaction dates. |
| Information | Source, confidence, subject, summary, creation, expiry, passive reports, and paid market/district/counterparty intelligence. |
| AI houses | Deterministic objectives for property, supply, office, debt, legitimacy, liquidity, and rival pressure; monthly upkeep with great-house wealth stewardship penalizing hoards; private-credit participation that works out aged defaults with their existing creditor before funding sound firms, with speculative credit reserved for liquidity-strained houses or structurally losing firms. Rival supply contracts commit near real weekly input need at penalties scaled to scheduled value. |
| Crises | Seven kinds with detection, escalation, response, resolution, and recovery: grain, banking, fire, epidemic, guild revolt, noble demand, and trade disruption. A response counts as containment for a bounded window; afterwards persistent crises may be answered again. Trade disruption resolves when every route heals below detection; paid responses never inflate severity. Route spikes outweigh calm healing; the levy is annual; a resolved panic raises the default bar for three years. |
| Crisis standing | Grain shortage declares while staple stores thin against their target stock under collapsed regional access — before shelves empty — leaving response routes something to protect. Crisis service earns standing with diminishing returns inside one year (full, half, quarter, one-eighth); material relief is never reduced. |
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
- Release-mode validation of references, indexes, ownership, lifecycle, numeric ranges, accounting, histories, schedules, and ID allocation. Active weekly obligations must be settleable within the coming fortnight: schedules signed mid-week keep their nominal one-week due date anchored to their signing rather than snapping onto the global weekly boundary.
- Preservation of RNG state and generated records required for deterministic continuation
- Same-directory synchronized temporary writes followed by atomic replacement (`SaveOutcome` distinguishes committed vs `CommittedWithDegradedDurability`)
- Boundary input strictness: `PlayerCommand`/`LoanTerms`/`SupplyContractTerms`/`NewGameConfig` use `#[serde(deny_unknown_fields)]` so unsupported fields fail closed; save JSON duplicate members are rejected before parsing and top-level schema probe rejects non-current `schema_version`

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
