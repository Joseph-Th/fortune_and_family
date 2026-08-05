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
| `opportunist` | Leverage, acquisition, counterparty intelligence, legal pressure, crisis exploitation, and replacement labor. |

Personas are diagnostic policies, not optimal strategies. Their purpose is to expose whether the command surface supports distinct priorities and outcomes.

## Decision cycle

Each decision cycle:

1. Captures the current campaign state.
2. Generates concrete `PlayerCommand` candidates from state.
3. Ranks candidates by urgency, persona priorities, coverage, resources, and repetition.
4. Preserves probe capacity across command families.
5. Validates candidates through `apply_player_command` on cloned state.
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

Institutional patronage becomes available after full commercial standing: established reputation and 104 credited contract deliveries. Patronage transfers treasury into an institution, creates membership, improves support among member houses, and must mature for 90 days. Office nomination then requires established support in the target institution. Office powers become available after a separate 180-day establishment period. Officeholders then face recurring duties, administrative load, and possible forfeiture.

Family growth remains player-directed through governance, ward adoption, and focused education. Commercial maturity gates the advanced family routes.

Established dynasties may also commission annual intelligence reports. The focus is persona-specific in automated runs: markets for entrepreneurs, district conditions for stewards, and rival-house conditions for political or opportunistic agents.

## Phase quality

The harness classifies each decision cycle by the furthest reached fantasy milestone:

| Phase | Begins when |
|---|---|
| Foundation | Campaign start. |
| Establishment | Reputation standing is reached. |
| Institutional ascent | The first institutional support campaign is launched. |
| Dynastic governance | The first city-shaping law or public work is sponsored. |

Each phase records action share, quiet and blocked cycles, viable command-family breadth, and multi-family choice frequency. Findings use these counters so a strong aggregate score cannot hide a passive establishment period or a strategically narrow governance phase.

## Recovery routes

The harness must be able to discover and evaluate canonical recovery actions, including:

- Business recapitalization and internal cash transfer
- New credit when available
- Delayed restructuring of defaulted credit
- Voluntary property liquidation
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
| Variety | Command-direction coverage, distribution, and viable alternatives. |
| Interconnection | Distinct command-to-domain edges and consequence breadth. |
| Feedback | Observable immediate and durable delayed results. |
| Resilience | Business continuity, food access, liquidity, labor pressure, and crisis load. |
| Overall | Weighted summary of the component scores. |

Use component scores and findings for diagnosis. The overall score is a coarse gate, not a complete design verdict.

## Findings

Findings use `Info`, `Warning`, or `Critical` severity. They cover conditions such as:

- Command families that are absent, blocked, unselected, or inconsequential
- Repetitive command streaks and housekeeping displacement
- Long periods without a substantive action
- Economic states with no viable recovery route
- Persistent business, food, labor, credit, crisis, or notification failure
- Public-work or office-duty overload
- Misordered, compressed, synchronized, or unreachable campaign milestones
- Weak strategic variety or persona convergence
- Political growth without family capacity
- Long campaigns that do not reach stable succession

Absence is interpreted against the configured horizon and prerequisite availability. Event-driven commands are not treated as broken before their trigger exists.

## Trace contract

Each retained step includes:

- Simulation day
- Compact economic, financial, political, and family context
- Considered, viable, and substantive candidate counts
- Distinct viable command families
- Ranked candidates and scores
- Selected command and outcome
- Rejection summary
- Immediate, persistent, delayed, and ambient domains
- World-feedback flags

Context includes treasury, business cash and condition, deliveries, loan states, property and collateral, reputation, legitimacy, offices, laws, public works, wards, family unity, generation, labor disputes, crises, and notifications.

The trace sampler retains representative opening, closing, and high-consequence decisions. `--trace-limit` affects diagnostics only.

## Structured report

The JSON report contains:

- `schema_version`
- Harness configuration
- Aggregate counters, scores, command statistics, and interaction edges
- Per-phase actionability and command-family breadth
- Per-campaign start and end snapshots
- Fantasy-arc milestones
- Findings and limitations
- Bounded traces

The current schema version is listed in `STATUS.md`. Consumers should reject unknown schema versions.

Increment the gameplay report schema when fields or semantics change for automated readers.

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
- Whether mechanically distinct options feel meaningfully distinct

Use human playtesting for those questions.
