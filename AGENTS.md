# Agent Guide

**BCA policy:** advisory

This file is the execution card for repository work. [ARCHITECTURE.md](ARCHITECTURE.md) owns implementation structure and mutation contracts, [STATUS.md](STATUS.md) owns current capability, [TESTING.md](TESTING.md) owns test policy and completion gates, [DESIGN.md](DESIGN.md) owns product intent, and [GAMEPLAY_HARNESS.md](GAMEPLAY_HARNESS.md) owns harness report semantics. Root [`../AGENTS.md`](../AGENTS.md) owns workspace coordination, task leases, and filesystem hygiene.

## Profiles

This repository implements **Universal**, **Stateful Application**, **Deterministic System**, **Automated Behavior Evaluation**, and **Artifact Generation**. Sections 5, 6, 7, 9, 10, 29-32, 38 apply universally; persistence/projection invariants, gameplay harness evaluation, and procedural-art generation follow their respective companion standards.

## Procedure

1. Inspect working-tree state (`git status --short`) and preserve unrelated concurrent work; do not treat another agent's dirty files as cleanup targets.
2. Read [README.md](README.md), [STATUS.md](STATUS.md), and only the relevant section of [ARCHITECTURE.md](ARCHITECTURE.md) for the change.
3. Identify the owning source module and sibling tests; trace the public entry point (`src/lib.rs` → `src/systems/*`) to canonical mutation and invariant validation.
4. Identify the narrowest test that proves the change. Run it before editing only to reproduce a failure; otherwise run it once behavior is ready for proof.
5. Use [TESTING.md](TESTING.md) to choose the smallest completion lane that owns the changed surface.
6. Update the one document that owns any changed architecture, behavior, schema, API, command, harness, or scope contract.

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

Use `bash scripts/test.sh fast <filter>` for focused iteration when it shortens feedback or isolates a failure. When ordinary work is ready for completion, go directly to `standard`; running `fast` immediately beforehand adds nothing.

For specialized surfaces, run the smallest lane from [TESTING.md](TESTING.md) that owns the changed contract, plus only genuinely distinct evidence that lane does not already contain. Deeper lanes are required only when their distinct contract changed.

Before handing off, confirm canonical ownership, deterministic/persistence/invariant behavior, current documentation, clean diff hygiene, and that no `target/agent-output` or workspace-root transient remains.
