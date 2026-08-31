# Gameplay Harness

The gameplay harness runs deterministic player agents through the same campaign construction, player-command, and simulation APIs used by the rest of the application. It evaluates whether the implemented systems produce reachable, varied, consequential, recoverable, and multi-generation play.

It complements behavioral tests and human playtesting; it replaces neither.

## Source of truth

- Implementation and report schema: `src/gameplay/`
- Behavioral coverage: `src/gameplay_tests.rs`
- Test tiers: `TESTING.md`
- Product goals evaluated by the harness: `DESIGN.md`

This document owns harness mechanics, report semantics, and integration requirements. Agent ranking constants and thresholds that do not define an external report contract are authoritative in code.

## When to use it

Run gameplay analysis when a change affects:

- Candidate discoverability or player-command validation
- Strategic pacing, cooldowns, or eligibility
- Delayed or persistent consequences
- Cross-domain interaction
- Recovery and failure states
- Political progression or succession
- Gameplay report fields, scores, findings, or traces

Use focused runs while implementing. Use release matrices for cross-domain validation and design review.

## Commands

Focused campaign — debug CLI for <1s warm iteration (solo default):

```bash
bash scripts/test.sh playtest --days 90 --persona entrepreneur --background baker
# release-faithful when you need gate throughput:
CIVIC_DYNASTY_PROFILE=release bash scripts/test.sh playtest --days 360 --persona entrepreneur
# or directly via cargo
cargo run --locked -- playtest --days 90 --persona steward
cargo run --release --locked -- playtest --days 360 --persona entrepreneur --trace-limit 20
```

Default matrix (all personas, backgrounds, and seeds) and structured output:

```bash
cargo run --release --locked -- playtest
cargo run --release --locked -- playtest \
  --start-seed 1 \
  --seeds 10 \
  --days 1080 \
  --json \
  --output gameplay-report.json
```

Repository gates (always release, ~8s–20s warm):

```bash
bash scripts/test.sh gameplay        # 36 + 3 campaigns, 60k simulated days, release gates
bash scripts/test.sh gameplay-audit  # larger multi-seed / generation / credit-stress matrices
```

Every run prints a concise progress line to stderr: elapsed time, campaign count, simulated days, actions, overall score, finding count, and simulated days per second. A quality-gate failure reports the exact score reason in the error output.

Solo-dev rule: `playtest` defaults to the debug CLI so `edit → playtest --days 90` stays <1s warm. Set `CIVIC_DYNASTY_PROFILE=release` or use `gameplay`/`gameplay-audit` when you need release-faithful simulation throughput and gate scores.

## Configuration

`GameplayHarnessConfig::default` in `src/gameplay/` is authoritative.

| Option | Meaning | Default |
|---|---|---|
| `--start-seed` | First deterministic seed; omit to rotate deterministically by UTC day | Daily rotation |
| `--seeds` | Number of consecutive seeds; omit to use the configured default | `3` |
| `--days` | Simulated days per campaign | `1080` |
| `--decision-interval` | Normal days advanced after a player decision | `30` |
| `--max-probes` | Maximum candidate commands validated per decision | `16` |
| `--consequence-horizon` | Maximum delayed-attribution horizon in days | `360` |
| `--trace-limit` | Chronological decision-trace steps retained per campaign | `40` |
| `--decision-log` | Campaigns whose full retained trace renders in the human report; `0` disables | `3` |
| `--persona` | Repeatable persona filter; omit for all | All |
| `--background` | Repeatable background filter; omit for all | All |
| `--json` | Emit structured report | Human text |
| `--output` | Write report to a file | Standard output |
| `--minimum-overall` | Fail below overall score | Disabled |
| `--fail-on-critical` | Fail if a critical finding exists | Disabled |

The decision interval is an observation cadence, not a limit on how many commands a human player could issue.

### Parallelism and world sampling

Independent campaigns run in parallel with the machine's available parallelism
capped by `CIVIC_DYNASTY_JOBS` when set
(e.g. `CIVIC_DYNASTY_JOBS=4` keeps a busy desktop responsive);
each campaign owns its `AppState` and the registry is immutable,
so scheduling changes no results. Report ordering is fixed by seed,
background, and persona. A single-campaign matrix stays serial and probes
its counterfactuals on a small bounded worker set instead.
The same `CIVIC_DYNASTY_JOBS` variable also caps `cargo --jobs`
in `scripts/test.sh`, so one knob governs both build and harness
parallelism for targeted, non-disruptive iteration.

The default matrix samples three independent world seeds. Personas share a world whenever the seed is fixed, so world-content claims — crisis variety, counterparty failure rates, civic drift — need several worlds before "never detected" means anything. Agent-choice claims (persona variety, command coverage) aggregate across every campaign.

Recurring runs should not replay identical worlds forever: the CLI rotates its default seed base deterministically by UTC calendar day (printed as `world seed base` on stderr), so scheduled gates sample fresh worlds while every run remains exactly reproducible from the configuration recorded in its report or log line. Pass `--start-seed` to pin a world explicitly; the library-level `GameplayHarnessConfig::default` stays fully deterministic.

## Personas

Personas are deterministic diagnostic policies, not optimal strategies.

| Persona | Bias |
|---|---|
| `steward` | Continuity, labor conditions, relief, administration, reform, public works |
| `entrepreneur` | Business policy, contracts, property, education, market intelligence, credit, expansion |
| `power-broker` | Family capacity, institutions, intelligence, laws, courts, public works, governance |
| `opportunist` | Leverage, acquisition, higher-risk credit, legal pressure, crisis exploitation, replacement labor |

Personas expose different viable routes through the same canonical systems. Candidate generation may rank and risk-weight per persona, but it must not create domain state or bypass canonical validation.

Standing policies that apply across personas:

- Candidate scores include a small reproducible exploration variation derived from campaign state. It can flip close calls without overriding urgency, reserve protection, or persona priorities.
- Optional standing expenses — family education, ward adoption, institution patronage — respect a shared discretionary floor: an emergency reserve plus two months of committed loan service. A house below the floor defers standing spending.
- Standing-burning responses (suppression, profiteering) additionally respect a legitimacy reserve, exactly as treasury policy reserves cash against known obligations.

## Decision cycle

Each decision cycle:

1. Capture the current campaign state.
2. Generate concrete `PlayerCommand` candidates from state.
3. Rank candidates by urgency, persona priorities, resources, coverage, and repetition.
4. Preserve probe capacity across command families.
5. Probe candidates through the scratch command entry (`apply_player_command_scratch`) on cloned state.
6. Select a viable substantive command; notification acknowledgement is fallback housekeeping.
7. Commit through `apply_player_command`.
8. Advance the action branch (counterfactual branches use the same day loop on disposable clones).
9. Advance a no-action baseline from the same decision point.
10. Record outcomes, attribution, scores, findings, and bounded trace data.

The harness must not directly mutate domain records during play. Agent spending policies may reserve cash for known obligations and may rank recovery ahead of discretionary growth; those policies must still use canonical commands and quotes.

## Progression and phases

The harness records durable milestones: commercial standing, institutional support, office campaigns, officeholding, city-shaping actions, labor conflict, and succession. City-shaping means exercising authority or committing a dynasty-sponsored civic project — enacting a law, starting a public work, or issuing an active office directive. Funding another sponsor's unfinished work is patronage and does not start dynastic governance by itself.

Decision cycles are grouped into product phases:

| Phase | Begins when |
|---|---|
| Foundation | Campaign start |
| Establishment | Reputation standing is established |
| Institutional ascent | Full commercial standing is established |
| Dynastic governance | The dynasty commits a city-shaping law, public work, or active office directive |
| Succession and legacy | A governing dynasty — one that has already shaped the city — completes its first succession |

The phase ladder follows durable milestones rather than capping at first succession. A house whose founder dies before shaping the city is still climbing the ascent arc under its heir, so its post-succession cycles read as institutional ascent; only a governing dynasty after succession enters the legacy era.

Phase diagnostics evaluate actionability, quiet/blocked time, choice breadth, consequence differentiation, strategic diversity, civic endpoints, recovery pressure, and post-succession continuity. Exact thresholds live in `src/gameplay/` and their tests.

## Counterfactual attribution

Routine simulation changes state without player action, so the harness advances two branches from each decision point:

| Branch | Operation |
|---|---|
| Action | Apply selected command, then advance time |
| Baseline | Apply no command, then advance the same time |

Changed domains are classified as:

| Class | Meaning |
|---|---|
| Immediate | Different at command commit |
| Persistent | Immediate difference remains at the attribution horizon |
| Delayed | New action-attributable difference appears after time advances |
| Ambient | Baseline changes without the command |

Only action-attributable differences create command-to-domain interaction edges.

## Quiet-cycle diagnosis

A no-action cycle happens when the agent has no viable substantive choice. Each quiet cycle resolves into one cause per command family:

| Cause | Meaning |
|---|---|
| Generator gap | An activation opportunity existed but no candidate was built, and the generator does not deliberately narrow that route |
| Agent restraint | An activation fired for a route the persona's standing policy narrows to strategic-need conditions; built by design only under need |
| Policy gate | Candidates were built but the persona's spending filters declined every one |
| Validation gate | Candidates were built and probed but canonical validation rejected every one |
| Dormant | No candidate was built and no activation opportunity fired |

Rules that keep the diagnosis honest:

- Every command kind has an activation predicate answering *would the canonical game accept some concrete command of this kind in this state?* Predicates mirror the game's own resource, cooldown, eligibility, capacity, and target gates and never encode the agent's portfolio or spending policy. An activation is therefore recorded even when no candidate is generated, so a quiet cycle is never misread as dormant just because a generator declined an offered action.
- This mirror is mechanically enforced: every decision cycle fails with `ActivationPredicateDrift` if a probe proves a command kind canonically viable while its predicate did not fire. Predicate drift cannot accumulate silently.
- Generators that deliberately narrow a broadly valid route — distress sales, wage-fairness cadence, succession-pressure designations, commission pacing, persona-relevant law sponsorship, discretionary floors for education, wards, endowments, and similar thresholds — classify an unfired activation as agent restraint rather than a generator gap. This keeps `generator_gaps` meaning "an offered action with no construction logic", so true coverage holes stay visible.
- Operational fallback actions (portfolio cash transfers, withdrawals) are context, not causes: an operational-only cycle carries that note in its `no_action_reason` on top of the classified strategic cause.

Diagnosis counts are recorded per campaign and summed in the aggregates. Each quiet trace step carries a human-readable `no_action_reason`, so a decision log explains why every gap happened.

## Report contract

`GAMEPLAY_REPORT_SCHEMA_VERSION` in `src/gameplay/` versions the structured report. The report preserves enough seed, persona, background, phase, entity, and trace context to reproduce a material finding.

### Contents

- Run configuration and schema version
- Aggregate and persona-level metrics
- Per-campaign endpoints, milestone timing, and phase-level activity, choice, and quiet-cause metrics
- Command generation, viability, selection, and consequence statistics
- Immediate, persistent, delayed, and ambient domain attribution
- Economic, civic, family, institutional, legal, crisis, and information snapshots
- Private-credit lifecycle counts including delinquent, defaulted, restructured, repaid, and written-off loans, with player lending and borrowing attribution
- Per-campaign commercial ledger: lifetime revenue, lifetime costs, implied margin, business cash
- Per-campaign affordability observations: `peak_player_treasury` and `minimum_unowned_property_value`, so an unexercised purchase route reads as an affordability ceiling rather than a declined choice
- Per-campaign rival context: every house's wealth, legitimacy, offices, and operating firms; the player's treasury and legitimacy ranks; a leaderboard of strongest houses
- Aggregate world-stress observations: city-wide attributed breach contracts, cumulative legal filings, peak route disruption, peak distressed-firm counts
- Per-campaign `player_breach_victim_contracts`, making counterparty wrongdoing that could ground a court claim visible even when the agent declines to litigate
- Representative decision traces and chronological decision logs
- Findings and stated limitations

Milestone days prefer the exact event day recorded in the chronicle over the coarser observation day: decision windows can straddle a year boundary, so a succession observed at day 367 may have executed at day 360.

Unexecuted command routes aggregate into three summary findings by cause: activations with no candidate construction (Critical for routes that are not deliberate-restraint routes, Warning for those that are) and kinds the world never offered (Info). Per-kind findings remain for more specific conditions — candidates never probed, always rejected, or viable but never selected.

### Decision traces

Each retained trace step records its phase and three measured consequence profiles: immediate changes at commit, changes attributable to the selected command at the horizon versus a no-action branch, and ambient changes from that branch. Feedback groups state their coverage (`simulation_window_days`, `ambient_window_days`): substantive cycles attribute over the consequence horizon; quiet cycles never branch, so both windows equal the ordinary advance.

Traces also retain bounded outbox and chronicle feedback events, so a transition can be read through the durable explanation the game produced rather than only through checksums.

Viable alternatives carry `projected_horizon_days` and compare over a shared horizon of three decision intervals bounded by `max_consequence_horizon_days`; the human log renders top alternatives with projected measures so a decision explains the tradeoff, not just the selection. Rendered alternatives deduplicate identical projected outcomes, and quiet-reason lists cap with a remainder count; the structured report keeps full lists.

Decision-log context lines carry the dynasty's power position: treasury, business cash, offices, legitimacy, generation, and player-facing legal exposure.

### Operational routes and pacing

Portfolio cash transfers and business-cash withdrawals are observable report routes excluded from substantive-action and strategic-streak metrics; a finding reports when they dominate agent activity. `ExtendCredit` statistics separately count new advances that changed business state immediately, new advances that stayed treasury-only, and zero-principal workouts of existing defaults. Workouts are recovery actions rather than fresh financing and are excluded from the productive-financing ratio.

The normal observation cadence is 30 days, shortened to seven days while an uncontained crisis or player labor dispute is active, and narrowed toward an underfunded legal case's hearing — urgent player-facing problems get recognized before the next ordinary decision.

## Scores

Scores range from 0 to 100:

| Score | Measures |
|---|---|
| Actionability | Whether substantive candidates pass canonical validation |
| Variety | Command coverage, distribution, choice breadth, and consequence diversity |
| Interconnection | Distinct command-to-system consequence edges |
| Feedback | Observable feedback combined with material consequence attribution |
| Resilience | Business continuity, liquidity, credit/labor/crisis pressure, food access, and civic conditions |
| Overall | Weighted summary of component scores |

Use component scores and findings for diagnosis. Overall score is a gate, not a complete design verdict.

## Findings

Findings use `Info`, `Warning`, or `Critical` severity. They identify conditions such as:

- Command families that are unreachable, persistently blocked, unselected, or inconsequential
- Repetitive action patterns or housekeeping displacement
- Excessive quiet/blocked periods, including campaign outliers hidden by averages
- Missing recovery routes or ineffective recovery churn
- Persistent business, labor, credit, crisis, food, or civic distress
- Strategically narrow mature play
- Convergent personas, backgrounds, properties, institutions, or civic strategies
- Weak political progression or office utility
- Succession without meaningful family, institutional, or strategic disruption
- Excessive mature liquidity or starting-background imbalance
- A property market priced out of reach: campaigns whose peak treasury never reached the cheapest unowned property never had the option to buy — an income-or-pricing signal, not agent restraint
- Crisis kinds no campaign ever detected: dead detection content rather than rare drama
- Counterparty performance that never fails: breach penalties, grounded claims, settlements, and seizure drama are unreachable without an originating grievance

Finding-rule unit tests arrange the minimum report fields needed to test the rule; they do not run long simulations to obtain a mutable template.

## Integration checklist

A new or changed player command is harness-integrated when all applicable items are updated:

1. Candidate generation and viability logic
2. `GameplayCommandKind` classification
3. Persona ranking where relevant
4. Snapshot/checksum coverage for affected state
5. Immediate and delayed consequence attribution
6. Interaction-domain mapping
7. Trace rendering/context
8. Findings or score inputs when the command changes a measured design contract
9. Exhaustive command-family coverage tests
10. Report schema version if serialized structure or semantics change

## Interpretation limits

The harness is deterministic automated analysis. It cannot establish whether prose is clear, UI hierarchy is legible, choices feel fair, pacing feels emotionally satisfying, or a consequence is narratively convincing; those require human review.

Additional reading rules:

- Choice breadth measures options emitted by the configured persona policy, not every legal command a human could discover; cross-persona matrices are required before treating a narrow candidate set as a hard game-system ceiling.
- Counterfactual attribution detects only consequences represented by the report snapshot within the configured consequence horizon.
- Alternative-choice profiles prove preserved strategic state, not human valuation of the difference.
- Material civic endpoints are measured per district, but the harness cannot judge whether neighborhood differences are fair or legible to a human player.

Use the harness to locate reproducible systemic behavior. Use behavioral tests to prove contracts and human playtesting to judge experience quality.
