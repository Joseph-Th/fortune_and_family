# Agent Guide

**BCA policy:** advisory

Execution card for repository work. [ARCHITECTURE.md](ARCHITECTURE.md) owns implementation structure and mutation contracts, [STATUS.md](STATUS.md) owns current capability and schemas, [TESTING.md](TESTING.md) owns test policy and completion gates, [DESIGN.md](DESIGN.md) owns product intent, and [GAMEPLAY_HARNESS.md](GAMEPLAY_HARNESS.md) owns harness report semantics. Root [`../AGENTS.md`](../AGENTS.md) owns workspace coordination, task leases, and filesystem hygiene.

## Profiles

This repository implements **Universal**, **Stateful Application**, **Deterministic System**, **Automated Behavior Evaluation**, and **Artifact Generation**. Sections 5, 6, 7, 9, 10, 29-32, 38 apply universally; persistence/projection invariants, gameplay harness evaluation, and procedural-art generation follow their respective companion standards.

## Procedure

1. Inspect working-tree state (`git status --short`) and preserve unrelated concurrent work.
2. Read [README.md](README.md), [STATUS.md](STATUS.md), and the relevant section of [ARCHITECTURE.md](ARCHITECTURE.md).
3. Trace the public entry point (`src/lib.rs` → `src/systems/*`) to the owning module, sibling tests, and invariant checks.
4. Identify the narrowest test that proves the change. Run before editing only to reproduce a failure; otherwise run once behavior is ready.
5. Select the smallest completion lane from [TESTING.md](TESTING.md) that owns the changed surface.
6. Update the one document that owns any changed architecture, behavior, schema, API, command, harness, or scope contract.

## Routing & impact

- **Task routing:** table below maps a change class to its owning module. Read the owning module's file header (`Purpose / Owns / Reads / Mutates / Does not own / Canonical operations / Relevant invariants / Focused tests`) before editing.
- **Change impact:** [ARCHITECTURE.md](ARCHITECTURE.md) Extension map lists required companion work per change class (state shape → validation/invariants/projection/tests; commands → feedback/projection/harness; schedule → ordering/tests; persistence → schema/round-trip; adapters → smoke). The same map is the check-list for companion obligations that are easy to miss.

## Primary owners

| Concern | Owner |
|---|---|
| Campaign construction | `src/systems/bootstrap.rs` |
| Player commands | `src/systems/commands/` (dispatch in `mod.rs`, family-specific submodules) |
| Daily/scheduled simulation | `src/systems/simulation/` (daily pipeline), `src/systems/strategic/` (weekly/monthly/annual) |
| Legal / progression | `src/systems/legal.rs`, `src/systems/progression.rs` |
| Persistence & validation | `src/persistence.rs`, `src/systems/invariants.rs` |
| Read models / HTML | `src/projection.rs` |
| Gameplay analysis | `src/gameplay/` |
| Procedural art | `src/art/` |
| Core types & state | `src/core/`, `src/ids.rs`, `src/money.rs`, `src/rng.rs`, `src/registry/` |

## Project guardrails

- `Registry` owns immutable definitions; `AppState` and its stores own serializable runtime state; systems own validation and mutation; adapters (CLI, persistence, projection, HTML, gameplay, art) translate external IO only and own no domain rules.
- Consequential operations validate references, ownership, permission, lifecycle, capacity, ranges, and arithmetic before one atomic commit; rejected operations preserve state and report a typed `CommandError`/`SimulationError` variant.
- Multi-record work resolves the complete result before commit or uses a consumed `Validated*` token with current-state revalidation; stale tokens fail without mutation.
- Preserve state-owned deterministic randomness (`AppState.rng`), stable ordered iteration with typed-ID tie-breakers, fixed-point `Money`/`Quantity` with wide ratio intermediates, checked scheduling (`checked_future_day`), and explicit overflow handling.
- Persistent identity uses typed IDs (`src/ids.rs`); optional relationships are explicit `Option<T>`; authoritative records and owned indexes/lifecycle memberships update coherently via store methods.
- Core systems perform no implicit IO. Durable external work is represented in state before an adapter performs it.
- Project-owned enums are exhaustive; consequential fields are private; domain failures use contextual typed errors with variant fields, not string parsing; replaced internal paths are deleted.
- History growth stays append-only via `HistoryLog<T>` (cheap clone, structural checksum); `CampaignEvidenceMemo` and checksum memos are pure derivations excluded from serialization/equality and rebuilt lazily.
- Ad hoc saves, gameplay reports, CLI captures, benchmark evidence, and scratch copies belong under ignored `target/agent-output/<task>/` (or an OS temporary directory), never in `../` or the workspace root. Remove task-owned transient output before handoff.
- Repository verification is local: `bash scripts/test.sh <lane>` (or `.\scripts\test.ps1 <lane>` on Windows). Do not create or depend on GitHub Actions workflows; `python ../tools/check_no_github_actions.py` must pass.

## Completion

For iteration use `bash scripts/test.sh fast <filter>` only when it isolates a failure or shortens feedback. For completion go directly to `standard`; an extra `fast` beforehand adds nothing.

For specialized surfaces run the smallest lane in [TESTING.md](TESTING.md) that owns the changed contract plus only genuinely distinct evidence that lane does not already contain. Deeper lanes are required only when their distinct contract changed.

Before handoff confirm canonical ownership, deterministic/persistence/invariant behavior, current documentation, clean diff hygiene, and that no `target/agent-output` or workspace-root transient remains.
