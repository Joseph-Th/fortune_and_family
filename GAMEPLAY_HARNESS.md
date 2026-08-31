# Gameplay Harness

Runs deterministic player agents through the same campaign construction, command, and simulation APIs used by the product. Evaluates whether systems produce reachable, varied, consequential, recoverable, and multi-generation play.

Complements behavioral tests and human playtesting; replaces neither.

## Source of truth

- Implementation and report schema: `src/gameplay/`
- Behavioral coverage: `src/gameplay_tests.rs`
- Test tiers: `TESTING.md`
- Product goals: `DESIGN.md`

This document owns harness mechanics, report semantics, and integration requirements. Ranking constants and thresholds that do not define the external report contract are authoritative in code.

## When to use it

Run analysis when a change affects:

- Candidate discoverability or command validation
- Pacing, cooldowns, or eligibility
- Delayed or persistent consequences
- Cross-domain interaction
- Recovery and failure states
- Political progression or succession
- Report fields, scores, findings, or traces

Use focused runs during implementation. Use release matrices for cross-domain validation and design review.

## Commands

Focused debug iteration (incremental, <1s warm — no release build):

```bash
bash scripts/test.sh playtest --days 90 --persona entrepreneur --background baker
cargo run --locked -- playtest --days 90 --persona steward
# Release-fidelity iteration (same candidates, optimized sim):
CIVIC_DYNASTY_PROFILE=release bash scripts/test.sh playtest --days 360 --persona entrepreneur
cargo run --release --locked -- playtest --days 360 --persona entrepreneur --trace-limit 20
```

Full matrix and structured output (release, one build reused):

```bash
cargo run --release --locked -- playtest
cargo run --release --locked -- playtest --start-seed 1 --seeds 10 --days 1080 --json --output gameplay-report.json
```

Repository gates (always release, one CLI build — warm budgets after first cold release ~56s):

```bash
bash scripts/test.sh gameplay        # ~16s warm: 36 + 3 campaigns, 60k days
bash scripts/test.sh gameplay-audit  # ~30s warm: multi-seed / generation / credit-stress matrices
```

`playtest` defaults to a debug CLI for fast iteration; `gameplay`/`gameplay-audit` always use release. Every run prints one progress line to stderr (elapsed, campaigns, simulated days, actions, overall score, findings, days/s) so you can see at a glance whether a change moved the metric. Quality-gate failures report the exact score reason.

## Configuration

`GameplayHarnessConfig::default` (`src/gameplay/`) is authoritative.

| Option | Meaning | Default |
|---|---|---|
| `--start-seed` | First deterministic seed; omit to rotate by UTC day | Daily rotation |
| `--seeds` | Consecutive seeds | `3` |
| `--days` | Simulated days per campaign | `1080` |
| `--decision-interval` | Days advanced after a decision | `30` |
| `--max-probes` | Candidate commands validated per decision | `16` |
| `--consequence-horizon` | Delayed-attribution horizon (days) | `360` |
| `--trace-limit` | Retained trace steps per campaign | `40` |
| `--decision-log` | Campaigns whose full trace renders in the human report; `0` disables | `3` |
| `--persona` / `--background` | Repeatable filters; omit for all | All |
| `--json` / `--output` | Structured report to file or stdout | Human text |
| `--minimum-overall` | Fail below overall score | Disabled |
| `--fail-on-critical` | Fail on a critical finding | Disabled |

The decision interval is an observation cadence, not a player action limit.

### Parallelism and world sampling

Independent campaigns run in parallel, capped by `CIVIC_DYNASTY_JOBS` when set. Each campaign owns its `AppState`; the `Registry` is immutable, so scheduling does not affect results. Report ordering is fixed by seed, background, and persona. Single-campaign matrices probe counterfactuals on a bounded worker set. The same `CIVIC_DYNASTY_JOBS` caps `cargo --jobs` in `scripts/test.sh`.

The default matrix samples three independent world seeds. Personas share a world when the seed is fixed, so world-content claims (crisis variety, failure rates, civic drift) require multiple worlds. Agent-choice claims (persona variety, command coverage) aggregate across all campaigns.

The CLI rotates its default seed base deterministically by UTC calendar day (printed as `world seed base`), so scheduled runs sample fresh worlds while remaining reproducible from the report. Pass `--start-seed` to pin a world. Library `GameplayHarnessConfig::default` remains fully deterministic.

## Personas

Deterministic diagnostic policies, not optimal strategies.

| Persona | Bias |
|---|---|
| `steward` | Continuity, labor conditions, relief, administration, reform, public works |
| `entrepreneur` | Business policy, contracts, property, education, intelligence, credit, expansion |
| `power-broker` | Family capacity, institutions, intelligence, laws, courts, governance |
| `opportunist` | Leverage, acquisition, higher-risk credit, legal pressure, crisis exploitation, replacement labor |

Personas expose different viable routes through the same canonical systems. Candidate generation may rank per persona, but must not create domain state or bypass canonical validation.

Standing policies across personas:

- Candidate scores include deterministic exploration variation (range 520, decision interval jitter ±12 days) derived from campaign state and persona. Timing jitter mixes persona and generation/crisis state so campaigns sharing a world seed sample distinct calendars; score variation mixes generation, business, and crisis state besides the RNG. Both flip close calls without overriding urgency, reserve protection, or persona priorities.
- Optional standing expenses (education, wards, patronage) respect a discretionary floor: emergency reserve plus two months of committed loan service. Below the floor, standing spending is deferred.
- Standing-burning responses (suppression, profiteering) additionally respect a legitimacy reserve, matching treasury reserve policy.

## Decision cycle

Each cycle:

1. Capture current state.
2. Generate `PlayerCommand` candidates from state.
3. Rank by urgency, persona priorities, resources, coverage, repetition.
4. Preserve probe capacity across command families.
5. Probe via `apply_player_command_scratch` on clones.
6. Select a viable substantive command; notification acknowledgement is fallback housekeeping.
7. Commit through `apply_player_command`.
8. Advance the action branch; counterfactual branches use the same day loop on disposable clones.
9. Advance a no-action baseline from the same decision point.
10. Record outcomes, attribution, scores, findings, and trace data.

The harness never directly mutates domain records. Spending policies may reserve cash for known obligations and rank recovery ahead of growth, but must still use canonical commands and quotes.

## Progression and phases

The harness records durable milestones: commercial standing, institutional support, office campaigns, officeholding, city-shaping actions, labor conflict, and succession. City-shaping means exercising authority or committing a dynasty-sponsored civic project — enacting a law, starting a public work, or issuing an active office directive. Funding another sponsor's unfinished work is patronage, not city-shaping.

| Phase | Begins when |
|---|---|
| Foundation | Campaign start |
| Establishment | Reputation standing established |
| Institutional ascent | Full commercial standing established |
| Dynastic governance | Dynasty commits a city-shaping law, public work, or active directive |
| Succession and legacy | A governing dynasty (already shaped the city) completes its first succession |

A house whose founder dies before city-shaping remains on the ascent arc under its heir; only a governing dynasty after succession enters the legacy era.

Phase diagnostics evaluate actionability, quiet/blocked time, choice breadth, consequence differentiation, strategic diversity, civic endpoints, recovery pressure, and post-succession continuity. Exact thresholds live in `src/gameplay/` and its tests.

## Counterfactual attribution

Two branches from each decision point:

| Branch | Operation |
|---|---|
| Action | Apply selected command, then advance time |
| Baseline | Apply no command, then advance the same time |

| Class | Meaning |
|---|---|
| Immediate | Different at commit |
| Persistent | Immediate difference remains at the horizon |
| Delayed | New action-attributable difference appears after time advances |
| Ambient | Baseline changes without the command |

Only action-attributable differences create command-to-domain interaction edges.

## Quiet-cycle diagnosis

A quiet cycle has no viable substantive choice. Each quiet cycle resolves to one cause per command family:

| Cause | Meaning |
|---|---|
| Generator gap | Activation existed but no candidate was built, and the generator does not deliberately narrow that route |
| Agent restraint | Activation fired for a route the persona narrows to strategic-need conditions by design |
| Policy gate | Candidates built but persona spending filters declined every one |
| Validation gate | Candidates built and probed but canonical validation rejected every one |
| Dormant | No candidate built and no activation fired |

Rules:

- Every command kind has an activation predicate: *would the canonical game accept some concrete command of this kind in this state?* Predicates mirror resource, cooldown, eligibility, capacity, and target gates. An activation is recorded even when no candidate is generated, so quiet cycles are never misread as dormant because a generator declined an offer.
- Mechanically enforced: a viable probe whose kind did not fire an activation fails the cycle with `ActivationPredicateDrift`.
- Deliberately narrowed routes (distress sales, wage-fairness cadence, succession-pressure designations, commission pacing, discretionary floors) classify an unfired activation as agent restraint, not generator gap. This keeps `generator_gaps` meaning "offered action with no construction logic."
- Operational fallbacks (cash transfers, withdrawals) are context, not causes. An operational-only cycle notes that atop the strategic cause.

Counts are recorded per campaign and summed in aggregates. Each quiet trace step carries a human-readable `no_action_reason`.

## Report contract

`GAMEPLAY_REPORT_SCHEMA_VERSION` (`src/gameplay/`) versions the structured report. The report preserves enough context to reproduce any material finding.

### Contents

- Run configuration and schema version
- Aggregate and persona-level metrics
- Per-campaign endpoints, milestone timing, phase-level activity/choice/quiet-cause metrics
- Command generation, viability, selection, and consequence statistics
- Immediate, persistent, delayed, and ambient domain attribution
- Economic, civic, family, institutional, legal, crisis, and information snapshots
- Private-credit lifecycle counts (delinquent, defaulted, restructured, repaid, written-off) with lending/borrowing attribution
- Per-campaign commercial ledger: lifetime revenue, costs, margin, business cash
- Per-campaign affordability: `peak_player_treasury` and `minimum_unowned_property_value`
- Per-campaign rival context: house wealth, legitimacy, offices, operating firms; player treasury/legitimacy ranks; leaderboard
- Aggregate world stress: city-wide breach contracts, cumulative legal filings, peak route disruption, peak distressed-firm counts
- Per-campaign `player_breach_victim_contracts`
- Representative traces and chronological decision logs
- Findings and stated limitations

Milestone days prefer the exact chronicle day over the observation day (decision windows can straddle a year boundary).

Unexecuted routes aggregate into three summary findings by cause: activations with no candidate construction (Critical for non-restraint routes, Warning for deliberate-restraint routes) and kinds the world never offered (Info). Per-kind findings remain for more specific conditions.

### Decision traces

Each step records its phase and three consequence profiles: immediate changes at commit, action-attributable changes at the horizon vs the baseline, and ambient baseline changes. Feedback groups state coverage (`simulation_window_days`, `ambient_window_days`): substantive cycles attribute over the horizon; quiet cycles use the ordinary advance for both.

Traces retain bounded outbox and chronicle feedback events.

Alternatives carry `projected_horizon_days` and compare over a shared horizon of three decision intervals bounded by `max_consequence_horizon_days`. The human log deduplicates identical projected outcomes and caps quiet-reason lists with a remainder count; the structured report keeps full lists.

Context lines carry treasury, business cash, offices, legitimacy, generation, and legal exposure.

### Operational routes and pacing

Portfolio cash transfers and withdrawals are observable but excluded from substantive-action and strategic-streak metrics; a finding reports when they dominate. `ExtendCredit` counts separately: new advances that immediately changed business state, new advances that stayed treasury-only, and zero-principal workouts of existing defaults. Workouts are recovery actions excluded from the productive-financing ratio.

The normal cadence is 30 days, shortened to seven days while an uncontained crisis or player labor dispute is active, and narrowed toward an underfunded legal hearing.

## Scores

Range 0–100:

| Score | Measures |
|---|---|
| Actionability | Whether substantive candidates pass validation |
| Variety | Coverage, distribution, choice breadth, consequence diversity |
| Interconnection | Distinct command-to-system edges |
| Feedback | Observable feedback combined with material attribution |
| Resilience | Business continuity, liquidity, credit/labor/crisis pressure, food access, civic conditions |
| Overall | Weighted summary |

Component scores and findings diagnose; overall score is a gate.

## Findings

Severity `Info`, `Warning`, `Critical`. Identify conditions such as:

- Unreachable, blocked, unselected, or inconsequential command families
- Repetitive patterns or housekeeping displacement
- Excessive quiet/blocked periods, including outliers hidden by averages
- Missing recovery routes or ineffective churn
- Persistent business, labor, credit, crisis, food, or civic distress
- Narrow mature play
- Convergent personas, backgrounds, properties, institutions, or civic strategies
- Weak political progression or office utility
- Succession without meaningful disruption
- Excessive mature liquidity or background imbalance
- Property market priced out of reach
- Crisis kinds never detected
- Counterparty performance that never fails

Finding-rule tests arrange minimal report fields needed to test the rule, not long simulations.

## Integration checklist

A new or changed command is harness-integrated when all applicable items are updated:

1. Candidate generation and viability logic
2. `GameplayCommandKind` classification
3. Persona ranking where relevant
4. Snapshot/checksum coverage for affected state
5. Immediate and delayed consequence attribution
6. Interaction-domain mapping
7. Trace rendering/context
8. Findings or score inputs when the measured contract changes
9. Exhaustive command-family coverage tests
10. Report schema version if serialized structure or semantics change

## Robustness and staleness contract

Divergence between harness and game is a harness defect.

- Activation predicates are checked every cycle (`ActivationPredicateDrift` on mismatch).
- Report schema version bumps on any semantic change (activation, scoring, findings, trace meaning).
- Organic variation (jitter ±12 days plus score range 520, each persona- and state-aware) prevents rigid replay while urgency and persona priorities remain dominant.
- Bounded work: probe caps, horizons, trace limits bound every run in domain terms. Parallelism capped by `CIVIC_DYNASTY_JOBS`; ordering is stable.

## Interpretation limits

Deterministic automated analysis cannot establish whether prose is clear, UI hierarchy is legible, choices feel fair, pacing feels satisfying, or consequences feel narratively convincing. Human review is required for those.

Reading rules:

- Choice breadth measures options from the configured persona policy, not every legal command a human could find. Cross-persona matrices are required before treating a narrow set as a hard ceiling.
- Attribution detects only consequences in the report snapshot within the configured horizon.
- Alternative profiles prove preserved strategic state, not human valuation of differences.
- Civic endpoints are measured per district; the harness cannot judge neighborhood fairness or legibility.

Use the harness to locate reproducible systemic behavior. Use behavioral tests to prove contracts and human playtesting to judge experience.
