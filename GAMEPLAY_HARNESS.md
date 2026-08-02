# Gameplay Testing Harness

The gameplay harness runs deterministic player agents through the same public command and simulation pipelines used by the CLI. It is intended for design review, regression testing, balance investigation, and measuring whether systems produce understandable consequences.

## Running it

A broad default run covers every starting background and all four player personas for three simulated years:

```bash
cargo run --release --locked -- playtest
```

A focused iteration run:

```bash
cargo run --release --locked -- playtest \
  --days 360 \
  --persona entrepreneur \
  --background baker \
  --trace-limit 20
```

A multi-seed machine-readable report:

```bash
cargo run --release --locked -- playtest \
  --start-seed 1 \
  --seeds 10 \
  --days 1080 \
  --json \
  --output gameplay-report.json
```

Useful controls:

- `--days`: simulated days per campaign.
- `--decision-interval`: days advanced after each player decision. The default is seven.
- `--max-probes`: maximum candidate commands validated per decision cycle.
- `--trace-limit`: retained representative decisions per campaign.
- `--persona`: repeatable filter for steward, entrepreneur, power-broker, or opportunist.
- `--background`: repeatable filter for baker, cloth-trader, or blacksmith.
- `--seeds`: number of consecutive deterministic seeds.
- `--json`: emit the complete structured report.
- `--minimum-overall`: fail after writing the report when the overall score is below a CI threshold.
- `--fail-on-critical`: fail after writing the report when any critical finding is present.

## How agents play

At each decision cycle, the harness reads current campaign state and derives concrete player commands. It does not mutate internal records directly during play. Every candidate is tested through `apply_player_command` against a cloned state. The highest-ranked viable command is then committed through that same canonical API, followed by `advance_days` through the normal simulation pipeline.

Candidate generation covers all exposed player command families:

- Direct dynasty capitalization of stressed owned businesses, distressed-business acquisition and
  recapitalization, business cash transfers, and operating policies.
- Supply contracts, loans, and property purchases.
- Laws, public works, legal cases, governance, and office nominations.
- Crisis responses, labor-dispute responses, and notification acknowledgement.

Agents choose based on urgency, persona preferences, unexplored command coverage, current resources, and repetition penalties. The policy is deterministic. The same configuration and code produce the same report.

## Player personas

The harness uses deliberately different priorities rather than one allegedly optimal agent.

- Steward: business continuity, worker conditions, relief, reform, and public works.
- Entrepreneur: inventory policy, contracts, property, finance, and market expansion.
- Power broker: laws, offices, public works, courts, and house governance.
- Opportunist: crisis exploitation, leverage, property acquisition, legal pressure, and replacement labor.

Personas are diagnostic instruments. A command reachable only by one persona is weaker evidence of broad player accessibility than a command used across several personas.

## Counterfactual consequence attribution

Ordinary simulation activity changes many records every week. Crediting all of those changes to the preceding command would make every action appear highly interconnected. The harness therefore advances two deterministic branches from each decision point:

1. The action branch applies the selected player command and advances time.
2. The baseline branch applies no command and advances the same number of days.

The report classifies changes as:

- Immediate: state changed at command commit time.
- Delayed: the action branch differs from the no-action branch after simulation.
- Ambient: the no-action branch changed from the initial state.

Only immediate and delayed changes create command-to-domain interaction edges. Ambient changes still count toward general system activity and appear in traces.

This is a deterministic intervention test. It does not prove philosophical causation, but it prevents routine weekly settlements, market movement, and AI activity from being misattributed to unrelated player choices.

## Report measures

The human report is compact enough for CI logs. The JSON report retains all campaign statistics and representative traces. It includes a top-level `schema_version` so automated consumers can reject incompatible report contracts explicitly.

### Actionability

The share of decision cycles with at least one command that passes canonical validation. A low score means the player frequently has no legal or affordable action.

### Variety

Combines command-family coverage with action distribution. It penalizes runs where one command dominates despite broad nominal reachability.

### Interconnection

Measures distinct causal command-to-domain edges and average causal consequence breadth. It uses the counterfactual action branch comparison, not ambient simulation changes.

### Feedback

The share of executed actions that produce immediate or delayed durable feedback through the outbox or chronicle.

### Resilience

A coarse outcome signal based on operating businesses, food satisfaction, player treasury, and
escalated crises. Distressed businesses remain operational but receive a lower score than healthy
ones. This is not a win score; it exists to identify play styles or starts that systematically
collapse.

### Overall

A weighted summary of actionability, variety, interconnection, feedback, and resilience. The component scores and findings are more informative than the single number.

## Findings

The harness emits explicit findings for conditions such as:

- A command family has no reachable candidate.
- Candidates exist but canonical validation always rejects them.
- A viable command is never selected by any configured persona.
- A command executes without an observed domain consequence.
- One action dominates the decision distribution.
- A domain remains static for the entire run.
- Entire player portfolios frequently end distressed, insolvent, or non-operational.
- Experience scores vary sharply by background, persona, or seed.

The command table separates generated, probed, viable, selected, and rejected choices. This distinguishes a missing gameplay route from a probe-budget or ranking problem, and distinguishes both from canonical validation barriers.

Rejection counts are retained because blocked choices are part of the player experience. Repeated insufficient-funds failures, unchanged-policy requests, or inaccessible targets can indicate pacing, discoverability, or candidate-quality problems.

## Traces

Each campaign retains a bounded deterministic trace. It includes:

- Simulation day.
- Candidate and viable-choice counts.
- Selected command and canonical command outcome.
- Representative rejection categories.
- Immediate, delayed, and ambient changed domains.
- Immediate, delayed, and ambient durable feedback flags.

The trace sampler keeps opening and closing decisions plus the highest-consequence decisions. Increasing `--trace-limit` preserves more detail without changing simulation behavior.

## Performance characteristics

The main cost is candidate probing because each probe clones campaign state and invokes canonical validation. `--max-probes` bounds this cost. The no-action counterfactual doubles time advancement per decision cycle but substantially improves consequence attribution.

Use a release build for broad or multi-seed analysis. Focused debug runs remain useful while editing candidate policies and assertions.

## Interpreting failures

A harness finding is evidence, not an automatic design verdict.

- No candidate may mean the game truly lacks a route to that action, or the candidate generator does not understand the route yet.
- High rejection rates may reveal meaningful constraints, poor pacing, or low-quality candidate generation.
- Business distress may be intended pressure, but insolvency across most personas and starts usually
  indicates weak recovery tools or a harsh opening economy.
- Low interconnection may mean consequences are absent, delayed beyond the run horizon, or not represented in the snapshot model.

When adding a new player command or major system, update command generation, domain snapshots, causal comparisons, findings, and coverage tests together.
