# STATUS.md

## Current milestone

Complete minimum coherent Rivergate game engine.

The repository now implements the full architectural foundation and a broad, integrated version of every major system required by the minimum coherent game in `DESIGN.md`. It is a deterministic headless game with a CLI, command API, projection API, HTML dashboard, and player-behavior testing harness rather than a narrow vertical slice.

## Architecture

- Rust 2024 library and CLI binary.
- Registry / AppState / Record / System ownership model.
- Code-owned immutable scenario definitions.
- Typed IDs for registry and runtime references.
- Fixed-point `Money` and `Quantity` values.
- Serializable state-owned deterministic RNG.
- Private synchronized character, business, and household stores.
- One canonical institution runtime record per definition; no parallel officeholder state.
- Canonical validate / resolve / commit mutation paths.
- Revalidated commit tokens for cross-record contracts and loans.
- Dedicated error enums for persistence, simulation, strategic operations, and commands.
- Fallible, bounded, whitespace-normalizing new-campaign input handling.
- Explicit daily, weekly, monthly, and annual execution order.
- Debug invariant validation after every simulated day and every CLI command.
- Deterministic gameplay agents with no-action counterfactual consequence attribution.
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
- Capacity-adjusted input and output reserve policies with nonnegative sale planning.
- Production capacity constrained by active employment agreements.
- Manager craft and commerce capabilities affect production yield and market throughput.
- Daily operating costs kept distinct from weekly household wages.
- Household demand and food satisfaction.
- Fixed-point price formation with causal records.
- Seasonal pressure, spoilage, and regional supply.
- Business policies, cash reserves, maintenance, condition, quality targets, distress, insolvency, terminal closure, and recorded recovery.
- Administrative-capacity penalties.
- Canonical acquisition and recapitalization of distressed, insolvent, or closed businesses with
  manager replacement, seller payment, ownership-index updates, and administrative-load transfer.
- Canonical dynasty-to-business capitalization with treasury validation, finance versioning,
  audit history, and durable notification.
- Canonical inter-business cash transfers.

### Contracts, finance, and property

- Explicit supply contracts.
- Weekly delivery, payment, breach, penalty, fulfillment, and termination.
- Symmetric nonperformance attribution and dynasty reliability consequences.
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

- Heads, heirs, roles, capabilities, annually changing health, loyalty, and lifecycle.
- Family links and parent-child records.
- Family councils, unity, charters, and governance models.
- Education progression.
- Dynastic marriages and relationship consequences.
- Health- and governance-risk-aware annual succession.
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
- Dynasty quality and reliability reputations derived from operational and financial behavior.
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
- Durable crisis-resolution notifications and one-active-instance guards for recurring crisis kinds.

### Adapters and presentation

- Human-readable JSON persistence with synchronized same-directory temporary writes and atomic replacement.
- Schema version 4.
- Explicit migrations from versions 0, 1, 2, and 3.
- Deterministic strategic hydration for version 1 Rivergate saves, including preservation of legacy officeholders.
- Version 2 migration consolidates duplicate institution state and removes redundant business staffing data.
- Version 3 migration removes the unused parallel business-debt aggregate; explicit loan records remain authoritative.
- Release-mode load validation for registry alignment, references, synchronized indexes, numeric ranges, role and lifecycle consistency, administrative-load derivation, contract compatibility, employment validity, property occupancy, reciprocal collateral, family councils, chronological histories, and next-ID allocators.
- Durable outbox notifications.
- Compact state summary.
- Complete campaign projection.
- Self-contained HTML dashboard with escaped visible content and script-safe embedded JSON.
- CLI commands for new, simulate, summary, inspect, dashboard, execute, validate, and playtest.
- JSON `PlayerCommand` API for every consequential player mutation currently exposed.

### Gameplay testing

- State-derived command candidates covering every exposed player-command family.
- Steward, entrepreneur, power-broker, and opportunist decision policies.
- Canonical candidate validation on cloned state before committing the selected action.
- Paired action and no-action simulation branches for immediate, delayed, and ambient attribution.
- Actionability, variety, interconnection, feedback, resilience, and overall experience scores.
- Command reachability, rejection pressure, action concentration, business distress and survival,
  and experience-variance findings.
- Bounded reproducible traces and complete JSON reports for CI and design analysis.
- Configurable seeds, starts, personas, simulation horizon, decision interval, probe limit, and trace retention.

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
- Institution membership, officeholder, budget, and registry consistency.
- Family councils and relationship pairs.
- Information, AI objective, court, route, crisis, outbox, chronicle, and audit dates.
- Deterministic chronological ordering.
- Save/load serialization completeness.

## Audit hardening

The August 2026 codebase audit removed or corrected:

- Negative sale quantities caused by treating integer saturation as domain clamping.
- Repeated office-nomination, governance, policy, crisis-exploitation, and AI legitimacy exploits.
- Production spending through protected cash reserves.
- Free positive-value transfers caused by fixed-point truncation.
- Inert quality targets, manager capabilities, health, loyalty, succession risk, and reputation fields.
- Arbitrary contract penalties when both parties failed, silent crisis resolution, and inconsistent loan reputation effects.
- Closed-business reopening, unreported insolvency recovery, and zero-value payroll version churn.
- Release-save acceptance of invalid roles, dates, policies, indexes, administrative loads, histories, and unsupported active laws.
- Duplicate business debt authority in favor of explicit loan records.
- Oversized validation and settlement functions that violated the warning-free structural lint gate.
- Operating-policy and labor mutations on inactive businesses, plus contract settlement that could
  still move cash or inventory through closed and insolvent parties.
- Household labor overallocation by enforcing finite worker pools in bootstrap, commands, debug
  invariants, and release-save validation.
- Duplicate unresolved lawsuits and identical law reenactments that could charge costs or create
  redundant history without a new strategic decision.
- Trade crises detected after route recovery, delayed trade effects, and guild revolts generated
  without any labor dispute or restrictive guild pressure.
- Missing audit-log ordering checks and incomplete market-flow and business-quality assertions.
- Silent omission of authored bootstrap contracts or loans when an immediately committed validated
  token failed.

## Gameplay harness tuning

The August 2026 gameplay review ran short, three-year, and six-year campaigns across every start,
persona, and multiple deterministic seeds. The initial report exposed a collapse loop that the old
headline score understated: most player portfolios lacked a healthy business, food access approached
zero outside favorable starts, labor disputes became permanent, contract breaches and credit defaults
outnumbered successful outcomes, crisis responses crowded out strategic play, and notification volume
became unusable.

The review corrected the underlying systems rather than only retuning agent priorities:

- Production recipes, quality yield, payroll, maintenance, batch rounding, and cost-aware price
  floors now support viable operating margins even at one-batch recovery utilization.
- Production throttles against output reserves, active contract obligations, and actual market
  capacity instead of spending indefinitely on saturated inventory.
- Households create sustained demand for fuel, cloth, and tools in addition to bread and ale.
- Contract sellers reserve inventory for delivery, and disputed workforces retain reduced capacity
  with a systemic payroll-based recovery path.
- Distressed firms liquidate discretionary reserves, preserve a one-batch operating float through
  payroll, and are classified from usable operating cash rather than gross reserved cash.
- Profitable businesses distribute bounded dividends, unoccupied commercial property earns external
  rent, and office powers now affect contracts, budgets, reputation, imports, safety, and employment.
- Funded office nominations schedule a timely election and can win political power; office powers
  then produce monthly economic and civic effects.
- Governance models affect administrative throughput, annual family cohesion, and succession risk,
  with an annual charter-amendment interval that prevents constitutional churn.
- Business operating policies have a 90-day strategy interval, preventing weekly template churn
  while preserving deliberate operational pivots.
- Crisis responses have a response interval, notification acknowledgement clears a backlog, and the
  harness no longer spends most decision cycles repeatedly selecting housekeeping actions.

The gameplay report schema is now version 4. It separates causal from ambient domain transitions,
distinguishes persistent immediate effects from genuinely delayed consequences, probes slow commands
over command-specific bounded horizons, measures distinct viable command families rather than raw
candidate variants, excludes notification acknowledgement from substantive scores, tracks mid-campaign
collapse and recovery, records peak office attainment rather than relying on endpoint incumbency, and
emits direct findings for severe single-campaign food collapse, labor, contracts, credit, notification
overload, crisis concentration, and autonomous-but-unresponsive systems.

## Verification status

The release gate requires all of the following to pass:

- `cargo fmt --all -- --check`
- `cargo check --all-targets --locked`
- `bash scripts/test.sh fast`
- `cargo test --quiet --locked --doc`
- `bash scripts/test.sh soak`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked`
- `bash scripts/verify_cli.sh`
- CLI create, simulate, summary, inspect, execute, dashboard, validate, playtest, and invalid-input smoke tests

The fast suite compiles and runs only non-ignored library tests. It currently contains 125 passing
tests, with two ignored long-horizon tests in the soak tier. It uses a shared immutable registry
fixture and fresh per-test campaign state, with domain filters and exact-name execution for focused
iteration. Large suites live in dedicated `*_tests.rs` files and are grouped by contracts, loans,
laws, crises, migrations, validation, gameplay, determinism, and soak coverage. The automated suite
includes failure-path rollback coverage, deterministic gameplay-report reproduction, atomic save
replacement, diagnostic first-difference reporting, a 3,000-day core invariant soak, and a
7,200-day strategic soak.

## Deliberate boundaries

The implementation follows the exclusions in `DESIGN.md`:

- No tactical battlefield mode.
- No manual movement of every character.
- No several-city simulation at Rivergate's detail level.
- No crafting minigames.
- No repetitive routine dialogue trees.
- No decorative interiors without systemic effects.

Future content expansion can add more authored industries, constitutions, cultures, scenarios, religious structures, external powers, and presentation clients without replacing the current state model or canonical pipelines.