# Agent Guide

**BCA policy:** advisory


**Rust agent diagnostics:** advisory. Use bounded diagnostics only for unresolved ownership, test-strength, or macro questions; commands and verification routing: [TESTING.md](TESTING.md).

**Profiles:** Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation, Artifact Generation

Execution card for repository work. `ARCHITECTURE.md` owns structure and mutation contracts, `STATUS.md` owns capability and schemas, `TESTING.md` owns test policy, `DESIGN.md` owns intent, `GAMEPLAY_HARNESS.md` owns harness semantics. The embedded workspace contract owns shared execution and filesystem rules.

<!-- workspace-contract:begin (generated from ../AGENTS.md by tools/sync_agent_context.py; edit the source, not this copy) -->
## Workspace contract

Applies to every agent in every harness. Source, rationale, and evidence: [../AGENTS.md](../AGENTS.md).

The user is a solo developer. Precedence: current request → this contract → project card → standards. This is a safety kernel, not a prescribed workflow. **Hard** rules survive lower-level advice.

### AG-1 Finish the work

Finish the authorized task. On open-ended goals, iterate from evidence until the goal is met or useful in-scope improvements are exhausted; pivot around blockers instead of stopping at them.

### AG-2 Make the calls yourself

Make routine in-scope decisions yourself. Read the project card once, identify the owner and proof route, then work. Ask only when a choice materially changes the deliverable and cannot be inferred.

### AG-3 Write only in your project (hard)

Write only in the requested project. Read-only sibling inspection is allowed when it materially helps. Do not write to the workspace root, parent, or siblings; use project-ignored output or OS temp for scratch.

### AG-4 Done means the user's copy works (hard)

Verify the result in the user's actual copy, including the release, deployment, or running app when applicable. Use the smallest complete project lane and [`REVIEW.md`](../REVIEW.md) for consequential code. Claim only checked results.

### AG-5 Look at what you made

Run or render changed visual, audio, interactive, or published output and inspect the real result against project references and [`STANDARDS_QUALITY.md`](../STANDARDS_QUALITY.md). Tests do not prove appearance.

### AG-6 Real behavior over proxies

Prefer observed product behavior over proxy success. Use tests, harnesses, validators, and metrics as evidence; fix production root causes and proxy divergence rather than weakening gates or hardcoding outcomes.

### AG-7 Answer first, then stop talking

Answer questions directly. Reports state changes, verification limits, and required user action; keep long audit material in project files instead of chat.

### AG-8 Own mistakes; trust the user's evidence

On a reported failure, re-check your work before blaming setup. Try a capability before claiming it is unavailable.

### AG-9 The outcome outranks the process

Skip ceremony and optional diagnostics when they cost more than they protect. Do not skip the proof required by AG-4 through AG-6 or any hard rule.

### AG-10 Leave nothing running

Run automation without disrupting the desktop. Stop task-owned processes, release locks, and remove scratch; never kill or replace the user's program silently.

### AG-11 Git: `main`, commit, push (hard)

Work on `main`; branches or worktrees only when asked. Commit and push verified work unless told not to. Stage task-owned paths explicitly in dirty trees and never discard or sweep in unrelated work.

### AG-12 Current docs describe the present

Update the authority for changed behavior. Keep history and session notes out of current docs and comments.

### AG-13 CI is local; GitHub Actions are banned (hard)

Use repository-owned local verification. Never create, enable, invoke, or push `.github/workflows/`.

### Optional workspace accelerator

Use native navigation first. If cold-start routing is still expensive or ambiguous, run:

```text
python ../tools/agent_context.py brief fortune_and_family <task ...>
```

It returns bounded authority, verification, and likely-source evidence. Tool output is evidence, not authority. Use `python ../tools/agent_context.py --help` only when a specific unresolved question justifies a deeper probe. No shared analysis command is a prerequisite to editing.
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
