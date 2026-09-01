# Civic Dynasty Design

Product intent: player fantasy, gameplay loop, system relationships, campaign structure, and scope. Does not define code ownership or current implementation status.

Use `ARCHITECTURE.md` for ownership and `STATUS.md` for capability.

## Product definition

| Item | Definition |
|---|---|
| Genre | Dynasty simulation, economic strategy, political strategy, social simulation |
| Setting | Rivergate — one late-medieval European-inspired city with abstract regional connections |
| Player role | Founder and successive heads of a merchant or artisan dynasty |
| Core fantasy | Convert useful work into commercial standing, institutional power, civic influence, and durable family continuity |
| Campaign scale | One detailed city across multiple generations |
| Primary mode | Single-player systemic campaign |

## Design thesis

Civic Dynasty is about building a family institution inside a living political economy.

The player starts with a household, a trade, limited capital, and weak protection. Reliable work creates income and reputation, which create property, credit, information, patronage, and family capacity. Institutional access converts those assets into authority over laws, contracts, public works, courts, trade, and enforcement. Every gain creates obligations, rivals, dependencies, and succession risk.

The dynasty becomes embedded in the city, not merely wealthy.

A feature belongs when it does at least one of:

- Creates economic interdependence.
- Gives people or institutions interests derived from material conditions.
- Converts one form of power into another at meaningful cost.
- Produces persistent civic consequences.
- Changes the class of decisions available as the dynasty grows.
- Makes succession test the organization built by prior generations.

Exclude features that add routine input without strategic consequence, spectacle without persistent state, or progression detached from the simulated world.

## Player fantasy

1. **Productive competence** — produce, trade, manage, lend, or serve reliably.
2. **Commercial intelligence** — read shortages, counterparties, districts, credit risk, disruption.
3. **Social ascent** — convert quality, reliability, education, family strategy, service, and patronage into standing.
4. **Institutional control** — gain authority over concrete economic and civic outcomes.
5. **Dynastic continuity** — build family, management, alliance, and succession capacity that survives individual characters.
6. **Historical imprint** — leave durable ownership, laws, debts, works, relationships, and public memory.

## Core loop

```text
observe conditions
  -> choose a commercial, family, social, or political commitment
  -> spend money, capacity, legitimacy, information, or obligation
  -> simulation resolves material and institutional effects
  -> relationships, opportunities, and risks change
  -> adapt the dynasty's structure and strategy
```

Routine operation is delegated. The player sets policy, allocates scarce resources, chooses exceptions, and responds to consequential change.

## Campaign arc

### Foundation

Stabilize the household and first business. Learn the local market and build a reliable commercial record.

### Establishment

Acquire stronger counterparties, property, credit relationships, trained family capacity, information, and social standing.

### Institutional ascent

Convert commercial credibility and patronage into membership, office, civic influence, and political obligations.

### Dynastic governance

Manage a portfolio of businesses, property, offices, family roles, debts, relationships, and public commitments. Decisions shift from operating one enterprise to governing an institutionally embedded house.

### Succession and legacy

Test whether control depends on one person or on a durable organization. Inheritance alters cohesion, legitimacy, responsibilities, and strategic position while preserving earlier consequences. Succession arrives within a playable horizon: founders start old enough that the first transition occurs in the same session that builds the dynasty.

Phases may overlap, but remain strategically distinct. Political authority requires sustained commercial credibility; late play is not an enlarged opening loop.

## Design pillars

### Living political economy

Production, consumption, prices, wages, property, credit, law, institutions, and public conditions are causally connected. Economic changes create political interests; political decisions create material winners and losers.

### Multiple forms of power

Cash, property, credit, reputation, legitimacy, office, information, kinship, coercion, administrative capacity, and public support are distinct resources. Conversion has limits and costs.

### Growth changes the game

Expansion increases delegation, coordination, exposure, and obligation. A larger dynasty gains strategic reach while becoming harder to manage safely.

### People and institutions have interests

Competence, loyalty, ambition, claims, obligations, membership, ownership, and relationships affect behavior and available strategies. Characters and institutions are not interchangeable bonuses.

### The city remembers

Ownership, contracts, court outcomes, public works, debts, relationships, laws, and family reputation persist beyond the event that created them.

### Information is strategic

Information has source, confidence, age, subject, and access path. It reduces uncertainty or opens a concrete follow-up decision; it is not a universal score.

### Power creates exposure

Wealth and office create administrative burden, scrutiny, coalition resistance, dependent counterparties, public obligations, family division, and succession risk.

### Recovery has a cost

Failure is recoverable through restructuring, asset sale, recapitalization, coalition change, office change, family intervention, retreat, or a final legal write-off after collectible assets are exhausted. A write-off is a lender loss, never repayment, and carries severe standing damage. Recovery consumes time, wealth, position, status, or future obligation. No failed claim traps a dynasty solely because the procedure cannot run again.

## System expectations

| Domain | Design expectation |
|---|---|
| Dynasty and family | Family structure changes succession, management capacity, office reach, loyalty, continuity. Education and governance create strategic specialization. |
| Households and labor | Households consume, work, pay rent, experience welfare changes, respond politically. Wage posture is a standing commitment: fairness is judged against the cost of living, stingy pay erodes loyalty toward dispute, generous pay buys resilience. |
| Businesses | Distinct cash, inventory, policy, management, capacity, quality, condition, lifecycle. Growth and recovery require capital and administration. |
| Markets and contracts | Prices respond to supply, demand, scarcity, policy, disruption. Contracts create durable counterparties and obligations. |
| Credit and debt | Leverage, dependency, collateral risk, restructuring, enforceable consequences, creditor continuity, and explicit lender losses when a final judgment is uncollectible. |
| Property | Links wealth to districts, rent, occupancy, collateral, public works, political interests. Useful but not perfectly liquid. |
| Institutions and office | Access is earned. Office controls concrete civic outcomes while imposing duties, load, coalition response, and loss risk. |
| Law and courts | Claims arise from concrete obligations or events. Procedure resolves the underlying dispute rather than creating duplicate economic paths. |
| Districts | Employment, food access, rent, safety, sanitation, infrastructure, ownership, and institutions shape local conditions and public response. |
| Relationships | Trust, fear, respect, obligation, and resentment alter cooperation, access, and risk. |
| Rivals | Rival dynasties pursue explicit objectives through the same political economy, not a separate ruleset. |
| Crises | Expose structural weaknesses, offer several defensible responses, and leave persistent consequences. |

## Player information contract

Primary views answer:

- What changed?
- Why did it change?
- Who benefits or loses?
- Which obligation is due next?
- What can the player change?
- What is uncertain?
- What happens without intervention?

Major consequences require causal explanation. Forecasts expose assumptions and uncertainty rather than false precision.

## Anti-snowball constraints

Growth introduces administrative friction, political visibility, shared economic exposure, dependence on skilled people and allies, public obligations, family division, succession risk, and rival adaptation. A strong position creates more consequential choices; it does not remove risk.

## Intended decisions

The game regularly asks the player to:

- Solve a commercial problem through a social or political route.
- Accept an economic vulnerability created by political success.
- Choose between efficient concentration and resilient diversification.
- Build obligations and dependencies instead of only accumulating money.
- Assign family members to parallel strategies.
- Hold office while funding its duties and managing opposition.
- Recover from failure by sacrificing assets, terms, time, or status.
- Preserve an organization through succession.
- Judge the city produced by the dynasty's power.

The player is not reduced to routine production correction, character movement, passive waiting, universal influence spending, or inevitable monopoly growth.

## Scope

Included: deep city-scale economic simulation; family, education, governance, succession; businesses, labor, contracts, property, credit, debt; guilds, offices, law, courts, public works, charity, legitimacy; persistent districts, relationships, information, crises, regional trade.

Deliberately limited: tactical combat, manual movement of every character, equal-detail multi-city simulation, crafting minigames, repetitive dialogue trees for routine transactions, decorative interiors without systemic effects.

A new scenario represents a different political economy, not only a different map.

## Design review checklist

1. Does the feature reinforce the economic-to-institutional-to-dynastic loop?
2. Does it create a meaningful decision instead of routine input?
3. Can the player explain its major consequences?
4. Does political power control something materially concrete?
5. Can growth make the dynasty less stable?
6. Do people and institutions have interests derived from their position?
7. Can the player recover from a major setback at real cost?
8. Does the city remember the outcome?
9. Does the system support distinct viable strategies?
10. Does late play require different decisions from early play?
11. Does succession test the organization rather than replace a character?
12. Is family success evaluated alongside civic consequence?
