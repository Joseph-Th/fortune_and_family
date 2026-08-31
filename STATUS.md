# Project Status

Current implementation surface, schemas, runtime guarantees, and deliberate limits. Product intent is in `DESIGN.md`; implementation ownership is in `ARCHITECTURE.md`. Profiles: **Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation** — see [AGENTS.md](AGENTS.md) and [ARCHITECTURE.md](ARCHITECTURE.md).

## Platform contracts

| Item | Current value |
|---|---|
| Crate version | `0.2.0` |
| Rust edition | 2024 |
| Minimum Rust version | 1.97 |
| Save schema | 31 |
| Supported save schemas | Current schema only |
| Maximum save file size | 256 MiB |
| Gameplay report schema | 76 |
| Art review report schema | 1 |
| Runtime services | None |
| Core randomness | Serializable state-owned deterministic RNG |
| Economic representation | Fixed-point `Money` and `Quantity` |

## Product surface

- Rust library API
- Command-line client
- Versioned JSON persistence
- Read-only JSON projections
- Self-contained HTML campaign dashboard
- Deterministic gameplay-analysis harness
- Deterministic procedural sprite renderer and review harness

Campaigns support deterministic continuation across multiple generations.

## Implemented domains

| Domain | Capability |
|---|---|
| World | One city (six districts), regional routes, seasonal pressure, external disruption. Capacity-weighted route health scales household income and import-trade output. Living costs 28/52/78 copper per member monthly (Laboring/Artisan/Merchant) times rent index. |
| Population | Dynasties, notable characters, grouped households, health, loyalty, family links, wards, education, councils, heirs, succession. Collapsed head with no heir designates the most capable adult; sole collapsed head generates a successor. Annual pass ensures no business retains an inactive manager; three collapsed years end life. |
| Succession pacing | Founder succession pressure targets median first transition in the middle of the third year: late enough that the founder has offices/memberships worth testing, early enough that continuity is ordinary play. |
| Businesses | Ownership, management, policy, wages, cash, inventory, production, quality, condition, distress, insolvency, closure, recovery, acquisition, recapitalization, protected owner distributions. Recovery requires deeper cash than onset; quality mean-reverts; operating/maintenance costs credit the market clearing pool. |
| Business transfer | Failing trades sell at discount to book value; going concerns quote at 140% controlling premium. Book value uses registry base price, not volatile market price. Portfolio growth converts wealth into capacity and administrative load. Tool-allocation priority rotates daily to avoid systematic starvation. |
| Markets | Scarce procurement, production, business/household/industrial demand, tool-constrained production and maintenance, spoilage, price formation, controls, regional supply. Production-cost floors override speculative ceilings at break-even. Households prefer bread, fall back to cheaper staples under scarcity/poverty, and reduce cloth demand when priced above reference. Unowned property purchases credit the clearing pool. |
| Contracts | Scheduled supply, payment, penalties, fulfillment, breach attribution, termination. Breach records the defendant from the first attributable miss; partially paid penalties accumulate as capped breach debt. Businesses losing active standing terminate active contracts the same day; penalty collection prefers cash of the inactive party; distressed sellers stop protecting contracted stock. |
| Private finance | Loans, interest at 1/360 share per 7 days, repayment, delinquency, default, collateral, seizure, restructuring, write-off. Unproductive installments count as missed. Unresolved defaults block fresh unrelated credit and can be worked out with the existing creditor. After final execution with no collectible assets, the deficiency is a lender write-off with severe standing damage. |
| Municipal finance | Authorizing laws, dynasty creditors, treasury proceeds, debt service, delinquency, default, civic consequences. Mirrors private-loan missed-installment semantics. |
| Property | Ownership, value drift with district conditions, monthly 180 bp condition repair, tenancy, occupancy, rent scaled by district index (discounted for fire damage), purchase, collateral, liquidation, lien settlement, distressed civic guarantees. Closed/insolvent occupants are evicted weekly; recovered firms re-occupy. Each district seeds one affordable vacant workshop. |
| Labor | Employment agreements, player-set wage posture, wage fairness vs market reference, capacity, conditions, loyalty, disputes, suspension, recovery, player responses. Sub-fair wages erode loyalty toward dispute; generous wages build a buffer; stingy pay stalls recovery. Employers retain one week of operating cover during settlement; closure returns workers to the household pool. |
| Institutions | Eleven guild, merchant, council, court, watch, treasury, charity, and market institutions with membership, budgets, legitimacy, coalitions, powers, terms, endowments, deterministic selection. Every trade maps to one chartered guild: guild managers sustain higher quality targets; legitimacy scales office rewards; entry restrictions reserve access for members and surcharge outsiders. |
| Political office | Commercial and capability gates, patronage, nomination, powers, directives, recurring duties funded to institutional budgets, monthly fees repaid from budgets, administrative load, coalition response, withdrawal, forfeiture, re-election limits. |
| Civic systems | Laws, differentiated public works, district conditions, grounded legal cases and settlements (filing fees fund the Civic Court), crisis response, municipal debt, private funding of unfinished public works with legitimacy earned by external contributors. |
| Relationships | Trust, fear, respect, obligation, resentment, memories, interaction dates. |
| Information | Source, confidence, subject, summary, creation, expiry, passive reports, paid market/district/counterparty intelligence. |
| AI houses | Objectives for property, supply, office, debt, legitimacy, liquidity, rival pressure; monthly upkeep with great-house stewardship penalizing hoards; credit participation that workouts aged defaults with the existing creditor before funding sound firms; speculative credit reserved for liquidity-strained or structurally losing firms. Rival supply contracts commit near real weekly input need at penalties scaled to scheduled value. |
| Crises | Seven kinds (grain, banking, fire, epidemic, guild revolt, noble demand, trade disruption) with detection, escalation, response, resolution, recovery. Responses count as containment for a bounded window. Trade disruption resolves when all routes heal; paid responses never inflate severity. Route spikes outweigh healing; levy is annual; resolved panic raises the default bar for three years. |
| Crisis standing | Grain shortage declares while staple stores thin against target stock under collapsed regional access — before shelves empty. Crisis service earns standing with diminishing returns inside one year (full, half, quarter, one-eighth); material relief is not reduced. |
| Observability | State summary, campaign projection, HTML dashboard, outbox, chronicle, audit history, validation, campaign progression, gameplay reports with causal traces, art review reports. |

## Player command surface

`PlayerCommand` in `src/systems/commands/` is authoritative. Families:

- Business capital, acquisition, investment, wage posture, operating policy
- Supply contracts and private loans
- Property purchase and liquidation
- Laws, public works, legal filing and settlement
- House governance, family council, heir designation, wards, education. Charter changes, heir designation, and ward adoption cost family unity; a divided council cannot pay.
- Institutional patronage, endowment, nomination, office directives, withdrawal
- Crisis and labor responses
- Information commissioning and leverage
- Notification acknowledgement

All callers use the same validation and mutation paths.

## Public library surface

`src/lib.rs` defines the supported integration facade: campaign construction and advancement, player commands, strategic quotes, persistence, projections, HTML rendering, gameplay analysis, art review, invariant validation.

## Persistence guarantees

- Exact current-schema round trips; older, future, and missing schemas are rejected.
- Loads require paths resolving to regular files and reject inputs larger than 256 MiB before parsing.
- Release-mode validation of references, indexes, ownership, lifecycle, numeric ranges, accounting, histories, schedules, ID allocation. Weekly obligations remain settleable within the coming fortnight; mid-week schedules keep their nominal one-week due date anchored to signing.
- RNG state and generated records required for deterministic continuation are preserved.
- Same-directory synchronized temp writes followed by atomic replacement. `SaveOutcome` distinguishes committed vs `CommittedWithDegradedDurability`.
- Boundary strictness: `PlayerCommand`/`LoanTerms`/`SupplyContractTerms`/`NewGameConfig` use `#[serde(deny_unknown_fields)]`; save JSON rejects duplicate members before parsing; top-level probe rejects non-current `schema_version`.

Serialized contract changes require a schema increment. Earlier schemas are unsupported.

## Runtime guarantees

- One canonical mutation path per operation class; validation before mutation; unchanged state on failure; revalidation for deferred commits.
- Stable ordering and typed-ID tie-breaking.
- Fixed-point arithmetic with wide ratio intermediates.
- Synchronized records, indexes, ownership, occupancy, collateral, lifecycle state.
- Campaign phases derived from durable commercial, institutional, civic, succession milestones.
- Explicit daily, weekly, monthly, annual execution order.
- Runtime invariant checks during simulation; release validation at persistence boundaries.

## Deliberate limits

- Rivergate is the only detailed scenario.
- External powers act through routes, demands, privileges, and crises, not a full diplomacy simulation.
- Religion and charity are institutionally represented without deep doctrine or faction simulation.
- No interactive graphical campaign client; clients are CLI, JSON projection, and HTML dashboard.
- Tactical combat, manual character movement, equal-detail multi-city simulation, routine crafting, repetitive dialogue, and non-systemic interiors are out of scope.

Validation workflow is in `TESTING.md`.
