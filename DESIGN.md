# Civic Dynasty Design

This document defines product intent: player fantasy, gameplay loop, system relationships, campaign structure, and scope. It does not define code ownership or current implementation status.

Use `ARCHITECTURE.md` for implementation ownership and `STATUS.md` for current capability.

## Product definition

| Item | Definition |
|---|---|
| Genre | Dynasty simulation, economic strategy, political strategy, social simulation |
| Setting | Rivergate, one late-medieval European-inspired city with abstract regional connections |
| Player role | Founder and successive heads of a merchant or artisan dynasty |
| Core fantasy | Convert useful work into commercial standing, institutional power, civic influence, and durable family continuity |
| Campaign scale | One detailed city across multiple generations |
| Primary mode | Single-player systemic campaign |

## Design thesis

Civic Dynasty is about building a family institution inside a living political economy.

The player begins with a household, a trade, limited capital, and weak protection. Reliable work creates income and reputation. Those create property, credit, information, patronage, and family capacity. Institutional access converts those assets into authority over laws, contracts, public works, courts, trade, and enforcement. Every gain also creates obligations, rivals, dependencies, and succession risk.

The dynasty should become embedded in the city, not merely wealthy.

A feature belongs when it does at least one of the following:

- Creates economic interdependence.
- Gives people or institutions interests derived from material conditions.
- Converts one form of power into another at a meaningful cost.
- Produces persistent civic consequences.
- Changes the class of decisions available as the dynasty grows.
- Makes succession test the organization built by prior generations.

Exclude or simplify features that add routine input without strategic consequence, spectacle without persistent state, or progression detached from the simulated world.

## Player fantasy

1. **Productive competence.** Produce, trade, manage, lend, or serve reliably.
2. **Commercial intelligence.** Read shortages, counterparties, districts, credit risk, and disruption.
3. **Social ascent.** Convert quality, reliability, education, family strategy, service, and patronage into standing.
4. **Institutional control.** Gain authority over concrete economic and civic outcomes.
5. **Dynastic continuity.** Build family, management, alliance, and succession capacity that survives individual characters.
6. **Historical imprint.** Leave durable ownership, laws, debts, works, relationships, and public memory.

## Core loop

```text
observe conditions
  -> choose a commercial, family, social, or political commitment
  -> spend money, capacity, legitimacy, information, or obligation
  -> simulation resolves material and institutional effects
  -> relationships, opportunities, and risks change
  -> adapt the dynasty's structure and strategy
```

Routine operation should be delegated. The player should set policy, allocate scarce resources, choose exceptions, and respond to consequential change.

## Campaign arc

### Foundation

Stabilize the household and first business. Learn the local market and build a reliable commercial record.

### Establishment

Acquire stronger counterparties, property, credit relationships, trained family capacity, information, and social standing.

### Institutional ascent

Convert commercial credibility and patronage into membership, office, civic influence, and political obligations.

### Dynastic governance

Manage a portfolio of businesses, property, offices, family roles, debts, relationships, and public commitments. The unit of decision shifts from operating one enterprise to governing an institutionally embedded house.

### Succession and legacy

Test whether control depends on one person or on a durable organization. Inheritance should alter cohesion, legitimacy, responsibilities, and strategic position while preserving the consequences of earlier generations. Succession must arrive within a playable horizon: founders begin old enough that succession pressure matures while the dynasty is established, so the first transition happens in the same session that builds the dynasty, rather than only in generation-length simulations.

The phases may overlap, but they must remain strategically distinct. Political authority should require sustained commercial credibility, and late play should not be an enlarged version of the opening business loop.

## Design pillars

### Living political economy

Production, consumption, prices, wages, property, credit, law, institutions, and public conditions are causally connected. Economic changes create political interests; political decisions create material winners and losers.

### Multiple forms of power

Cash, property, credit, reputation, legitimacy, office, information, kinship, coercion, administrative capacity, and public support are distinct resources. Conversion between them must have limits and costs.

### Growth changes the game

Expansion increases delegation, coordination, exposure, and obligation. A larger dynasty should gain strategic reach while becoming harder to manage safely.

### People and institutions have interests

Characters and institutions are not interchangeable bonuses. Competence, loyalty, ambition, claims, obligations, membership, ownership, and relationships should affect behavior and available strategies.

### The city remembers

Ownership, contracts, court outcomes, public works, debts, relationships, laws, and family reputation persist beyond the event that created them.

### Information is strategic

Information has source, confidence, age, subject, and access path. It should reduce uncertainty or open a concrete follow-up decision, not function as a universal score.

### Power creates exposure

Wealth and office create administrative burden, scrutiny, coalition resistance, dependent counterparties, public obligations, family division, and succession risk.

### Recovery has a cost

Failure should be recoverable through restructuring, asset sale, recapitalization, coalition change, office change, family intervention, retreat, or a final legal write-off after collectible assets are exhausted. A write-off is a lender loss, never repayment, and carries severe standing damage for the borrower. Recovery must consume time, wealth, position, status, or future obligation; no failed claim should trap a dynasty forever solely because the underlying procedure cannot run again.

## System expectations

| Domain | Design expectation |
|---|---|
| Dynasty and family | Family structure changes succession, management capacity, office reach, loyalty, and continuity. Education and governance create strategic specialization. |
| Households and labor | Households consume, work, pay rent, experience welfare changes, and respond politically. Wage posture is a standing commitment: fairness is judged against the cost of living, stingy pay erodes loyalty toward dispute, and generous pay buys resilience. Labor connects business outcomes to people. |
| Businesses | Businesses have distinct cash, inventory, policy, management, capacity, quality, condition, and lifecycle. Growth and recovery require capital and administration. |
| Markets and contracts | Prices respond to supply, demand, scarcity, policy, and disruption. Contracts create durable counterparties and obligations. |
| Credit and debt | Credit creates leverage, dependency, collateral risk, restructuring, enforceable consequences, creditor continuity after default, and explicit lender losses when a final judgment proves uncollectible. |
| Property | Property links wealth to districts, rent, occupancy, collateral, public works, and political interests. It is useful but not perfectly liquid. |
| Institutions and office | Access is earned. Office controls concrete civic outcomes while imposing duties, administrative load, coalition response, and loss risk. |
| Law and courts | Claims arise from concrete obligations or events. Procedure and judgment must resolve the underlying dispute rather than create duplicate economic paths. |
| Districts | Employment, food access, rent, safety, sanitation, infrastructure, ownership, and institutions shape local conditions and public response. |
| Relationships | Trust, fear, respect, obligation, and resentment alter cooperation, access, and risk. |
| Rivals | Rival dynasties pursue explicit objectives through the same political economy rather than a separate ruleset. |
| Crises | Crises expose structural weaknesses, offer several defensible responses, and leave persistent consequences. |

## Player information contract

Primary views should answer:

- What changed?
- Why did it change?
- Who benefits or loses?
- Which obligation is due next?
- What can the player change?
- What is uncertain?
- What happens without intervention?

Major consequences require causal explanation. Forecasts should expose assumptions and uncertainty rather than false precision.

## Anti-snowball constraints

Growth should introduce administrative friction, political visibility, shared economic exposure, dependence on skilled people and allies, public obligations, family division, succession risk, and rival adaptation.

A strong position should create more consequential choices, not remove risk.

## Intended decisions

The game should regularly ask the player to:

- Solve a commercial problem through a social or political route.
- Accept an economic vulnerability created by political success.
- Choose between efficient concentration and resilient diversification.
- Build obligations and dependencies instead of only accumulating money.
- Assign family members to parallel strategies.
- Hold office while funding its duties and managing opposition.
- Recover from failure by sacrificing assets, terms, time, or status.
- Preserve an organization through succession.
- Judge the city produced by the dynasty's power.

The player should not be reduced to routine production correction, character movement, passive waiting, universal influence spending, or inevitable monopoly growth.

## Scope

Included:

- Deep city-scale economic simulation
- Family, education, governance, and succession
- Businesses, labor, contracts, property, credit, and debt
- Guilds, offices, law, courts, public works, charity, and legitimacy
- Persistent districts, relationships, information, crises, and regional trade

Deliberately limited:

- Tactical combat
- Manual movement of every character
- Equal-detail simulation of multiple cities
- Crafting minigames
- Repetitive dialogue trees for routine transactions
- Decorative interiors without systemic effects

A new scenario should represent a different political economy, not only a different map or set of names.

## Design review checklist

1. Does the feature reinforce the economic-to-institutional-to-dynastic loop?
2. Does it create a meaningful decision instead of routine input?
3. Can the player explain its major consequences?
4. Does political power control something materially concrete?
5. Can growth make the dynasty less stable?
6. Do people and institutions have interests derived from their position?
7. Can the player recover from a major setback at a real cost?
8. Does the city remember the outcome?
9. Does the system support distinct viable strategies?
10. Does late play require different decisions from early play?
11. Does succession test the organization rather than simply replace a character?
12. Is family success evaluated alongside civic consequence?
