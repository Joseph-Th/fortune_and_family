# Agent Guide

**BCA policy:** advisory


**Rust agent diagnostics:** advisory. Use bounded diagnostics only for unresolved ownership, test-strength, or macro questions; commands and verification routing: [TESTING.md](TESTING.md).

**Profiles:** Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation

Execution card for repository work. `ARCHITECTURE.md` owns structure and mutation contracts, `STATUS.md` owns capability and schemas, `TESTING.md` owns test policy, `DESIGN.md` owns intent, `GAMEPLAY_HARNESS.md` owns harness semantics. The embedded workspace contract owns shared execution and filesystem rules.

<!-- workspace-contract:begin (generated from ../AGENTS.md by tools/sync_agent_context.py; edit the source, not this copy) -->
## Workspace contract

Applies to every agent in every harness. Source, rationale, and evidence: [../AGENTS.md](../AGENTS.md).

The user is a solo developer. Precedence: current request → this contract → project card → standards. **Hard** rules survive lower-level workflow advice.

### AG-1 Finish the work

Finish the authorized task; plans, progress, or deferred notes are not delivery. On open-ended or hill-climbing goals, keep iterating—observe, diagnose, fix, verify—until the goal is met or in-scope improvements are exhausted; if one track is blocked, move to the next unblocked work in scope. Delegate only bounded read-only work when authorized.

### AG-2 Make the calls yourself

Make in-scope decisions yourself; state consequential choices briefly and continue. Do not ask permission for routine verification, bounded evaluations, or the next in-scope step; ask only when a choice changes the deliverable and cannot be inferred. Read the project card once, then resolve authority, owner, and proof. Expand only for unresolved scope or boundaries. Links and profiles are lookups; research and artifact work starts at the product workflow.

### AG-3 Write only in your project (hard)

Write only in the requested project. Bounded read-only inspection of maintained sibling projects is allowed when it materially helps prior-art discovery or comparison; it never authorizes edits there. Do not create top-level directories or write to the workspace root, project parent, or sibling projects. Put scratch work in the project's ignored output (normally `target/agent-output/`) or OS temp.

### AG-4 Done means the user's copy works (hard)

Verify every requirement against the user's actual copy. Refresh and check the affected release, deployment, or running app; source edits alone do not update it. Docs-only work verifies docs and routes. Use the smallest complete project lane and [`REVIEW.md`](../REVIEW.md) for consequential code. Claim only checked results; report what remains unverified.

### AG-5 Look at what you made

Run or render changed visual, audio, interactive, or published output; inspect it against [`STANDARDS_QUALITY.md`](../STANDARDS_QUALITY.md) and project references. Use isolated workbench or multi-angle views where detail matters, compare directly against references at matching angles and delivery scale, fix defects before reporting, and report rendered evidence. Internal prose needs only readability and route review. Tests do not prove appearance.

### AG-6 Real behavior over proxies

Observe user-experienced behavior. Tests, harnesses, validators, and metrics are evidence for hill climbing, not the goal. Use their findings to diagnose and fix systemic root causes in production owners; repair proxy/product divergence, and never weaken gates, hardcode outcomes, suppress warnings, or retry until green. Simulations stay causal and parameterized (STANDARDS_QUALITY.md QUAL-3).

### AG-7 Answer first, then stop talking

Answer questions directly; they are not permission to act. Reports state changes, verification limits, and required user action. Put long audits or research in a project file. No lectures or revisiting dismissed topics.

### AG-8 Own mistakes; trust the user's evidence

On a user-reported failure, re-check your work before blaming setup; own and fix mistakes. Try a tool, file, or capability before claiming it is unavailable.

### AG-9 The outcome outranks the process

Skip ceremony, optional diagnostics, or intermediate process when they cost more than they protect, and state what you skipped. Do not use this rule to skip the smallest complete verification lane (AG-4), applicable inspection of user-visible output (AG-5), or real-behavior evidence (AG-6). Hard rules still apply.

### AG-10 Leave nothing running

Run automation headlessly or in the background without popping up extra terminal or GUI windows that disrupt the desktop. Stop task-owned processes, servers, watchers, and terminals; release locks and remove scratch. Keep deliverables and the app the user should run. Never kill or replace the user's program silently. Built software cleans up child processes on exit.

### AG-11 Git: `main`, commit, push (hard)

Work on `main`; branches or worktrees only when asked. Commit and push verified work unless told not to. In a dirty shared tree, stage task-owned paths explicitly; broad add/commit-all modes MUST NOT sweep in unrelated changes. Never revert, stash, overwrite, or discard others' changes. Shared progress MAY be included only when deliberately selected, reviewed, and verified as complete; otherwise leave it untouched and report it.

### AG-12 Current docs describe the present

Update the single authority for changed behavior. Keep history, session notes, and changelog prose out of current docs/comments. Plans and design intent do not prove capability.

### AG-13 CI is local; GitHub Actions are banned (hard)

Use repository-owned local verification. Never create, enable, invoke, or push `.github/workflows/`; remove existing workflows while keeping their local equivalent.

### Shared workspace tools

Shared portfolio tools are optional accelerators available from this project root through `../tools/`; using them does not authorize edits outside this project. Skip them when native model/harness navigation already provides equivalent evidence or ordinary source inspection is cheaper. No shared analysis command is a prerequisite to editing. Tool output is evidence, never authority; if it conflicts with current source or project authority, trust the authority and treat the tool result as stale or defective. When useful:

```text
python ../tools/agent_context.py brief fortune_and_family <task ...>
python ../tools/agent_context.py --help
python ../tools/agent_context.py diff-impact fortune_and_family
```

`agent_context.py` provides `brief`, `context`, `impact`, `diff-impact`, `outline`, `structure`, `architecture`, `backends`, `audit`, `rule`, and `exemplar`. `brief` returns only task-grounded optional next commands for the current working directory. `exemplar` surfaces non-normative sibling-project prior art and automatically excludes the assigned project when run from its root; adapt ideas locally unless a real shared contract justifies shared code.
For Cargo projects, `python ../tools/rust_agent_tools.py check` verifies the shared Rust diagnostics; use them only as routed by this card's Rust diagnostics marker and `TESTING.md`.
<!-- workspace-contract:end -->

## Profiles

**Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation.** Universal rules apply throughout; persistence and projection follow DATA and the stateful OWN rules; harness evaluation in `src/gameplay/` follows the Automated Behavior Evaluation companion; procedural art in `src/art/` and HTML/report output follow the Artifact Generation companion with PROD-1.

## Procedure

1. Inspect working-tree state (`git status --short`); preserve unrelated concurrent work.
2. Use the routing table below; consult the relevant scope in `STATUS.md` and contract in `ARCHITECTURE.md`. Use `README.md` for run/setup questions.
3. Trace the public entry point (`src/lib.rs` → `src/systems/*`) to the owning module. Read its file header (`Purpose / Owns / Reads / Mutates / Does not own / Canonical operations / Relevant invariants / Focused tests`) before editing.
4. Identify the narrowest test that proves the change. Run before editing only to reproduce a failure; otherwise run once behavior is ready.
5. Select the smallest completion lane from `TESTING.md` that owns the changed surface.
6. Update the one document that owns any changed architecture, behavior, schema, API, command, harness, or scope contract.

## Routing

| Concern | Owner |
|---|---|
| Campaign construction | `src/systems/bootstrap.rs` |
| Player commands | `src/systems/commands/` (dispatch `mod.rs`, family submodules) |
| Daily / scheduled simulation | `src/systems/simulation/` (daily), `src/systems/strategic/` (weekly/monthly/annual) |
| Legal / progression | `src/systems/legal.rs`, `src/systems/progression.rs` |
| Persistence & validation | `src/persistence.rs`, `src/systems/invariants.rs` |
| Read models / HTML | `src/projection.rs` |
| Gameplay analysis | `src/gameplay/` |
| Procedural art | `src/art/` |
| Core types & state | `src/core/`, `src/ids.rs`, `src/money.rs`, `src/rng.rs`, `src/registry/` |

Impact map: `ARCHITECTURE.md` § Extension map lists required companion work per change class (state shape → validation/invariants/projection/tests; commands → feedback/projection/harness; schedule → ordering/tests; persistence → schema/round-trip; adapters → smoke).

## Guardrails

- `Registry` owns immutable definitions; `AppState` and its stores own serializable runtime state; systems own validation and mutation; adapters (CLI, persistence, projection, HTML, gameplay, art) translate IO only and own no domain rules.
- Consequential operations validate references, ownership, permission, lifecycle, capacity, ranges, and arithmetic before one atomic commit; rejection preserves state and reports a typed `CommandError` / `SimulationError` variant.
- Multi-record work resolves the complete result before commit or uses a consumed `Validated*` token with current-state revalidation; stale tokens fail without mutation.
- Preserve state-owned deterministic randomness (`AppState.rng`), ordered `BTreeMap` iteration with typed-ID tie-breakers, fixed-point `Money`/`Quantity` with `i128` intermediates, checked scheduling (`checked_future_day`), and explicit overflow handling.
- Persistent identity uses typed IDs (`src/ids.rs`); optional relations are explicit `Option<T>`; authoritative records and owned indexes/lifecycle memberships update coherently via store methods.
- Core systems perform no implicit IO. Durable external work is represented in state before an adapter performs it.
- Project-owned enums are exhaustive; consequential fields are private; domain failures use typed errors with variant fields, not string parsing; replaced internal paths are deleted.
- History is append-only via `HistoryLog<T>` (cheap clone, structural checksum); `CampaignEvidenceMemo` and checksum memos are pure derivations excluded from serialization/equality and rebuilt lazily.
- Ad hoc saves, reports, captures, and scratch copies belong under ignored `target/agent-output/<task>/` or an OS temp dir, never in `../` or the workspace root. Remove task-owned transient output before handoff.
- Verification is local: `bash scripts/test.sh <lane>` (or `.\scripts\test.ps1 <lane>` on Windows); `python ../tools/check_no_github_actions.py` must pass.

## Completion

For iteration use `bash scripts/test.sh fast <filter>` only to isolate a failure or shorten feedback. For completion go directly to `standard`; an extra `fast` beforehand adds no evidence.

For specialized surfaces run the smallest lane in `TESTING.md` that owns the changed contract plus only genuinely distinct evidence that lane does not already contain. Deeper lanes are required only when their distinct contract changed.

Before handoff confirm canonical ownership, deterministic/persistence/invariant behavior, current documentation, clean diff hygiene, and that no `target/agent-output` or workspace-root transient remains.
