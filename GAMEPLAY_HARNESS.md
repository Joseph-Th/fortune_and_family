# Gameplay Harness

The gameplay harness runs deterministic player agents through the same command and simulation APIs used by the CLI. It evaluates command reachability, strategic pacing, delayed consequences, system interaction, recovery, and multi-generation progression.

It complements behavior tests and human playtesting. It does not replace either.

## When to use it

Run the harness when a change affects:

- Candidate discoverability or command validation
- Strategic pacing, cooldowns, or eligibility
- Delayed or persistent consequences
- Cross-domain interaction
- Economic recovery and failure states
- Political progression or succession
- Gameplay report fields or semantics

Use focused runs during implementation. Use release-mode matrices for design review.

## Commands

Default matrix:

```bash
cargo run --release --locked -- playtest
```

Focused campaign:

```bash
cargo run --release --locked -- playtest \
  --days 360 \
  --persona entrepreneur \
  --background baker \
  --trace-limit 20
```

Multi-seed JSON report:

```bash
cargo run --release --locked -- playtest \
  --start-seed 1 \
  --seeds 10 \
  --days 1080 \
  --json \
  --output gameplay-report.json
```

Quality gate:

```bash
cargo run --release --locked -- playtest \
  --minimum-overall 75 \
  --fail-on-critical \
  --json \
  --output gameplay-report.json
```

The report is written before a quality-gate failure is returned.

The repository wrapper runs the same gate locally:

```bash
bash scripts/test.sh gameplay
```

The repository wrapper runs two release-mode gates. The default 12-campaign matrix enforces the minimum overall score and critical-finding threshold. A focused 7,200-day campaign then verifies that succession occurs and that the succession-and-legacy phase contains real decision cycles. `bash scripts/test.sh all` includes both gates. Focused command tests and CLI smoke coverage remain separate so report serialization, quality-gate failure behavior, generation-length progression, and broad gameplay quality are verified independently.

For design review rather than routine CI, run the deeper matrix:

```bash
bash scripts/test.sh gameplay-audit
```

The deep audit adds a 3,600-day, two-seed matrix across every persona and starting background, then a 7,200-day Baker campaign for each persona. The generation matrix must reach succession for every persona and must contain substantive succession-and-legacy decisions. This longer mode exists because rare recovery routes, credit distress, labor conflict, political overextension, and mature portfolio behavior cannot be judged reliably from the default three-year horizon or from one long Steward campaign.

## Configuration

| Option | Meaning | Default |
|---|---|---|
| `--start-seed` | First deterministic seed. | `1` |
| `--seeds` | Number of consecutive seeds. | `1` |
| `--days` | Simulated days per campaign. | `1080` |
| `--decision-interval` | Days advanced after each player decision. | `30` |
| `--max-probes` | Maximum candidate commands validated per decision. | `24` |
| `--consequence-horizon` | Maximum delayed-attribution horizon in days. | `360` |
| `--trace-limit` | Representative trace steps retained per campaign. | `40` |
| `--persona` | Repeatable persona filter. Omit for all personas. | All |
| `--background` | Repeatable background filter. Omit for all backgrounds. | All |
| `--json` | Emit the structured report. | Human text |
| `--output` | Write the report to a file. | Standard output |
| `--minimum-overall` | Fail below the specified overall score. | Disabled |
| `--fail-on-critical` | Fail when any critical finding exists. | Disabled |

The monthly decision interval matches the strategic cadence and avoids treating routine weekly simulation as required player input.

## Personas

The harness uses four deterministic priority models:

| Persona | Primary bias |
|---|---|
| `steward` | Continuity, worker conditions, relief, administration, reform, and public works. |
| `entrepreneur` | Business policy, contracts, property, commercial education, market intelligence, credit, and expansion. |
| `power-broker` | Family capacity, house intelligence, taxation, laws, courts, public works, and governance. |
| `opportunist` | Leverage, acquisition, high-yield short-term credit, counterparty intelligence, legal pressure, crisis exploitation, and replacement labor. |

Personas are diagnostic policies, not optimal strategies. Their purpose is to expose whether the command surface supports distinct priorities and outcomes. A politically weakened `power-broker` may commission a house brief on an equally or more institutionally embedded rival even before the relationship becomes openly hostile; the normal two-year intelligence cadence and office-duty reserve still prevent this from becoming routine housekeeping.

Public-work ranking also includes a modest portfolio-repeat penalty after the player has completed a project of the same kind. The first project still follows material district need and persona priorities, but close later alternatives can overtake a repeated School, Market, or other template. This is a harness-agent policy rather than a canonical gameplay rule: it prevents a deterministic persona preference from hiding whether the public-work system supports a varied civic portfolio as district conditions change.

The opportunist deliberately accepts greater operating and credit exposure than the other personas. Its growth policy spends less on maintenance, and its new lending uses the shortest, highest-yield terms a non-player borrower will normally accept. Credit candidates require an actual counterparty financing pressure such as low household liquidity, a distressed business, or delinquent debt. Solvent houses no longer accept high-yield debt merely because the player has surplus cash. When a non-player borrower accepts credit while one of its businesses is distressed, the financing unlocks a recapitalization commitment from the borrower as well: treasury above the protected household reserve is deployed into the business up to its recovery shortfall. This keeps the stress strategy inside the canonical political economy: lending can rescue a counterparty, create dependency, strengthen a rival enterprise, and expose the lender to repayment risk without the harness manufacturing a distressed borrower. The deep audit separately checks whether player credit actually changes borrower businesses, whether the long-horizon stress sample is large enough to judge repayment risk, and whether observed delinquency/default reaches a player enforcement action. Snapshots and campaign maxima distinguish delinquency/default on loans issued by the player from distress on unrelated private loans, so NPC-to-player or NPC-to-NPC defaults cannot create false coverage.

Legal pressure is likewise grievance-driven rather than synthetic. The harness only proposes a player case when the command layer can quote a concrete distressed loan or an attributable breached contract with an unpaid terminal penalty, and it uses the evidence ceiling and recoverable damages derived from that exact source. New campaigns no longer begin with an arbitrary lawsuit. Debt judgments reduce the source loan balance; contract judgments reduce only the tracked unpaid breach penalty. Money already recovered through repayment or ordinary contract settlement therefore cannot be collected a second time through court. Legacy saves may retain older cases without a persisted claim source, but new player filings always identify the obligation being litigated.

## Decision cycle

Each decision cycle:

1. Captures the current campaign state.
2. Generates concrete `PlayerCommand` candidates from state.
3. Ranks candidates by urgency, persona priorities, coverage, resources, and repetition.
4. Preserves probe capacity across command families.
5. Validates candidates through `apply_player_command` on cloned state and retains every successfully probed concrete target or template, not only the first target from each command family.
6. Selects the highest-ranked viable substantive command; notification acknowledgement is fallback housekeeping.
7. Commits through the canonical command API.
8. Advances the action branch through `advance_days`.
9. Advances a no-action baseline from the same decision point.
10. Records outcomes, scores, findings, and bounded trace data.

The harness does not directly mutate domain records during play.

## Progression model

The harness records separate fantasy milestones:

- Reputation standing
- Earned commercial standing
- First institutional support campaign
- First office campaign
- First office
- First city-shaping action
- First player labor dispute
- First succession

Institutional patronage becomes available after established reputation and 52 credited contract deliveries, representing roughly one year of reliable weekly trade for a single-contract business. This lets social ascent overlap with late establishment instead of beginning only after commercial progression is complete. Patronage transfers treasury into an institution, creates membership, improves support among member houses, and must mature for 180 days. Full commercial standing remains stricter: office nomination requires at least 78 credited deliveries, roughly 18 months of reliable weekly trade, plus established support in the target institution. Candidate preparation now matters on top of that minimum. A character whose administration, commerce, social, or craft capability is weak for the target institution needs additional credited deliveries before the house can credibly launch that candidacy, up to an additional half-year of commercial proof. Education can reduce that extra preparation by improving the relevant capability. Patronage and nomination cooldowns are tracked per character, so a larger trained family can pursue several political projects in parallel. Harness agents may take any viable institution as their first foothold, but after that they expand only toward institutions that add a new office power valued by the persona; an existing foothold remains eligible for follow-through even when it was not an ideal long-term fit. This keeps long-run political portfolios specialized instead of turning family growth into automatic universal access. A funded nomination resolves after a 120-day campaign and provides a material campaign advantage, but it no longer overwhelms candidate capability, dynasty legitimacy, and member-house relationships by itself. A weak candidate can therefore lose a funded contest, making education, coalition support, and choice of institution part of political preparation. An active nomination locks that character for one year; after the contest resolves, another nomination requires a two-year recovery from the original campaign date. Voluntarily surrendering a membership creates the same two-year recovery for that character; surrendering an actual office additionally creates a dynasty-wide political recovery window, preventing the house from immediately swapping another relative into patronage or a different candidacy. Each player-family character may belong to at most two institutions, which keeps political identity focused and makes additional trained relatives the source of broader dynastic reach. Established NPC houses begin with broader institutional networks, so this cap deliberately governs player advancement rather than all historical memberships. Office powers become available after a separate 120-day establishment period, leaving meaningful time within the 360-day term for active use rather than consuming most of the term as onboarding. Harness agents use an available directive only when its district, institution, crisis, debt, or business conditions create a material need; persona preference alone no longer turns office power into scheduled maintenance. Officeholders then face recurring duties, administrative load, and possible forfeiture. Recurring duties also include a portfolio surcharge for every office beyond the first, so a broad political network has a growing monthly carrying cost rather than behaving like a collection of independent upgrades. If a multi-office dynasty exhausts its legitimacy or can no longer maintain its forward duty reserve, the harness treats that as political overextension and can surrender an office before treasury collapse makes retreat unavoidable. Winning additional offices also creates coalition resistance among member houses: concentrated authority raises fear and resentment, reduces trust, and therefore feeds political expansion back into future election support and commercial bargaining.

Family growth remains player-directed through governance, ward adoption, focused education, and family-council intervention. Once the dynasty has the developing commercial record required for institutional patronage, harness agents may begin educating relatives and adopting wards before full office-candidacy standing. Advanced education remains limited to once per year for each person, and the house may sponsor a different family member after 180 days. This lets a larger family prepare several political and commercial specialists over time without turning education into scheduled quarterly optimization. Persona-specific family preparation can therefore shape institutional contests while leaving more decision cycles for business, civic, political, information, and recovery commitments. When unity falls below 7,000 basis points and the dynasty can fund it, harness agents may convene the family council to spend treasury on settlements, hospitality, and internal obligations; the action restores family unity and active-member loyalty and is limited to once per year. Under meaningful succession pressure, the harness either selects a strategically superior adult council member or formally confirms the default heir when no better replacement exists. A default heir may be formally confirmed once; repeated unchanged designations remain invalid. Formal preparation reduces the cohesion, loyalty, and legitimacy shock when succession occurs. Developing commercial standing gates family expansion and education; full commercial standing remains the gate for office nomination.

Rival-house relationships now have direct commercial consequences. Distrust and resentment narrow the acceptable NPC supply-contract price band, so sustained containment by another dynasty can force the player to pay a premium or accept a discount. Harness agents price proposed contracts inside the current relationship-adjusted band, and reports record the maximum relationship-driven contract pressure reached in each campaign.

Established dynasties may also commission intelligence reports. The canonical player command retains its annual cooldown, while automated personas normally wait two years between commissions and hold a new report for at least 90 days before leveraging it. Automated commissions also require material uncertainty or pressure: significant price or stock movement or a materially adverse contract-to-market gap for entrepreneurs, severe district shortfalls for stewards, and strained or strongly asymmetric rival-house conditions for political or opportunistic agents. When relationship-driven contract pressure reaches 1,500 basis points, political and opportunistic agents may use the canonical annual commission cadence because coalition resistance is already imposing a material commercial penalty and warrants active counterparty management. Commissioning and leverage are treated as activation-dependent routes in reachability diagnostics, so a calm campaign without a relevant trigger is not misreported as a broken command path. This prevents the harness from manufacturing a predictable annual two-click ritual while ensuring that severe political exposure has a timely player response.

The report treats a commission followed by leverage within 180 days as one information-use pair even when other decisions intervene. The scheduled-maintenance warning evaluates only campaigns that remained below 1,500 basis points of relationship-driven contract pressure; within those calmer campaigns it requires both a high completion share and a cadence of at least one commission every two campaign-years. Severe-pressure campaigns are excluded because their faster intelligence cadence is explicitly caused by material political exposure rather than by the calendar alone.

Automated agents preserve twelve months of projected office-duty costs plus a household liquidity buffer before taking discretionary spending actions. The projection includes held offices, every unresolved player office campaign as a possible future obligation, a proposed additional nomination, and the resulting multi-office portfolio surcharge. Emergency crisis and severe business-rehabilitation actions may override that reserve. A family-council intervention triggered by unity below 7,000 basis points may draw into the long-term buffer, but must still leave six months of projected office duties plus a smaller liquidity reserve after paying for the meeting. This keeps family recovery available under genuine internal pressure without teaching the agent to fund routine expansion by accepting predictable duty defaults. Political breadth therefore competes with commercial and family uses of treasury, while phase findings continue to report genuinely quiet periods rather than ones created only by an overly rigid reserve rule.

Crisis exploitation is intentionally not containment. It may be used once for immediate gain, leaves the crisis on its escalation trajectory, and still permits one later relief, reform, or suppression response. A containing response closes further player responses and starts monthly recovery. Epidemics impose a localized welfare shock when the outbreak is first recognized, representing harm accumulated before the monthly detection boundary; prompt containment can prevent continued losses but cannot retroactively erase the outbreak itself.

## Phase quality

The harness classifies each decision cycle by the furthest reached fantasy milestone:

| Phase | Begins when |
|---|---|
| Foundation | Campaign start. |
| Establishment | Reputation standing is reached. Late establishment can include early patronage and coalition-building. |
| Institutional ascent | Full commercial standing is reached through reputation plus 78 credited deliveries. |
| Dynastic governance | The first city-shaping law or public work is sponsored. |
| Succession and legacy | The first succession completes and the inherited organization begins operating under the next generation. |

Each phase records action share, quiet and blocked cycles, how many quiet cycles still contain autonomous world change, the longest consecutive quiet streak, viable option depth, viable command-family breadth, multi-family choice frequency, and closely ranked alternatives. It records consequence differentiation twice: once across command families and again across every concrete viable target or template inside those families. Concrete profiles expose the direction of important economic, civic, family, and risk measures plus an impact fingerprint derived from measured outcomes. Civic measurement includes average district employment, sanitation, safety, unrest, citywide plus worst-district food satisfaction, and a stable per-district endpoint profile. Cross-persona convergence compares the same districts directly instead of allowing a strong local project effect to disappear inside a citywide average. A separate strategic fingerprint preserves identity-sensitive state so two different properties, institutions, wards, laws, or districts remain inspectable even when their short-horizon measured effects are equivalent. Campaign reports also track maximum relationship-driven contract pressure and the minimum family unity observed after succession. Findings distinguish consequential time passage from genuinely static downtime and apply phase-specific limits to consecutive quiet streaks: establishment and institutional ascent are expected to remain comparatively tight, while mature governance may tolerate longer world-moving intervals without manufacturing busywork. In dynastic governance and succession/legacy, concrete target depth no longer substitutes for cross-system breadth: those phases must satisfy their command-family breadth threshold independently because mature play is specifically expected to make business, family, political, and civic commitments compete. Phase findings name the exact thresholds that failed and identify the seed, persona, and background that produced the longest drought, so an outlier can be reproduced instead of disappearing inside aggregate statistics. They warn when establishment or institutional ascent becomes mostly waiting, when political ascent collapses into repetitive campaign administration, when apparently broad choices have one obvious winner, equivalent immediate effects, or convergent short-term trajectories, when repeated property acquisition becomes a universal progression path across otherwise distinct personas, when mature civic builders converge on one public-work type, when rival hostility rarely changes commercial leverage, when mature governance is strategically narrow or develops an excessive uninterrupted drought, when succession rarely disrupts family cohesion, or when succession produces no meaningful post-transition strategy. Quiet observation while the world continues to change is reported but is not treated as dead time. A strong aggregate score must not hide a passive phase or a single severe campaign drought.

## Recovery routes

The harness must be able to discover and evaluate canonical recovery actions, including:

- Business recapitalization and internal cash transfer
- New credit when available
- Delayed restructuring of defaulted credit
- Voluntary property liquidation, including emergency sales by healthy but cash-poor dynasties with several properties
- Lien settlement from sale proceeds
- Distressed civic auction guarantees when private liquidity is insufficient
- Voluntary institutional withdrawal when office duties threaten business or household liquidity

A campaign with assets or institutional options should not be classified as unrecoverable merely because immediate cash is low. Trace context includes collateral and liquidity state so blocked recovery can be diagnosed.

## Counterfactual attribution

Routine simulation changes state without player action. The harness separates ambient changes from command consequences by advancing two branches from the same decision point:

| Branch | Operation |
|---|---|
| Action | Apply the selected command, then advance time. |
| Baseline | Apply no command, then advance the same time. |

Changed domains are classified as:

| Class | Meaning |
|---|---|
| Immediate | Changed at command commit. |
| Persistent | An immediate difference remains at the attribution horizon. |
| Delayed | A new action-attributable difference appears after time advances. |
| Ambient | The baseline changed without the command. |

Only action-attributable differences create command-to-domain interaction edges.

## Scores

Scores range from 0 to 100.

| Score | Measures |
|---|---|
| Actionability | Whether substantive candidates pass canonical validation. |
| Variety | Command-direction coverage, action distribution, command-family breadth, and projected consequence diversity among concrete viable alternatives. |
| Interconnection | Distinct command-to-domain edges and consequence breadth. |
| Feedback | Observable immediate and durable delayed results. |
| Resilience | Business continuity, citywide and worst-district food access, liquidity, labor pressure, crisis load, and material district employment, sanitation, safety, and unrest. |
| Overall | Weighted summary of the component scores. |

Use component scores and findings for diagnosis. The overall score is a coarse gate, not a complete design verdict.

## Findings

Findings use `Info`, `Warning`, or `Critical` severity. They cover conditions such as:

- Command families that are absent, blocked, unselected, or inconsequential
- Repetitive command streaks and housekeeping displacement
- Long periods without a substantive action, including severe outlier campaigns hidden by healthy aggregate averages
- Quiet decision cycles that produce neither a player action nor meaningful autonomous world change
- Asset-rich but cash-poor quiet streaks measured while the liquidity condition is actually present, not inferred from unrelated endpoint wealth
- Economic states with no viable recovery route
- Active but ineffective recovery churn that remains under severe financial pressure for at least one campaign-year through the endpoint
- Persistent business, food, labor, credit, crisis, or notification failure
- Household welfare that remains mechanically flat despite repeated crises
- District employment that collapses from the campaign baseline or remains structurally weak
- Broad civic distress in sanitation, safety, or unrest even when food access and treasury remain healthy
- Distinct civic strategies whose laws, offices, projects, or governance differ but whose material city conditions still converge
- Mature office directives that create immediate effects but no later trajectory change
- Generation-length matrices that never expose private-credit or civic-debt distress
- Public-work or office-duty overload
- Misordered, compressed, synchronized, or unreachable campaign milestones
- Weak strategic variety or persona convergence
- Near-universal property acquisition that turns a scarce portfolio choice into automatic progression
- Mature house-governance convergence after campaigns have actively rewritten their family charters
- Multi-family cycles dominated by one obviously superior option
- Viable alternatives with indistinguishable immediate consequence profiles
- Viable alternatives whose simulated trajectories converge after the command-specific attribution horizon
- Commission-and-leverage information loops that become scheduled maintenance
- Crisis responses that change the immediate record but rarely alter the future trajectory
- Patronage and nomination administration consuming too much of substantive play
- Political growth without family capacity
- Near-universal institutional representation that erodes specialization
- Long campaigns that do not reach stable succession

Absence is interpreted against the configured horizon and prerequisite availability. Event-driven and office-power-dependent commands are not treated as broken before their trigger or mature authority exists. A viable command that appears in fewer than three decision cycles but loses to a stronger alternative is informational; repeated viable non-selection remains a warning. The report records actual law and public-work activation opportunities rather than inferring availability from campaign age alone.

## Trace contract

Each retained step includes:

- Simulation day
- Compact economic, financial, political, and family context
- Considered, viable, and substantive candidate counts
- Distinct viable command families
- Ranked candidates and scores
- Every successfully probed viable concrete alternative, its score and target description, immediate and projected domains, directional measured impacts, an impact fingerprint, and an identity-sensitive strategic fingerprint
- Score distance between the strongest two command families
- Number of distinct immediate consequence profiles among viable command families
- Number of distinct projected consequence profiles among viable command families
- Number of cycles with multiple concrete viable options, close-ranked concrete alternatives, and distinct immediate or projected concrete consequence profiles
- Selected command and outcome
- Rejection summary
- Immediate, persistent, delayed, and ambient domains
- World-feedback flags

Context includes treasury, business cash and condition, deliveries, loan states, property and collateral, reputation, legitimacy, offices, total institutional memberships, distinct institutions represented, laws, public works, district employment, sanitation, safety and unrest, wards, family unity, generation, labor disputes, crises, and notifications.

The trace sampler retains representative opening, closing, and high-consequence decisions. `--trace-limit` affects diagnostics only.

## Structured report

The JSON report contains:

- `schema_version`
- Harness configuration
- Aggregate counters, scores, command statistics, and interaction edges
- Per-phase executed-command counts, so mature phases can be checked for maintenance-task dominance instead of relying only on aggregate command totals or sampled traces
- The same aggregate view split by configured persona, so route breadth, pacing, and system use can be compared without external post-processing
- Per-phase actionability, longest quiet streak, viable option depth, command-family breadth among actionable cycles, patronage-and-nomination administration share, family-level and concrete-option close-choice frequency, and immediate plus one-interval projected consequence differentiation
- Per-phase and aggregate separation of quiet cycles with ambient world change from genuinely static quiet cycles
- Per-campaign start and end snapshots
- Material civic endpoint measures for average district employment, sanitation, safety, and unrest in addition to food access, plus the stable per-district condition profile used to detect localized divergence
- Readable active-law and player-completed-public-work kind sets, so civic identity is inspectable without reverse-engineering checksums
- Fantasy-arc milestone timing plus the first institutional support target, office-campaign target, and city-shaping command, so synchronized timing can be distinguished from synchronized strategy
- Crisis kinds observed by each campaign, so domain-specific diagnostics only judge crises that can actually affect the measured outcome
- Findings and limitations
- Bounded traces

The current schema version is listed in `STATUS.md`. Consumers should reject unknown schema versions.

Increment the gameplay report schema when fields or semantics change for automated readers.

## Drift protection

The harness is coupled to the real game at the canonical boundaries rather than reimplementing them: campaigns use `build_new_game`, candidate probes and selected actions use `apply_player_command`, and all elapsed time uses `advance_days`.

Additional tests make stale coverage fail visibly:

- An exhaustive classifier maps every `PlayerCommand` variant back to its harness command family. Adding an actual player command requires updating that match to compile.
- Every generated candidate is checked against that classifier before probing. Incorrect labels, including supply-versus-sale and borrowing-versus-lending splits, abort the harness.
- The serialized top-level `AppState` component manifest is compared with an explicit observed or intentionally unobserved list. Adding a new state subsystem requires a harness review. Persistent audit history is observed because cooldowns, office-duty forfeiture, and candidate availability depend on it.
- Domain snapshots include deterministic structural checksums for businesses, households, markets, routes, contracts, finance, property, labor, relationships, characters, family state, institutions, laws, districts, public works, legal cases, crises, information, AI objectives, notifications, chronicle history, and audit history. Equal aggregate totals cannot conceal a material state transition. Audit-only changes are represented as a typed persistent-history signal rather than being mislabeled as player-facing feedback.
- Focused tests cover changes that were previously easy to miss, including district sanitation, route disruption, legal hearing progression, and pledged collateral.

## Integration checklist

A new or changed player command is harness-integrated only when all applicable items are updated:

1. `GameplayCommandKind` and exhaustive mappings
2. Candidate generation and ranking
3. Command-family and strategic-direction classification
4. Probe selection and rejection categorization
5. Snapshot fields and decision context
6. Immediate, persistent, delayed, and ambient attribution
7. Scores and findings
8. Trace rendering
9. Structured report schema
10. Harness tests and this document

## Interpretation limits

The harness can evaluate deterministic state and command behavior. It cannot establish:

- Whether a human understands the interface
- Whether decision comparison is cognitively manageable
- Whether prose creates attachment or urgency
- Whether incomplete information is presented clearly
- Whether real-time pacing feels appropriate
- Whether mechanically distinct options feel emotionally or narratively meaningful
- Whether a persistent target-identity difference is strategically valuable to a human when its measured short-horizon impact profile is otherwise equivalent
- Whether intentionally risky stress strategies are legible or attractive to a human player
- Whether a player recognizes which rival caused a setback or understands that rival's intent
- Whether a relationship-driven premium or succession shock feels fair, attributable, and emotionally salient rather than merely measurable
- How every unchosen branch develops beyond the first decision interval and across its full delayed consequence horizon
- Whether persistent state and chronicle changes feel like a coherent remembered family legacy

Use human playtesting for those questions.
