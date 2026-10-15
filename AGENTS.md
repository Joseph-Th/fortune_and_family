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

Finish the authorized task; plans, progress, or deferred notes are not delivery. Delegate only bounded read-only work when authorized.

### AG-2 Make the calls yourself

Make in-scope decisions yourself; state consequential choices briefly and continue. Ask only when the answer changes the result and cannot be inferred. Read the project card once, then resolve authority, owner, and proof. Expand only for unresolved scope or boundaries. Links and profiles are lookups; research and artifact work starts at the product workflow.

### AG-3 Stay in your project (hard)

Work only in the requested project. Do not create top-level directories or write to the workspace root or project parent. Put scratch work in the project's ignored output (normally `target/agent-output/`) or OS temp.

### AG-4 Done means the user's copy works (hard)

Verify every requirement against the user's actual copy. Refresh and check the affected release, deployment, or running app; source edits alone do not update it. Docs-only work verifies docs and routes. Use the smallest complete project lane and [`REVIEW.md`](../REVIEW.md) for consequential code. Claim only checked results; report what remains unverified.

### AG-5 Look at what you made

Run or render changed visual, audio, interactive, or published output; inspect it against [`STANDARDS_QUALITY.md`](../STANDARDS_QUALITY.md) and project references. Verify requested files, fix defects, and report rendered evidence. Internal prose needs only readability and route review. Tests do not prove appearance.

### AG-6 Real behavior over proxies

Observe user-experienced behavior. Tests, harnesses, validators, and metrics are evidence, not the goal. Repair proxy/product divergence; never weaken gates, hardcode outcomes, suppress warnings, or retry until green. Simulations stay causal and parameterized (STANDARDS_QUALITY.md QUAL-3).

### AG-7 Answer first, then stop talking

Answer questions directly; they are not permission to act. Reports state changes, verification limits, and required user action. Put long audits or research in a project file. No lectures or revisiting dismissed topics.

### AG-8 Own mistakes; trust the user's evidence

On a user-reported failure, re-check your work before blaming setup; own and fix mistakes. Try a tool, file, or capability before claiming it is unavailable.

### AG-9 The outcome outranks the process

If procedure harms the result or costs more than it protects, favor the result and state what you skipped. Hard rules still apply.

### AG-10 Leave nothing running

Stop task-owned processes, servers, watchers, and terminals; release locks and remove scratch. Keep deliverables and the app the user should run. Never kill or replace the user's program silently. Built software cleans up child processes on exit.

### AG-11 Git: `main`, commit, push (hard)

Work on `main`; branches or worktrees only when asked. Commit and push verified work unless told not to. Never revert, stash, or discard others' changes. Include sound shared progress when appropriate; leave broken unrelated work out and report it.

### AG-12 Current docs describe the present

Update the single authority for changed behavior. Keep history, session notes, and changelog prose out of current docs/comments. Plans and design intent do not prove capability.

### AG-13 CI is local; GitHub Actions are banned (hard)

Use repository-owned local verification. Never create, enable, invoke, or push `.github/workflows/`; remove existing workflows while keeping their local equivalent.
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
