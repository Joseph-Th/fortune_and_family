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
- `--consequence-horizon`: maximum days used by command-specific counterfactual consequence probes.
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

The probe budget first reserves the highest-ranked candidate from each offered command family, then uses any remaining slots on additional targets or templates. A large set of office, law, property, or public-work variants therefore cannot hide an otherwise viable command family merely by exhausting the global probe limit.

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

The consequence horizon is command-specific. Reactive actions use the normal decision interval,
while policies, contracts, finance, property, law, public works, legal cases, governance, and office
nominations are probed over longer bounded horizons. This avoids declaring a slow system inert
merely because it cannot resolve inside one week.

The report classifies changes as:

- Immediate: state changed at command commit time.
- Persistent: an immediate change still differs from baseline at the consequence horizon.
- Delayed: a new action-attributable domain change appears after commit time.
- Ambient: the no-action branch changed from the initial state.

Only immediate and delayed changes create command-to-domain interaction edges. Ambient changes still count toward general system activity and appear in traces.

This is a deterministic intervention test. It does not prove philosophical causation, but it prevents routine weekly settlements, market movement, and AI activity from being misattributed to unrelated player choices.

## Report measures

The human report is compact enough for CI logs. The JSON report retains all campaign statistics and representative traces. It includes a top-level `schema_version` so automated consumers can reject incompatible report contracts explicitly. Schema version 9 records how many decision cycles offered each command family and how many cycles actually contained the external trigger required by legal, crisis, or labor responses. It separates persistent consequences from newly delayed consequences, tracks building and peak unfinished public works, separates player-involved contract outcomes from ambient city contracts, records available offices and the identities of offices, laws, governance, and sponsored works, and measures minimum food satisfaction only after simulation begins so the authored starting value cannot mask trajectory changes. It also separates inter-dynasty relationships, earned intelligence reports, and notification feedback into distinct domains. Trigger-aware reachability prevents long horizons from classifying a reactive route as broken when no grievance, crisis, or dispute occurred, while still making an unhandled trigger a release-gate failure.

### Actionability

The share of cycles with at least one substantive candidate in which a substantive command passes
canonical validation. Cycles where no substantive route is offered because systems are on deliberate
cooldowns are recorded as quiet rather than inaccessible. Cycles with substantive candidates but no
viable action are recorded as blocked. Notification acknowledgement is excluded from this measure.

### Variety

Combines command-family coverage, action distribution, and distinct viable command families per
decision. Multiple templates or targets from one command family no longer masquerade as broad
strategic choice.

### Interconnection

Measures distinct causal command-to-domain edges and average causal consequence breadth. The target
is several meaningful system links per command family, not an unrealistic expectation that every
command alter every domain. It uses the counterfactual action branch comparison, not ambient
simulation changes.

Commercial contracts, credit, repayment, default, and legal conflict are also measured as relationship changes. These links matter because the central game loop requires productive and commercial power to become social or institutional leverage rather than remaining isolated financial activity.

### Feedback

The share of substantive executed actions that produce a direct state/projection change or genuinely
delayed durable feedback through the outbox or chronicle. Persistent immediate messages are not
counted twice.

### Resilience

A coarse outcome and trajectory signal based on operating businesses, food satisfaction, player
treasury, escalated crises, minimum operating businesses, minimum food access, peak labor disputes,
and peak crisis load. Distressed businesses remain operational but receive a lower score than
healthy ones. This is not a win score; it exists to identify play styles or starts that collapse or
recover only after a prolonged dead period.

### Overall

A weighted summary of actionability, variety, interconnection, feedback, and resilience. The component scores and findings are more informative than the single number.

## Findings

The harness emits explicit findings for conditions such as:

- A command family is never offered during the configured horizon, is offered but never probed, is always rejected, or is viable but never selected.
- Candidates exist but canonical validation always rejects them.
- A viable command is never selected by any configured persona.
- A command executes without an observed domain consequence.
- One action dominates the decision distribution.
- A domain remains static, or changes autonomously without attributable player influence.
- Entire player portfolios frequently end distressed, insolvent, or non-operational.
- Any campaign falls below 10% food satisfaction, food access collapses broadly, contracts breach more often than they complete, credit defaults outnumber
  repayments, or labor disputes dominate active employment.
- Notification volume becomes unusable or crisis responses crowd out strategic play.
- Public works accumulate faster than the civic treasury can complete them.
- Raw choice counts hide a low number of distinct viable command families.
- Experience scores vary sharply by background, persona, or seed.
- Distinct personas converge on the same most-used action families.
- Civic outcomes converge on the same law mix, sponsored works, offices, and governance despite different strategies.
- Commercial actions do not create relationship or institutional leverage, or institutional actions do not reshape material conditions.
- Intelligence changes only through autonomous reports rather than player-directed commercial activity.
- Long campaigns do not exercise succession, or funded businesses suffer long-run condition collapse.

Command and domain absence is evaluated against days per campaign, not total days across the matrix.
Event-dependent systems remain informational until their normal activation horizon has elapsed.

The command table separates cycles in which a family was offered, generated variants, probed variants, viable variants, selected actions, persistent consequences, and newly delayed consequences. Campaign and aggregate records also separate quiet cycles from blocked cycles. This distinguishes a command that simply did not become relevant in a short run, or a deliberate waiting period, from a probe-budget, validation, ranking, or consequence-attribution problem.

Rejection counts are retained because blocked choices are part of the player experience. Repeated insufficient-funds failures, unchanged-policy requests, or inaccessible targets can indicate pacing, discoverability, or candidate-quality problems.

## Traces

Each campaign retains a bounded deterministic trace. It includes:

- Simulation day.
- Candidate and viable-choice counts.
- Distinct viable command families.
- Selected command and canonical command outcome.
- Representative rejection categories.
- Immediate, persistent, delayed, and ambient changed domains.
- Immediate, delayed, and ambient durable feedback flags.

The trace sampler keeps opening and closing decisions plus the highest-consequence decisions. Increasing `--trace-limit` preserves more detail without changing simulation behavior.

## Performance characteristics

The main costs are candidate probing and consequence attribution because both clone campaign state.
`--max-probes` bounds validation work and `--consequence-horizon` bounds long-term intervention
simulation. Long-horizon governance and office analysis is intentionally more expensive than a
weekly smoke test.

Use a release build for broad or multi-seed analysis. Focused debug runs remain useful while editing candidate policies and assertions.

## Interpreting failures

A harness finding is evidence, not an automatic design verdict.

- A command not exercised in a short run is informational when it was never offered. Event-dependent actions should become warnings or critical findings only after the world exposes their prerequisites or generated candidates fail validation or selection.
- High rejection rates may reveal meaningful constraints, poor pacing, or low-quality candidate generation.
- Business distress may be intended pressure, but insolvency across most personas and starts usually
  indicates weak recovery tools or a harsh opening economy.
- Low interconnection may mean consequences are absent, delayed beyond the run horizon, or not represented in the snapshot model.

When adding a new player command or major system, update command generation, domain snapshots, causal comparisons, findings, and coverage tests together.

## What the harness cannot establish

The report includes these limitations explicitly. Automated agents measure command reachability, deterministic consequences, and simulated outcomes. They do not establish whether a human understands the interface, finds the choices enjoyable, becomes emotionally invested in the family, or can compare options without excessive cognitive burden. Counterfactual attribution is also limited to fields represented in the snapshot and to the configured consequence horizon. Human playtesting remains necessary for those questions.
