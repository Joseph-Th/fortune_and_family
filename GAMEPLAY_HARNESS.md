# Gameplay Harness

The gameplay harness runs deterministic player agents through the same campaign construction, player-command, and simulation APIs used by the rest of the application. It evaluates whether the implemented systems produce reachable, varied, consequential, recoverable, and multi-generation play.

It complements behavioral tests and human playtesting. It does not replace either.

## Source of truth

- Implementation and report schema: `src/gameplay.rs`
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

Focused campaign:

```bash
cargo run --release --locked -- playtest \
  --days 360 \
  --persona entrepreneur \
  --background baker \
  --trace-limit 20
```

Default matrix:

```bash
cargo run --release --locked -- playtest
```

Structured report:

```bash
cargo run --release --locked -- playtest \
  --start-seed 1 \
  --seeds 10 \
  --days 1080 \
  --json \
  --output gameplay-report.json
```

Repository gates:

```bash
bash scripts/test.sh gameplay
bash scripts/test.sh gameplay-audit
```

`gameplay` runs the normal release quality and generation-length gates. `gameplay-audit` runs larger mature, generation, and credit-stress matrices for design review. `bash scripts/test.sh all` includes the normal release gameplay gates.

## Configuration

`GameplayHarnessConfig::default` in `src/gameplay.rs` is authoritative.

| Option | Meaning | Default |
|---|---|---|
| `--start-seed` | First deterministic seed | `1` |
| `--seeds` | Number of consecutive seeds | `1` |
| `--days` | Simulated days per campaign | `1080` |
| `--decision-interval` | Normal days advanced after a player decision | `30` |
| `--max-probes` | Maximum candidate commands validated per decision | `16` |
| `--consequence-horizon` | Maximum delayed-attribution horizon in days | `360` |
| `--trace-limit` | Representative trace steps retained per campaign | `40` |
| `--decision-log` | Campaigns whose full retained trace renders in the human report; `0` disables | `3` |
| `--persona` | Repeatable persona filter; omit for all | All |
| `--background` | Repeatable background filter; omit for all | All |
| `--json` | Emit structured report | Human text |
| `--output` | Write report to a file | Standard output |
| `--minimum-overall` | Fail below overall score | Disabled |
| `--fail-on-critical` | Fail if a critical finding exists | Disabled |

The normal decision interval is an observation cadence, not a gameplay rule limiting the number of commands a human player could issue.

Independent campaigns in a matrix run in parallel using the machine's available parallelism. Each campaign builds and advances its own `AppState` from the shared immutable registry, so parallelism never changes the state of another campaign. Report ordering is fixed by seed, background, and persona regardless of scheduling; a matrix with one campaign remains serial.

## Personas

Personas are deterministic diagnostic policies, not optimal strategies.

| Persona | Bias |
|---|---|
| `steward` | Continuity, labor conditions, relief, administration, reform, public works |
| `entrepreneur` | Business policy, contracts, property, education, market intelligence, credit, expansion |
| `power-broker` | Family capacity, institutions, intelligence, laws, courts, public works, governance |
| `opportunist` | Leverage, acquisition, higher-risk credit, legal pressure, crisis exploitation, replacement labor |

The personas should expose different viable routes through the same canonical systems. Candidate generation may use persona-specific ranking and risk tolerances, but it must not create domain state or bypass canonical validation.

## Decision cycle

Each decision cycle:

1. Capture the current campaign state.
2. Generate concrete `PlayerCommand` candidates from state.
3. Rank candidates by urgency, persona priorities, resources, coverage, and repetition.
4. Preserve probe capacity across command families.
5. Probe candidates through `apply_player_command` on cloned state.
6. Select a viable substantive command; notification acknowledgement is fallback housekeeping.
7. Commit through `apply_player_command`.
8. Advance the action branch through `advance_days`.
9. Advance a no-action baseline from the same decision point.
10. Record outcomes, attribution, scores, findings, and bounded trace data.

The harness must not directly mutate domain records during play.

Agent spending policies may reserve cash for known debt, office, family, or legal obligations and may rank recovery actions ahead of discretionary growth. Those policies are diagnostic behavior and must still use canonical commands and quotes.

## Progression and phases

The harness records durable milestones including commercial standing, institutional support, office campaigns, officeholding, city-shaping actions, labor conflict, and succession.

Decision cycles are grouped into product phases:

| Phase | Begins when |
|---|---|
| Foundation | Campaign start |
| Establishment | Reputation standing is established |
| Institutional ascent | Full commercial standing is established |
| Dynastic governance | The dynasty commits a city-shaping law, public work, or active office directive |
| Succession and legacy | The first succession completes |

Phase diagnostics evaluate actionability, quiet/blocked time, viable choice breadth, consequence differentiation, strategic diversity, civic endpoints, recovery pressure, and post-succession continuity. Exact thresholds belong in `src/gameplay.rs` and their tests.

## Counterfactual attribution

Routine simulation changes state without player action. The harness separates command consequences from ambient change by advancing two branches from the same decision point:

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

A no-action cycle happens when the agent has no viable substantive choice. The report distinguishes three causes per command family so design work does not conflate game gaps with agent discipline:

| Cause | Meaning |
|---|---|
| Generator gap | An activation opportunity existed but no candidate was built |
| Policy gate | Candidates were built but the persona's spending filters declined every one of them |
| Validation gate | Candidates were built and probed but canonical validation rejected every one of them |
| Dormant | No candidate was built and no activation opportunity fired; the game world offered no detected action |

Quiet cycles with no recorded cause are dormant state: the game world offered no detected opportunity, no candidate was built, and no activation predicate fired. Every quiet cycle either resolves into a cause or is logged as dormant, so no-action play is never silent by accident.

The diagnosis is recorded per campaign and summed in the aggregate and persona aggregates. Each trace step that took no substantive action also carries a human-readable `no_action_reason`, so a chronological decision log explains *why* each quiet gap happened.

Activation opportunities are recorded for every command kind. Commands with a dedicated world-state predicate (crisis, labor, legal, settlement, property liquidation, institution withdrawal, credit extension, and business-cash transfer) use that predicate; every other command kind records an activation whenever its generator built a candidate. This keeps the `triggers` column and the generator-gap diagnosis meaningful for all command families.

## Report contract

`GAMEPLAY_REPORT_SCHEMA_VERSION` in `src/gameplay.rs` versions the structured report.

Reports contain:

- Run configuration and schema version
- Aggregate and persona-level metrics
- Per-campaign endpoints and milestone timing
- Command generation, viability, selection, and consequence statistics
- Phase-level activity and choice metrics
- Immediate, persistent, delayed, and ambient domain attribution
- Quiet-cycle diagnosis separating generator gaps, agent-policy gates, validation gates, and dormant waiting
- Economic, civic, family, institutional, legal, crisis, and information snapshots
- Representative decision traces
- A chronological decision log for a configured number of campaigns, each retained step showing context, the selected command and outcome, and the reason no action was taken on quiet cycles
- Findings and stated limitations

Each retained trace step includes three measured consequence profiles: immediate
changes at command commit, changes attributable to the selected command at the
configured horizon versus a no-action branch, and ambient changes from that
no-action branch. This makes a trace answer both “what did the command do?” and
“what would have happened anyway?” with concrete before/after values, not only
domain labels. Portfolio cash transfers are retained as observable operational
actions but excluded from substantive-action and strategic-streak metrics; a
separate finding reports when they dominate the agent's activity.

The report should preserve enough seed, persona, background, phase, entity, and trace context to reproduce a material finding.

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
- Excessive quiet/blocked periods, including campaign outliers hidden by aggregate averages
- Missing recovery routes or ineffective recovery churn
- Persistent business, labor, credit, crisis, food, or civic distress
- Strategically narrow mature play
- Convergent personas, backgrounds, properties, institutions, or civic strategies
- Weak political progression or office utility
- Succession without meaningful family, institutional, or strategic disruption
- Excessive mature liquidity or starting-background imbalance

Finding-rule unit tests should arrange the minimum report fields needed to test the rule. They should not run a long simulation merely to obtain a mutable report template.

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

The harness is deterministic automated analysis. It cannot establish whether prose is clear, UI hierarchy is legible, choices feel fair, pacing feels emotionally satisfying, or a consequence is narratively convincing. Those require human review.

Use the harness to locate reproducible systemic behavior. Use behavioral tests to prove contracts and human playtesting to judge experience quality.
