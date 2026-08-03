# Civic Dynasty Design

This document defines the product contract. It describes the intended player experience, system relationships, campaign structure, and scope. It does not describe implementation history or current code coverage.

Use `STATUS.md` for implemented features and `ARCHITECTURE.md` for code structure.

## Product definition

| Item | Definition |
|---|---|
| Genre | Dynasty simulation, economic strategy, political strategy, and social simulation |
| Setting | One late-medieval European-inspired city and its surrounding region |
| Player role | Founder and successive heads of a merchant or artisan dynasty |
| Core fantasy | Convert productive competence into commercial leverage, social standing, institutional power, and durable civic influence |
| Campaign scale | One detailed city, abstract regional connections, multiple generations |
| Primary mode | Single-player systemic campaign |

## Design thesis

Civic Dynasty is about building a family institution inside a living political economy.

The player begins with a household, a trade, limited capital, and weak institutional protection. Useful work creates income and reputation. Income creates property, credit, information, and patronage. Those assets create influence in guilds, courts, councils, neighborhoods, charities, and markets. Institutional power changes laws, contracts, enforcement, public works, and access to trade. These changes create winners, losers, obligations, rivals, and succession risks.

The dynasty should not merely become richer. It should become embedded in the city.

A feature belongs when it does at least one of the following:

- Creates economic interdependence.
- Gives a person or institution interests derived from material conditions.
- Converts one form of power into another at a meaningful cost.
- Produces persistent civic consequences.
- Changes the class of decisions available as the dynasty grows.
- Makes succession test the organization built by the prior generation.

A feature should be simplified or excluded when it adds routine input without strategic consequence, spectacle without persistent state, or progression detached from the simulated world.

## Core player fantasy

The intended fantasy has six connected parts.

### Productive competence

The dynasty begins by being useful. It produces goods, moves cargo, offers services, manages property, extends credit, or practices a profession. Early legitimacy comes from reliable work rather than abstract status.

### Commercial intelligence

The player identifies shortages, reliable counterparties, changing districts, credit risk, seasonal pressure, and opportunities to place capital where it creates leverage.

### Social ascent

Quality, reliability, charity, education, marriage, guild service, and public duty create standing. Wealth helps but does not automatically create legitimacy or trust.

### Institutional control

The dynasty gains influence over licenses, contracts, courts, budgets, public works, guild rules, market regulation, and officeholding. Political power must control economically concrete outcomes.

### Dynastic continuity

Relatives, spouses, heirs, branches, wards, managers, and allies form the organization. Their competence, claims, and loyalty determine what the dynasty can safely control.

### Historical imprint

The city should preserve the consequences of major decisions. Laws, ownership, districts, public works, debts, institutions, relationships, and public memory should show what the dynasty built and who paid for it.

## Design pillars

### Living political economy

Production, consumption, prices, wages, property, credit, law, and politics are causally connected. Economic changes create political interests. Political decisions create material winners and losers.

### Multiple forms of power

Cash, property, credit, reputation, legitimacy, office, information, kinship, coercion, administrative capacity, and popular support are distinct. They can be converted, but not freely or universally.

### Growth changes the game

Expansion should change the player's unit of decision. The player moves from operator to owner, from owner to patron, and from patron to institutional governor. Direct control becomes less viable as scale increases.

### People have interests

Characters are not interchangeable bonuses. They have competencies, relationships, obligations, ambitions, claims, memories, and loyalties. Important systems operate through people who can bargain, defect, inherit, expose, or resist.

### The city remembers

Property, laws, contracts, court outcomes, public works, grudges, obligations, and family reputation persist. Consequences should survive the event that created them.

### Information is strategic

The player should not automatically know every price, intention, debt, shortage, crime, or relationship. Information has sources, confidence, age, and access requirements.

### Power creates exposure

A large dynasty has more tools and more vulnerabilities. Administrative load, political scrutiny, concentrated assets, dependent partners, public expectations, family division, and succession risk prevent simple exponential growth.

### Routine work is delegated

The player sets policies, chooses exceptions, commits resources, and responds to consequential events. Routine operation should not require repetitive manual correction.

## Player experience goals

The game should regularly produce decisions such as:

- Solving a commercial problem through a social or political route.
- Accepting an economic vulnerability created by political success.
- Choosing between efficient concentration and resilient diversification.
- Building obligations and dependencies rather than only accumulating money.
- Using different family members for parallel strategies.
- Winning an office while inheriting its duties, enemies, and scrutiny.
- Facing a succession that exposes organizational weakness.
- Reviewing the city decades later and recognizing accumulated consequences.

The game should avoid making the player feel like:

- A courier moving characters between buildings.
- A clerk repeatedly correcting obvious production quantities.
- A passive observer waiting for timers.
- A monopolist whose victory becomes inevitable after early profit.
- A politician spending generic influence points detached from material interests.
- A dynasty manager whose relatives are stat packages without independent stakes.

## World model

### Primary geography

The campaign centers on one deeply simulated city with:

- Six to ten distinct districts.
- A surrounding agricultural and resource hinterland.
- Regional roads, rivers, ports, or comparable trade infrastructure.
- Abstract external markets and authorities connected through routes, contracts, demands, and crises.

One detailed city supports persistent memory, legible coalitions, recognizable families, meaningful property relations, local monopolies, and institutional continuity.

Regional expansion should occur through estates, trade privileges, branch houses, diplomatic missions, contracts, and financial interests. It should not require direct operation of several equal-detail cities.

### Population layers

Notable characters are individually simulated when their identity has strategic consequence. This includes family members, major rivals, officeholders, guild leaders, clergy, judges, creditors, managers, and other socially important people.

Ordinary residents are grouped into households. Households consume goods, supply labor, pay rent, move between districts, respond to prices, form opinions, and participate in unrest. Individuals can become notable when events make them strategically important.

### Time

The design uses several cadences:

- Daily: production, procurement, sales, local prices, maintenance, and immediate incidents.
- Weekly: wages, household consumption, contracts, rent, debt service, and recurring obligations.
- Monthly or seasonal: districts, trade routes, institutions, elections, public works, information, and crisis pressure.
- Annual: education, health, marriage, succession planning, taxation, offices, and family governance.
- Generational: inheritance, family branching, institutional continuity, and legacy.

A full campaign should normally cover fifty to eighty in-game years and two to four generations. Consequential decisions should pause or slow play; routine systems should continue under policy and delegation.

## Campaign arc

| Phase | Player organization | Primary decisions |
|---|---|---|
| Foundation | Household and one business | Cash flow, input reliability, quality, hiring, living costs, guild entry, local relationships |
| Establishment | Small portfolio and visible family | Delegation, contracts, credit, property, marriage, education, guild politics, neighborhood standing |
| Ascendancy | Major commercial and political interest | Coalitions, regulation, public contracts, rival containment, family branches, regional trade, office pursuit |
| Dominion | Institutionally powerful dynasty | Public order, reform, external pressure, debt, succession, constitutional structure, legitimacy of concentrated power |
| Legacy | Multi-generation civic institution | Preserve, divide, reform, constitutionalize, regionalize, or sacrifice the order the dynasty built |

Later phases should require fundamentally different decisions from earlier phases. The late game must not be an extended sequence of larger purchases.

## Core gameplay loop

1. Observe conditions and acquire information.
2. Form a commercial, familial, social, or political plan.
3. Commit capital, labor, relationships, reputation, legitimacy, or office power.
4. Operate directly or delegate through people and institutions.
5. Receive economic, social, and political consequences.
6. Convert gains into another form of power.
7. Defend the new position against obligations, rivals, and systemic reactions.
8. Reorganize the dynasty before the next expansion or succession.

Production is the foundation of the loop, not its final form.

## Forms of power

| Form | Strategic role |
|---|---|
| Liquid capital | Wages, purchases, taxes, emergencies, investment, and immediate bargaining power |
| Property | Income, collateral, physical control, district presence, and constitutional standing |
| Credit | Borrowing capacity, lending leverage, guarantees, and mobilization beyond current cash |
| Reputation | Audience-specific beliefs about quality, reliability, generosity, competence, piety, or ruthlessness |
| Legitimacy | Accepted right to exercise authority through law, election, service, tradition, religion, or competence |
| Influence | Relational ability to persuade or pressure particular people and institutions |
| Information | Knowledge of prices, inventories, debts, intentions, crimes, shortages, and negotiations |
| Kinship | Access, obligations, claims, continuity, and inheritance through family relationships |
| Coercive capacity | Courts, guards, watch authority, seizure, retainers, or criminal pressure |
| Administrative capacity | Ability to coordinate records, assets, managers, contracts, and institutions |
| Popular support | District and group willingness to tolerate, assist, elect, or resist the dynasty |

No single form should dominate every context. Conversion must depend on institutions, relationships, and current conditions.

## System contracts

### Dynasty and family

The dynasty is the player's organization, not a roster of fully controllable units.

The family system should support:

- Heads, heirs, branches, spouses, children, wards, and in-laws.
- Education, competencies, health, ambition, and loyalty.
- Family roles tied to real operational or institutional responsibilities.
- Governance rules and a family council.
- Marriage as a social, economic, and political contract.
- Succession that transfers control while testing claims, unity, and administrative continuity.
- Partial continuity when a branch, office, or asset survives a failed succession.

Family decisions must affect business, property, relationships, and institutional access. They should not exist as cosmetic role-playing detached from strategy.

### Households and labor

Households provide labor, consume goods, pay rent, build local opinion, and experience material conditions.

Labor should include:

- Finite workers and competing employment demand.
- Wages, skill, conditions, loyalty, and organization.
- Effects from workload, maintenance, safety, prices, and district conditions.
- Disputes, negotiation, improvement, replacement, suspension, and recovery.
- Consequences for production, reputation, unrest, and politics.

Workers should not be interchangeable capacity points. Employers should face tradeoffs between cost, stability, skill, and legitimacy.

### Businesses and production

A business has an owner, manager, location, recipe or service model, workforce, policy, cash, inventory, condition, quality, capacity, and lifecycle.

Business strategy should cover:

- Input security and supplier dependence.
- Output markets and customer concentration.
- Quality, maintenance, reserve policy, and capacity.
- Vertical integration versus diversified counterparties.
- Direct management versus delegation.
- Expansion, acquisition, recapitalization, distress, insolvency, and closure.

Production chains should create interdependence rather than isolated upgrade trees. Byproducts, substitution, storage, transport, and regional supply should matter when they create strategic alternatives.

### Economic simulation

The economy should model:

- Household and business demand.
- Scarce market supply and deterministic allocation.
- Production constrained by labor, inputs, capacity, condition, and management.
- Inventory, storage, spoilage, and reserve behavior.
- Prices derived from stock, flows, costs, laws, crises, and regional conditions.
- Business entry, failure, recovery, and asset transfer.
- Causal explanations for major price and availability changes.

The player should be able to understand why an important economic result occurred.

### Contracts, credit, and debt

Contracts should specify counterparties, goods or obligations, quantities, prices, schedules, duration, penalties, and termination conditions.

Credit should include:

- Principal, interest, schedule, collateral, guarantees, and maturity.
- Creditworthiness based on assets, history, relationships, and political confidence.
- Delinquency, restructuring, default, seizure, and reputation consequences.
- Lending as a source of information, dependency, and political leverage.

Municipal debt is valid expansion scope only when represented by one explicit civic ledger with concrete creditors, obligations, and policy effects.

### Property and urban development

Property should provide more than passive income. It creates location, collateral, tenancy, district influence, exposure to regulation, and control over commercial space.

The system should distinguish:

- Ownership, tenancy, occupancy, and business control.
- Residences, workshops, warehouses, estates, and civic infrastructure.
- Rent, condition, value, development, and seizure.
- Speculation and neighborhood change.
- Landlord interests and tenant politics.

### Institutions and politics

Institutions should control concrete resources and rules. Examples include guilds, councils, courts, market offices, watch bodies, treasuries, charities, and religious organizations.

Institutional systems should include:

- Membership and eligibility.
- Offices, terms, elections, appointments, and succession.
- Budgets, powers, duties, legitimacy, and scrutiny.
- Coalitions and audience-specific support.
- Laws, licenses, contracts, enforcement, and public works.
- Corruption, investigation, patronage, and opposition.

Political play must remain connected to the economic and social position of participants.

### Law, courts, and enforcement

Law should shape contracts, property, debt, trade, labor, inheritance, crime, and public authority.

Cases should have parties, claims, evidence, procedure, attention, judgment, and consequences. Courts should be institutions with constrained authority, not generic outcome buttons.

Coercive tools should solve immediate problems while creating cost, visibility, resentment, and legitimacy risk.

### Religion, charity, and legitimacy

Religious and charitable institutions should influence legitimacy, welfare, education, social networks, moral conflict, and crisis response.

Patronage should create obligations and standing, not function as a direct purchase of universal reputation. Different groups may interpret the same act differently.

### Districts and public opinion

Districts should have persistent economic and social identities shaped by:

- Industry and employment.
- Rent and property concentration.
- Food access and household welfare.
- Safety, sanitation, infrastructure, and public works.
- Institutional presence and local leadership.
- Support, resentment, and unrest.

Public opinion is group-specific and issue-specific. It should respond to material outcomes and remembered actions.

### Information and secrecy

Information should have a source, confidence, age, subject, and access path.

Sources may include records, contracts, guild membership, office, agents, gossip, correspondence, investigation, and trusted relationships.

The player should distinguish confirmed facts from uncertain reports. Information advantage should support planning, negotiation, and counterintelligence.

### Rival dynasties and AI

Rivals should pursue explicit objectives based on assets, relationships, institutional position, family needs, and current threats.

AI behavior should support different strategic identities, such as commercial expansion, office pursuit, property consolidation, debt reduction, patronage, or rival containment.

Rivals should:

- Use the same economic and institutional rules as the player.
- Remember important interactions.
- Change objectives when conditions change.
- Form dependencies and coalitions.
- Avoid unexplained economic bonuses.

### External powers and regional context

External actors should affect the city through trade access, tolls, demands, privileges, credit, diplomacy, migration, and crisis pressure.

War is primarily an economic, fiscal, political, and demographic condition. Tactical battlefield command is outside scope.

### Events and crises

Crises should emerge from state whenever possible. They should expose prior structural choices rather than ignore them.

A crisis should define:

- Cause and warning indicators.
- Affected districts, markets, institutions, and groups.
- Escalation and recovery conditions.
- Several defensible responses with different costs.
- Persistent consequences and public memory.

Suitable crisis families include grain shortage, banking panic, urban fire, epidemic, guild revolt, succession dispute, trade disruption, and external intervention.

## Player control and information design

The player directly controls consequential commitments and exceptions. Routine operations should be policy-driven or delegated.

Primary views should answer:

- What changed?
- Why did it change?
- Who benefits or loses?
- Which obligation is due next?
- What can the player change?
- What is uncertain?
- What will happen if no action is taken?

Useful views include city, dynasty, ledger, market, political, relationship, and chronicle perspectives.

Every major change should have a causal explanation. Forecasts should expose assumptions and uncertainty rather than present false precision.

## Anti-snowball design

Growth should create new constraints:

- Administrative friction from excessive holdings and weak delegation.
- Political visibility and coalition response.
- Portfolio exposure to common suppliers, debtors, districts, and regulations.
- Succession risk and family division.
- Dependency on skilled managers, creditors, offices, and allies.
- Public expectations created by prior patronage or officeholding.
- Rival adaptation to concentrated power.

Recovery from major setbacks should remain possible through restructuring, coalition change, asset sale, recapitalization, marriage, office, or strategic retreat. Recovery should cost time, position, or obligation rather than arrive as an arbitrary rescue.

## Late game and legacy

Late-game play should focus on governing an order rather than accumulating more businesses.

Suitable late-game decisions include:

- Institutional governance and constitutional change.
- Municipal finance and regional credit.
- Family federation or branch autonomy.
- Reform, repression, concession, or public service.
- External alignment and trade dependence.
- Succession under concentrated power.
- Preservation or transformation of civic institutions.

Legacy should evaluate both family outcomes and civic consequences. A wealthy dynasty presiding over a weakened city is a valid result, but not an unqualified success.

Possible legacy identities include civic oligarchs, merchant princes, industrial monopolists, bankers, guild constitutionalists, popular patrons, landowners, criminal sovereigns, religious benefactors, or regional trade powers.

Failure should be graded. The player may lose office but preserve the business, lose wealth but preserve the family, divide the dynasty into branches, or survive through a reduced but viable position.

## Scenario contract

A scenario defines:

- City geography and districts.
- Population groups and notable dynasties.
- Goods, industries, recipes, and regional routes.
- Institutions, constitutions, offices, and laws.
- Starting property, contracts, debts, relationships, and information.
- External powers and crisis pressures.
- Starting backgrounds and strategic asymmetries.
- Campaign-specific legacy conditions.

A new scenario should represent a different political economy, not only a different map or list of names.

## Minimum coherent game

The minimum coherent product should contain enough content to test the full economic-to-institutional-to-dynastic loop.

### World

- One city with at least six districts.
- A hinterland and several external trade routes.
- Several major dynasties and grouped households.

### Economy

- Connected food, drink, textile, fuel, metal, tool, transport, property, and credit systems.
- Businesses, labor, contracts, inventory, prices, maintenance, failure, and recovery.

### Institutions

- Craft and merchant guilds.
- Council, market office, court, watch, treasury, and charity or religious institution.
- Offices with concrete powers and obligations.

### Dynasty

- Marriage, children, education, roles, governance, family council, branches, and succession.

### Civic content

- Elections or appointments.
- Substantive economic and civic laws.
- Public works, courts, relationships, information, and crises.

### Campaign arc

- Foundation through second-generation succession.
- Commercial, political, civic, and dynastic milestones.
- Several viable legacy identities.
- At least one systemic late-game pressure.

## Content boundaries

### Included

- Deep business and economic simulation.
- Family and succession.
- Urban politics and institutions.
- Guilds, property, credit, law, courts, religion, charity, and regional trade.
- Persistent districts and city development.
- Abstract warfare and external-power effects.

### Deliberately limited

- Direct tactical combat.
- Manual movement of every character.
- Equal-detail simulation of multiple cities.
- Crafting minigames.
- Repetitive dialogue trees for routine transactions.
- Decorative interiors without systemic relevance.

### Rejected as primary design

- Linear profession ladders.
- Buildings that only generate passive income.
- Politics represented by universal influence points.
- Crime as consequence-free sabotage.
- Universal reputation detached from audience and issue.
- AI rivals with unexplained economic bonuses.
- A late game consisting only of larger production chains.

## Design validation questions

Every major feature or review should answer:

1. Does it reinforce the economic-to-institutional-to-dynastic loop?
2. Does it create a meaningful decision rather than routine input?
3. Can the player explain the major consequences?
4. Does political power control something materially concrete?
5. Can growth make the dynasty less stable?
6. Do people and institutions have interests derived from their position?
7. Can the player recover from a major setback at a real cost?
8. Does the city remember the outcome?
9. Does the system create different viable strategies?
10. Are routine tasks delegated while consequential choices remain visible?
11. Does the late game require different decisions from the early game?
12. Does succession test the organization rather than only replace a character?
13. Can two wealthy dynasties possess meaningfully different forms of power?
14. Can the player succeed without owning every production stage or institution?
15. Is family success evaluated alongside civic consequence?

## Intended experience

Civic Dynasty begins with a household surviving through useful work. It expands into contracts, property, credit, labor, and guild standing. It then becomes a game of marriage, coalitions, law, public office, and civic construction. Its late game concerns institutions, succession, legitimacy, external pressure, and historical legacy.

The final question is not only whether the dynasty becomes powerful. It is what kind of city that power creates and whether the family can survive the order it built.
