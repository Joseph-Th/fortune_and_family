# Civic Dynasty Design

This document defines the product contract: player fantasy, gameplay loop, system relationships, campaign structure, and scope. It does not describe implementation status or code structure.

Use `STATUS.md` for current capability and `ARCHITECTURE.md` for implementation ownership.

## Product definition

| Item | Definition |
|---|---|
| Genre | Dynasty simulation, economic strategy, political strategy, and social simulation |
| Setting | One late-medieval European-inspired city with abstract regional connections |
| Player role | Founder and successive heads of a merchant or artisan dynasty |
| Core fantasy | Turn useful work into commercial standing, institutional power, civic influence, and durable family continuity |
| Campaign scale | One detailed city across multiple generations |
| Primary mode | Single-player systemic campaign |

## Design thesis

Civic Dynasty is about building a family institution inside a living political economy.

The player begins with a household, a trade, limited capital, and weak protection. Reliable work creates income and reputation. Income creates property, credit, information, and patronage. Those assets create access to guilds, courts, councils, markets, charities, and public office. Institutional power changes laws, contracts, enforcement, public works, and trade access. Every gain creates obligations, rivals, dependencies, and succession risk.

The dynasty should not only become richer. It should become embedded in the city.

A feature belongs when it does at least one of the following:

- Creates economic interdependence.
- Gives people or institutions interests derived from material conditions.
- Converts one form of power into another at a meaningful cost.
- Produces persistent civic consequences.
- Changes the class of decisions available as the dynasty grows.
- Makes succession test the organization built by prior generations.

Exclude or simplify features that add routine input without strategic consequence, spectacle without persistent state, or progression detached from the simulated world.

## Core player fantasy

The fantasy has six connected parts:

1. **Productive competence.** The dynasty earns legitimacy by producing, trading, managing, lending, or serving reliably.
2. **Commercial intelligence.** The player reads shortages, counterparties, districts, credit risk, and seasonal pressure.
3. **Social ascent.** Quality, reliability, education, family strategy, service, and patronage create standing.
4. **Institutional control.** The dynasty gains authority over concrete economic and civic outcomes.
5. **Dynastic continuity.** Family members, wards, managers, allies, and heirs determine what can be controlled safely.
6. **Historical imprint.** Ownership, laws, debts, public works, institutions, relationships, and public memory preserve major decisions.

## Core loop

```text
observe conditions
  -> choose a commercial, family, social, or political commitment
  -> spend money, capacity, legitimacy, information, or obligation
  -> simulation resolves material and institutional effects
  -> relationships, opportunities, and risks change
  -> adapt the dynasty's structure and strategy
```

Routine operation should be delegated. The player should set policy, choose exceptions, allocate scarce resources, and respond to consequential changes.

## Campaign arc

### Foundation

The player stabilizes the household and first business, builds reliable supply and sales, and establishes a commercial record.

### Establishment

The dynasty acquires property, credit relationships, trained family capacity, stronger counterparties, and social standing. Once reputation and a developing commercial record are credible, the player can begin patronage and coalition-building before the dynasty is ready to contest public office.

### Institutional ascent

Sustained commercial credibility turns maturing patronage into candidacy and creates access to guild offices, courts, laws, public works, and municipal finance. Officeholding introduces duties, scrutiny, administrative load, and enemies.

### Dynastic governance

The player manages a portfolio of businesses, property, offices, obligations, family roles, and public commitments. The unit of decision shifts from individual operation to institutional governance. Officeholding must provide active directives with visible district tradeoffs, not only passive periodic bonuses. A directive creates six months of institutional momentum, so political choices continue changing businesses, households, markets, relationships, or crises after the initial order.

### Succession and legacy

A succession tests whether control depends on one exceptional person or on a durable organization. The player can prepare that test by formally confirming the default adult heir or choosing another adult council member, accepting legitimacy and family-unity costs to bind the succession into the family charter. Formal preparation reduces the cohesion, loyalty, and legitimacy shock when succession actually occurs. Legacy evaluates both family outcomes and civic consequences.

These phases may overlap, but they should remain strategically distinct. Political authority should not arrive before sustained commercial performance, and late play should not remain an enlarged version of the opening business loop.

## Design pillars

### Living political economy

Production, consumption, prices, wages, property, credit, law, and politics are causally connected. Economic changes create political interests. Political decisions create material winners and losers.

### Multiple forms of power

Cash, property, credit, reputation, legitimacy, office, information, kinship, coercion, administrative capacity, and public support are distinct resources. They can be converted, but not freely or universally.

### Growth changes the game

Expansion changes the player’s unit of decision. Direct control becomes less effective as holdings, offices, family members, and obligations increase.

### People have interests

Important characters are not interchangeable bonuses. Competence, loyalty, ambition, claims, obligations, and relationships affect what they will support, inherit, expose, or resist.

### The city remembers

Property, laws, contracts, court outcomes, public works, debts, grudges, obligations, and family reputation persist beyond the event that created them.

### Information is strategic

Information has a source, confidence, age, subject, and access path. The player should distinguish confirmed state from uncertain reports. Commissioned intelligence must unlock a discrete follow-up decision, such as renegotiating a contract, approaching a house, or targeting a district problem, with an explicit financial or political cost.

### Power creates exposure

A larger dynasty has more tools and more vulnerabilities: administrative friction, political scrutiny, concentrated assets, dependent partners, public expectations, family division, and succession risk.

### Recovery has a cost

Major setbacks should be recoverable through restructuring, asset sale, recapitalization, coalition change, marriage, office, resignation, or retreat. Recovery should cost time, wealth, position, public resources, or future obligation.

## System contracts

### Dynasty and family

Family structure determines succession, parallel capacity, office eligibility, management coverage, and long-term continuity. Education, governance, wards, marriage, loyalty, claims, and deliberate heir designation should create strategic differences rather than static bonuses. When cohesion deteriorates, the player can convene a costly family council to settle obligations and rebuild unity and loyalty, with an annual limit so reconciliation remains a response to pressure rather than routine maintenance.

### Households and labor

Households consume goods, supply labor, pay rent, experience welfare changes, and form political responses. Labor agreements connect business performance to wages, conditions, loyalty, disputes, and replacement costs.

### Businesses and production

Businesses have ownership, management, policy, inventory, cash, capacity, condition, quality, and lifecycle. Growth requires capital and competent administration. Failure and recovery must remain part of the same system.

### Markets, contracts, and credit

Prices should emerge from supply, demand, scarcity, policy, and disruption. Contracts create durable counterparties and obligations. Credit creates leverage, dependency, collateral risk, restructuring, and default consequences.

### Property and urban development

Property connects wealth to districts, rent, tenancy, occupancy, collateral, public works, and political interests. Assets should be useful, illiquid, and recoverable through sale at a real cost.

### Institutions and politics

Institutions have earned membership, offices, budgets, legitimacy, powers, and selection rules. Established reputation plus a developing commercial record opens patronage; patronage creates membership and coalition support while the dynasty continues proving itself. Sustained commercial standing and mature support then open candidacy. Political power must control concrete outcomes such as laws, debt, public works, courts, licensing, trade, or enforcement. Incumbents can spend legitimacy on time-bounded directives that intensify an office power and produce explicit civic benefits or backlash.

Officeholding is not a permanent upgrade. It consumes administrative capacity, creates recurring duties, and can be lost through failure.

Concentrating several offices in one dynasty should also create political exposure even when that dynasty is wealthy. Member houses become more fearful and resentful as one family consolidates authority, which should weaken future coalition support and feed back into commercial relationships rather than allowing cash alone to neutralize the cost of political expansion.

### Law, courts, and enforcement

Law shapes contracts, property, debt, trade, labor, inheritance, and public authority. Cases require parties, claims, evidence, procedure, judgment, and consequences. A player-filed case must identify the concrete obligation or event that created the claim; evidence and recoverable damages are bounded by that source, and a judgment must settle the source obligation rather than create a parallel recovery path. Contract-breach damages represent only the terminal contractual penalty that remained unpaid after normal settlement, so litigation cannot collect a penalty the performing party already received.

### Districts and public opinion

District conditions reflect employment, rent, food access, safety, sanitation, infrastructure, institutional presence, and ownership. Public response should follow material outcomes and remembered actions.

### Information and relationships

Relationships track distinct dimensions such as trust, fear, respect, obligation, and resentment. Information and relationships should open or close strategies rather than act as universal scores.

### Rival dynasties and external pressure

Rivals use the same economic and institutional rules as the player, pursue explicit objectives, and adapt to changing conditions. External powers affect Rivergate through trade, tolls, demands, privileges, migration, credit, and crises rather than tactical warfare.

### Events and crises

Crises should emerge from state where possible. They expose structural choices, provide warning, create several defensible responses, and leave persistent consequences.

## Player information

Primary views should answer:

- What changed?
- Why did it change?
- Who benefits or loses?
- Which obligation is due next?
- What can the player change?
- What is uncertain?
- What happens without intervention?

Major consequences require causal explanation. Forecasts should expose assumptions and uncertainty rather than present false precision.

## Anti-snowball constraints

Growth should introduce:

- Administrative friction from holdings and weak delegation
- Political visibility and coalition response
- Shared exposure to suppliers, debtors, districts, and regulation
- Dependence on skilled managers, creditors, offices, and allies
- Public obligations created by service and patronage
- Family division and succession risk
- Rival adaptation to concentrated power

A strong position should create more consequential choices, not remove risk.

## Intended decisions

The game should regularly ask the player to:

- Solve a commercial problem through a social or political route.
- Accept an economic vulnerability created by political success.
- Choose between efficient concentration and resilient diversification.
- Build obligations and dependencies instead of only accumulating money.
- Assign different family members to parallel strategies.
- Hold office while funding its duties and managing its enemies.
- Recover from failure by sacrificing assets, terms, or status.
- Preserve an organization through succession.
- Judge the city created by the dynasty’s power.

The game should not reduce the player to routine production correction, character movement, passive waiting, universal influence spending, or inevitable monopoly growth.

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

## Design review questions

Every major feature or review should answer:

1. Does it reinforce the economic-to-institutional-to-dynastic loop?
2. Does it create a meaningful decision instead of routine input?
3. Can the player explain the major consequences?
4. Does political power control something materially concrete?
5. Can growth make the dynasty less stable?
6. Do people and institutions have interests derived from their position?
7. Can the player recover from a major setback at a real cost?
8. Does the city remember the outcome?
9. Does the system support different viable strategies?
10. Does late play require different decisions from early play?
11. Does succession test the organization rather than only replace a character?
12. Is family success evaluated alongside civic consequence?

The final question is not only whether the dynasty becomes powerful. It is what kind of city that power creates and whether the family can survive the order it built.
