# Gameplay Harness

The gameplay harness runs deterministic player agents through the same command and simulation APIs used by the CLI. It is a design and regression instrument for command reachability, delayed consequences, strategic variety, resilience, and multi-generation progression.

It does not replace behavior tests or human playtesting.

## Run modes

Default matrix:

```bash
cargo run --release --locked -- playtest
```

The default runs all four personas across all three starting backgrounds for 1,080 days per campaign.

Focused iteration:

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

CI quality gate:

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
| `--decision-interval` | Days advanced after each player decision. | `7` |
| `--max-probes` | Maximum candidate commands validated per decision cycle. | `24` |
| `--consequence-horizon` | Maximum days used for delayed consequence attribution. | `360` |
| `--trace-limit` | Representative trace steps retained per campaign. | `40` |
| `--persona` | Repeatable persona filter. Omit for all. | All |
| `--background` | Repeatable starting-background filter. Omit for all. | All |
| `--json` | Emit the structured versioned report. | Human text |
| `--output` | Write the report to a file. | Standard output |
| `--minimum-overall` | Fail when overall score is lower. | Disabled |
| `--fail-on-critical` | Fail when any critical finding exists. | Disabled |

Use release mode for broad matrices. Candidate probing and counterfactual simulation clone campaign state and are intentionally more expensive than ordinary tests.

## Personas

The harness uses four deterministic priority models:

- `steward`: continuity, worker conditions, relief, administrative education, reform, and public-works offices.
- `entrepreneur`: operating policy, contracts, property, commercial education, credit, market-toll offices, and expansion.
- `power-broker`: social education, ward adoption, taxation, public works, debt enforcement, laws, courts, and governance.
- `opportunist`: leverage, commercially trained wards, crisis exploitation, acquisition, debt-enforcement offices, legal pressure, and replacement labor.

Personas are diagnostic policies, not claims about optimal play. Differences between them help detect whether the command surface supports distinct strategies.
For variant-sensitive families such as office campaigns and family education, convergence analysis compares the resulting institutional and capability identities rather than treating the shared command label alone as evidence of identical play.

## Decision cycle

Each cycle follows this path:

1. Read the current campaign state.
2. Generate concrete `PlayerCommand` candidates from visible state.
3. Rank candidates by urgency, persona priorities, coverage, resources, and repetition penalties.
4. Reserve probe capacity across offered command families.
5. Validate candidates through `apply_player_command` on cloned state.
6. Select the highest-ranked viable command.
7. Commit through the same canonical command API.
8. Advance time through `advance_days`.
9. Compare action and baseline branches.
10. Record scores, findings, and bounded trace data.

The harness does not directly mutate domain records during play.

## Candidate coverage

Candidate generation covers the exposed player command surface:

- Dynasty capitalization, business acquisition, cash transfer, and operating policy
- Input supply, output sale, borrowing, lending, and property acquisition
- Laws and public works authorized by the powers of offices actually held
- Legal cases, governance, ward adoption, family education, and persona-specific office nomination
- Crisis response, labor-dispute response, and notification acknowledgement

Political progression is intentionally staged. Commercial standing is recorded before political eligibility; office campaigns require both established reputation and 16 completed contract deliveries. After selection, an office's powers require 60 days of tenure before they can authorize laws or public works.

Dynastic growth is also player-directed. A commercially established house can spend legitimacy and treasury to adopt a trained ward every two years, up to the configured household limit. Wards are active council members and valid office nominees. Once per year, the dynasty can fund focused administration, commerce, social, or craft education for an active family member. The harness does not offer these mature-household actions until the commercial record exists.

When a new command is added, it is not considered harness-integrated until generation, classification, snapshots, attribution, findings, traces, and coverage tests are updated.

## Counterfactual attribution

Routine simulation changes many domains without player action. The harness separates those changes from command consequences by advancing two deterministic branches from the same decision point:

- Action branch: apply the selected command, then advance time.
- Baseline branch: apply no command, then advance the same time.

The report classifies differences as:

| Class | Meaning |
|---|---|
| Immediate | The command changed state at commit time. |
| Persistent | An immediate difference still exists at the attribution horizon. |
| Delayed | A new action-attributable difference appears after time advances. |
| Ambient | The baseline changed from the initial state without the command. |

Only immediate and delayed action-attributable differences create command-to-domain interaction edges. Ambient changes describe system activity but are not credited to the command.

The consequence horizon is bounded and command-sensitive. Slow civic, legal, financial, and institutional actions can be observed beyond the normal decision interval without allowing unbounded simulation cost.

## Scores

Scores range from 0 to 100.

| Score | Measures |
|---|---|
| Actionability | Whether offered substantive commands pass canonical validation. |
| Variety | Strategic-direction coverage, action distribution, and viable alternatives. |
| Interconnection | Distinct command-to-domain edges and consequence breadth. |
| Feedback | Whether actions produce observable immediate or durable delayed results. |
| Resilience | Business continuity, food access, treasury, labor pressure, and crisis load. |
| Overall | Weighted summary of the five component scores. |

Use component scores and findings for diagnosis. The overall score is suitable for a coarse CI threshold, not as a complete design verdict.

## Findings

Findings are emitted with informational, warning, or critical severity. They cover conditions such as:

- A command family is never offered, probed, viable, selected, or consequential.
- Substantive choices are repeatedly blocked by canonical validation.
- One command or direction dominates the action distribution.
- Repeated command streaks indicate routine micromanagement.
- Year-scale gaps pass without a substantive player decision.
- Player businesses become persistently distressed, insolvent, or non-operational.
- Food access, contracts, credit, labor, crises, or notifications become structurally unhealthy.
- Public works exceed execution capacity.
- Political or commercial milestones are unreachable, misordered, overly synchronized, or compressed into the same phase.
- Political growth stalls at the founding household because no ward is adopted.
- Personas and starts converge despite different priorities.
- Commercial power fails to produce relationships, information, office, or civic effects.
- Long campaigns fail to reach succession or stable second-generation play.

Absence is interpreted against the configured campaign horizon and whether the route was offered. Event-dependent commands are not treated as broken before their prerequisites exist.

## Traces

Each campaign retains a bounded deterministic trace containing:

- Simulation day
- Offered, probed, and viable candidate counts
- Distinct viable command families
- Top ranked candidates with scores and descriptions
- Selected command and canonical outcome
- Representative rejection categories
- Immediate, persistent, delayed, and ambient changed domains
- Durable feedback flags
- Core campaign milestone timing

The sampler retains opening, closing, and high-consequence decisions. Increasing `--trace-limit` changes retained diagnostics, not simulation behavior.

## Structured report contract

The JSON report has a top-level `schema_version`. The current version is listed in `STATUS.md`.

Consumers should reject unknown schema versions rather than infer compatibility. Increment the schema when fields or semantics change in a way that affects automated readers.

The report contains:

- Harness configuration
- Aggregate scores and counters
- Per-command statistics
- Command-to-domain interaction edges
- Per-campaign snapshots and milestone data
- Eligible officeholder and active-ward counts, family capability checksums, and longest substantive-action gaps
- Representative traces
- Findings and limitations

## Integration checklist

When changing a player command or major system, review:

- Candidate generation and affordability filtering
- Persona ranking
- Command-family and strategic-direction labels
- Activation tracking for event-dependent routes
- Snapshot coverage for affected records
- Immediate and delayed comparison logic
- Interaction-domain classification
- Score inputs and findings
- Trace rendering
- JSON schema version
- Deterministic reproduction tests

## Interpretation limits

The harness measures deterministic reachability and represented state consequences. It cannot establish:

- Interface comprehension
- Enjoyment or emotional investment
- Cognitive load
- Narrative quality
- Whether a human recognizes the best option
- Long-horizon plans that a deterministic local-priority persona would not formulate
- Consequences outside the configured horizon
- Consequences not represented in snapshots

Use human playtesting for those questions.
