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
| `--max-probes` | Maximum candidate commands validated per decision | `24` |
| `--consequence-horizon` | Maximum delayed-attribution horizon in days | `360` |
| `--trace-limit` | Representative trace steps retained per campaign | `40` |
| `--persona` | Repeatable persona filter; omit for all | All |
| `--background` | Repeatable background filter; omit for all | All |
| `--json` | Emit structured report | Human text |
| `--output` | Write report to a file | Standard output |
| `--minimum-overall` | Fail below overall score | Disabled |
| `--fail-on-critical` | Fail if a critical finding exists | Disabled |

The normal decision interval is an observation cadence, not a gameplay rule limiting the number of commands a human player could issue.

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

## Report contract

`GAMEPLAY_REPORT_SCHEMA_VERSION` in `src/gameplay.rs` versions the structured report.

Reports contain:

- Run configuration and schema version
- Aggregate and persona-level metrics
- Per-campaign endpoints and milestone timing
- Command generation, viability, selection, and consequence statistics
- Phase-level activity and choice metrics
- Immediate, persistent, delayed, and ambient domain attribution
- Economic, civic, family, institutional, legal, crisis, and information snapshots
- Representative decision traces
- Findings and stated limitations

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
